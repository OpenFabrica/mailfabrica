// Package store 读写与既有版本兼容的 JSON 索引和 EML 原文。
package store

import (
	"encoding/json"
	"errors"
	"fmt"
	"hash/fnv"
	"io/fs"
	"os"
	"path/filepath"
	"sort"

	"github.com/openfabrica/mailfabrica/internal/mail"
)

// Index 保留旧版蛇形 uidvalidity 字段。
type Index struct {
	UIDValidity uint32         `json:"uidvalidity"`
	Messages    []mail.Message `json:"messages"`
}

// Store 只由引擎锁保护的会话访问，不自行启动后台任务。
type Store struct {
	revision uint64
	Root     string
	Accounts []mail.Account
	Contacts []mail.Contact
	Folders  map[string][]mail.Folder
	Indexes  map[string]*Index
}

// Open 读取旧数据；损坏 JSON 返回错误，避免空数据覆盖原文件。
func Open(root string) (*Store, error) {
	if err := os.MkdirAll(root, 0700); err != nil {
		return nil, err
	}
	s := &Store{Root: root, Accounts: []mail.Account{}, Contacts: []mail.Contact{}, Folders: map[string][]mail.Folder{}, Indexes: map[string]*Index{}}
	if e := ReadJSON(filepath.Join(root, "accounts.json"), &s.Accounts); e != nil {
		return nil, e
	}
	if e := ReadJSON(filepath.Join(root, "contacts.json"), &s.Contacts); e != nil {
		return nil, e
	}
	for ai, a := range s.Accounts {
		if a.IMAP.Security == "" {
			a.IMAP.Security = "tls"
		}
		if a.SMTP.Security == "" {
			a.SMTP.Security = "tls"
		}
		s.Accounts[ai] = a
		if !mail.ValidAccountID(a.ID) {
			return nil, fmt.Errorf("非法账户 id: %q", a.ID)
		}
		f := []mail.Folder{}
		if e := ReadJSON(filepath.Join(root, a.ID, "folders.json"), &f); e != nil {
			return nil, e
		}
		s.Folders[a.ID] = f
	}
	return s, nil
}

// ReadJSON 对不存在文件保留调用方默认值，其余错误上报。
func ReadJSON(path string, v any) error {
	b, e := os.ReadFile(path)
	if errors.Is(e, fs.ErrNotExist) {
		return nil
	}
	if e != nil {
		return e
	}
	if e = json.Unmarshal(b, v); e != nil {
		return fmt.Errorf("读取 %s 失败: %w", filepath.Base(path), e)
	}
	return nil
}

// WriteAtomic 在同一目录创建私有临时文件，刷盘后原子替换。
func WriteAtomic(path string, b []byte) error {
	if e := os.MkdirAll(filepath.Dir(path), 0700); e != nil {
		return e
	}
	f, e := os.CreateTemp(filepath.Dir(path), ".mailfabrica-*")
	if e != nil {
		return e
	}
	defer os.Remove(f.Name())
	if _, e = f.Write(b); e != nil {
		f.Close()
		return e
	}
	if e = f.Sync(); e != nil {
		f.Close()
		return e
	}
	if e = f.Close(); e != nil {
		return e
	}
	return os.Rename(f.Name(), path)
}

// WriteJSON 将格式化 JSON 以原子方式持久化。
func WriteJSON(path string, v any) error {
	b, e := json.MarshalIndent(v, "", "  ")
	if e != nil {
		return e
	}
	return WriteAtomic(path, b)
}

// FolderDirName 沿用 UTF-8 文件夹路径的 FNV-1a 目录名，保持旧缓存可读。
func FolderDirName(folder string) string {
	h := fnv.New64a()
	_, _ = h.Write([]byte(folder))
	return fmt.Sprintf("f-%016x", h.Sum64())
}

// FolderDir 返回经过账户校验的文件夹目录。
func (s *Store) FolderDir(account, folder string) (string, error) {
	if !mail.ValidAccountID(account) || folder == "" {
		return "", fmt.Errorf("账户或文件夹无效")
	}
	return filepath.Join(s.Root, account, FolderDirName(folder)), nil
}

// Index 按需加载索引，调用方应持有引擎锁。
func (s *Store) Index(account, folder string) (*Index, error) {
	dir, e := s.FolderDir(account, folder)
	if e != nil {
		return nil, e
	}
	if i, ok := s.Indexes[dir]; ok {
		return i, nil
	}
	i := &Index{Messages: []mail.Message{}}
	if e = ReadJSON(filepath.Join(dir, "index.json"), i); e != nil {
		return nil, e
	}
	s.Indexes[dir] = i
	return i, nil
}

// SaveIndex 保存文件夹索引。
func (s *Store) SaveIndex(account, folder string) error {
	dir, e := s.FolderDir(account, folder)
	if e != nil {
		return e
	}
	i, e := s.Index(account, folder)
	if e != nil {
		return e
	}
	sort.SliceStable(i.Messages, func(a, b int) bool {
		if i.Messages[a].Date == i.Messages[b].Date {
			return i.Messages[a].Key.UID > i.Messages[b].Key.UID
		}
		return i.Messages[a].Date > i.Messages[b].Date
	})
	if err := WriteJSON(filepath.Join(dir, "index.json"), i); err != nil {
		return err
	}
	s.revision++
	return nil
}

// SaveAccounts 保存账户配置，不写凭据。
func (s *Store) SaveAccounts() error {
	if err := WriteJSON(filepath.Join(s.Root, "accounts.json"), s.Accounts); err != nil {
		return err
	}
	s.revision++
	return nil
}

// SaveContacts 保存联系人。
func (s *Store) SaveContacts() error {
	if err := WriteJSON(filepath.Join(s.Root, "contacts.json"), s.Contacts); err != nil {
		return err
	}
	s.revision++
	return nil
}

// SetFolders 保存账户目录列表。
func (s *Store) SetFolders(account string, folders []mail.Folder) error {
	if !mail.ValidAccountID(account) {
		return fmt.Errorf("非法账户")
	}
	if e := WriteJSON(filepath.Join(s.Root, account, "folders.json"), folders); e != nil {
		return e
	}
	s.Folders[account] = folders
	s.revision++
	return nil
}

// Raw 读取指定 UID 的原始邮件。
func (s *Store) Raw(k mail.Key) ([]byte, error) {
	d, e := s.FolderDir(k.Account, k.Folder)
	if e != nil {
		return nil, e
	}
	return os.ReadFile(filepath.Join(d, fmt.Sprintf("%d.eml", k.UID)))
}

// SaveRaw 原子保存原始邮件。
func (s *Store) SaveRaw(k mail.Key, b []byte) error {
	d, e := s.FolderDir(k.Account, k.Folder)
	if e != nil {
		return e
	}
	return WriteAtomic(filepath.Join(d, fmt.Sprintf("%d.eml", k.UID)), b)
}

// RemoveRaw 删除邮件缓存，不影响其他 UID。
func (s *Store) RemoveRaw(k mail.Key) error {
	d, e := s.FolderDir(k.Account, k.Folder)
	if e != nil {
		return e
	}
	e = os.Remove(filepath.Join(d, fmt.Sprintf("%d.eml", k.UID)))
	if errors.Is(e, fs.ErrNotExist) {
		return nil
	}
	return e
}

// RemoveAccount 删除账户对应的缓存；仅允许安全的账户目录。
func (s *Store) RemoveAccount(id string) error {
	if !mail.ValidAccountID(id) {
		return fmt.Errorf("非法账户")
	}
	for i, a := range s.Accounts {
		if a.ID == id {
			s.Accounts = append(s.Accounts[:i], s.Accounts[i+1:]...)
			break
		}
	}
	if e := s.SaveAccounts(); e != nil {
		return e
	}
	delete(s.Folders, id)
	return os.RemoveAll(filepath.Join(s.Root, id))
}

// ResetFolder 在 UIDVALIDITY 改变后清除已失效的 UID 缓存，禁止误读同号旧邮件。
func (s *Store) ResetFolder(account, folder string) error {
	dir, e := s.FolderDir(account, folder)
	if e != nil {
		return e
	}
	if e = os.RemoveAll(dir); e != nil {
		return e
	}
	delete(s.Indexes, dir)
	return nil
}

// Revision 是账户、联系人和索引的变更序号，由会话锁保护；只读查询不使联系人索引失效。
func (s *Store) Revision() uint64 { return s.revision }
