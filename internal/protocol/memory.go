package protocol

import (
	"context"
	"fmt"
	"github.com/openfabrica/mailfabrica/internal/mail"
	"sort"
	"sync"
	"time"
)

// Memory 是仅用于显式 demo 和测试的内存服务器。
type Memory struct {
	mu       sync.Mutex
	Folders  map[string][]mail.Folder
	Messages map[string]map[uint32][]byte
	Flags    map[string]map[uint32]mail.Flags
}

// NewMemory 创建不会连接真实邮箱的假服务器。
func NewMemory() *Memory {
	return &Memory{Folders: map[string][]mail.Folder{}, Messages: map[string]map[uint32][]byte{}, Flags: map[string]map[uint32]mail.Flags{}}
}

// memoryKey 避免账户与文件夹拼接歧义。
func memoryKey(a, f string) string { return a + "\x00" + f }

// Verify 在取消时返回错误，其余演示凭据有效。
func (m *Memory) Verify(ctx context.Context, _ mail.Account, _ string) error { return ctx.Err() }

// Put 在测试或 demo 初始化阶段添加原文和标记。
func (m *Memory) Put(a, f string, uid uint32, raw []byte, flags mail.Flags) {
	m.mu.Lock()
	defer m.mu.Unlock()
	key := memoryKey(a, f)
	if m.Messages[key] == nil {
		m.Messages[key] = map[uint32][]byte{}
		m.Flags[key] = map[uint32]mail.Flags{}
	}
	m.Messages[key][uid] = append([]byte{}, raw...)
	m.Flags[key][uid] = flags
}

// Sync 返回一致的服务器快照。
func (m *Memory) Sync(ctx context.Context, a mail.Account, _ string, _, _ uint32, _ ...map[string]KnownFolder) ([]FolderResult, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if e := ctx.Err(); e != nil {
		return nil, e
	}
	out := []FolderResult{}
	for _, folder := range m.Folders[a.ID] {
		key := memoryKey(a.ID, folder.Path)
		fr := FolderResult{Folder: folder, Messages: []mail.Message{}, UIDs: []uint32{}, Raw: map[uint32][]byte{}}
		fr.Folder.Total = uint32(len(m.Messages[key]))
		fr.Folder.Unread = 0
		for uid, raw := range m.Messages[key] {
			b, e := Parse(raw)
			if e != nil {
				return nil, e
			}
			flags := m.Flags[key][uid]
			if !flags.Seen {
				fr.Folder.Unread++
			}
			fr.Messages = append(fr.Messages, Metadata(mail.Key{Account: a.ID, Folder: folder.Path, UID: uid}, b, len(raw), flags))
			fr.UIDs = append(fr.UIDs, uid)
			fr.Raw[uid] = append([]byte{}, raw...)
		}
		sort.Slice(fr.UIDs, func(i, j int) bool { return fr.UIDs[i] < fr.UIDs[j] })
		out = append(out, fr)
	}
	return out, nil
}

// Fetch 返回指定邮件的独立字节副本。
func (m *Memory) Fetch(ctx context.Context, _ mail.Account, _ string, k mail.Key) ([]byte, error) {
	m.mu.Lock()
	defer m.mu.Unlock()
	if e := ctx.Err(); e != nil {
		return nil, e
	}
	raw, ok := m.Messages[memoryKey(k.Account, k.Folder)][k.UID]
	if !ok {
		return nil, fmt.Errorf("邮件不存在")
	}
	return append([]byte{}, raw...), nil
}

// MoveMany 模拟批量移动；任一邮件不存在时返回错误。
func (m *Memory) MoveMany(ctx context.Context, a mail.Account, p, folder string, uids []uint32, dest string) error {
	for _, uid := range uids {
		if e := m.Change(ctx, a, p, mail.Key{Account: a.ID, Folder: folder, UID: uid}, "move", dest); e != nil {
			return e
		}
	}
	return nil
}

// SetFlags 模拟批量标记；任一邮件不存在时返回错误。
func (m *Memory) SetFlags(ctx context.Context, a mail.Account, p, folder string, uids []uint32, op string, value bool) error {
	for _, uid := range uids {
		if e := m.Change(ctx, a, p, mail.Key{Account: a.ID, Folder: folder, UID: uid}, op, value); e != nil {
			return e
		}
	}
	return nil
}

// Change 模拟远端标记、移动和删除。
func (m *Memory) Change(ctx context.Context, _ mail.Account, _ string, k mail.Key, op string, value any) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	if e := ctx.Err(); e != nil {
		return e
	}
	key := memoryKey(k.Account, k.Folder)
	raw, ok := m.Messages[key][k.UID]
	if !ok {
		return fmt.Errorf("邮件不存在")
	}
	flags := m.Flags[key][k.UID]
	switch op {
	case "seen":
		flags.Seen, _ = value.(bool)
	case "flagged":
		flags.Flagged, _ = value.(bool)
	case "answered":
		flags.Answered, _ = value.(bool)
	case "move":
		folder, ok := value.(string)
		if !ok {
			return fmt.Errorf("无效文件夹")
		}
		dest := memoryKey(k.Account, folder)
		if m.Messages[dest] == nil {
			m.Messages[dest] = map[uint32][]byte{}
			m.Flags[dest] = map[uint32]mail.Flags{}
		}
		uid := uint32(1)
		for old := range m.Messages[dest] {
			if old >= uid {
				uid = old + 1
			}
		}
		m.Messages[dest][uid] = raw
		m.Flags[dest][uid] = flags
		delete(m.Messages[key], k.UID)
		delete(m.Flags[key], k.UID)
		return nil
	case "delete":
		delete(m.Messages[key], k.UID)
		delete(m.Flags[key], k.UID)
		return nil
	default:
		return fmt.Errorf("未知变更")
	}
	m.Flags[key][k.UID] = flags
	return nil
}

// Create 创建演示服务器目录。
func (m *Memory) Create(ctx context.Context, a mail.Account, _ string, name string) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	if e := ctx.Err(); e != nil {
		return e
	}
	for _, f := range m.Folders[a.ID] {
		if f.Path == name {
			return fmt.Errorf("文件夹已存在")
		}
	}
	m.Folders[a.ID] = append(m.Folders[a.ID], mail.Folder{Account: a.ID, Path: name, Role: mail.FolderRole(name, nil), Selectable: true, UIDValidity: 1})
	return nil
}

// Append 向目标目录保存草稿或已发送副本。
func (m *Memory) Append(ctx context.Context, a mail.Account, _ string, folder string, raw []byte, flags []string) error {
	m.mu.Lock()
	defer m.mu.Unlock()
	if e := ctx.Err(); e != nil {
		return e
	}
	key := memoryKey(a.ID, folder)
	if m.Messages[key] == nil {
		m.Messages[key] = map[uint32][]byte{}
		m.Flags[key] = map[uint32]mail.Flags{}
	}
	uid := uint32(time.Now().Unix())
	for {
		if _, exists := m.Messages[key][uid]; !exists {
			break
		}
		uid++
	}
	m.Messages[key][uid] = append([]byte{}, raw...)
	f := mail.Flags{}
	for _, v := range flags {
		if v == "\\Seen" {
			f.Seen = true
		}
		if v == "\\Draft" {
			f.Draft = true
		}
	}
	m.Flags[key][uid] = f
	return nil
}

// Send 完整执行 MIME 编码，发送结果由引擎负责归档。
func (m *Memory) Send(ctx context.Context, _ mail.Account, _ string, out mail.Outgoing) ([]byte, error) {
	if e := ctx.Err(); e != nil {
		return nil, e
	}
	return Build(out)
}
