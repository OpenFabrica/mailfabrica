package engine

import (
	"context"
	"fmt"
	"github.com/openfabrica/mailfabrica/internal/mail"
	"github.com/openfabrica/mailfabrica/internal/protocol"
	"github.com/openfabrica/mailfabrica/internal/store"
	"os"
	"strings"
)

// mutation 记录每封邮件已完成的远端变更，允许批量操作部分失败可见。
type mutation struct {
	Key   mail.Key
	Op    string
	Value any
	Err   error
}

// selectedKeys 校验显式多选参数或当前选择。
func (s *Session) selectedKeys(p Params) ([]mail.Key, error) {
	keys := s.view.Selected
	if v, exists := p["keys"]; exists {
		a, ok := v.([]any)
		if !ok {
			return nil, fmt.Errorf("keys 必须是字符串数组")
		}
		keys = []string{}
		for _, v := range a {
			k, ok := v.(string)
			if !ok {
				return nil, fmt.Errorf("邮件键必须是字符串")
			}
			keys = append(keys, k)
		}
	}
	if len(keys) == 0 {
		return nil, fmt.Errorf("请选择邮件")
	}
	if len(keys) > 10000 {
		return nil, fmt.Errorf("单次最多处理 10000 封邮件")
	}
	out := []mail.Key{}
	for _, v := range keys {
		k, e := mail.ParseKey(v)
		if e != nil {
			return nil, e
		}
		out = append(out, k)
	}
	return out, nil
}

// findMeta 定位本地元数据。
func (s *Session) findMeta(k mail.Key) (mail.Message, error) {
	idx, e := s.Store.Index(k.Account, k.Folder)
	if e != nil {
		return mail.Message{}, e
	}
	for _, m := range idx.Messages {
		if m.Key == k {
			return protocol.DecodeMetadata(m), nil
		}
	}
	return mail.Message{}, fmt.Errorf("邮件不存在")
}

// roleFolder 优先使用服务器 SPECIAL-USE 识别结果。
func (s *Session) roleFolder(account, role string) (string, error) {
	for _, f := range s.Store.Folders[account] {
		if f.Role == role {
			return f.Path, nil
		}
	}
	return "", fmt.Errorf("账户缺少 %s 文件夹", role)
}

// groupedRows 为命令查询生成与 UI 相同的主题分组摘要。
func groupedRows(messages []mail.Message) []Row {
	out := []Row{}
	indices := map[string]int{}
	for _, m := range messages {
		subject := mail.SubjectKey(m.Subject)
		index, ok := indices[subject]
		if !ok || subject == "" {
			index = len(out)
			out = append(out, Row{Message: m, Keys: []string{}, Senders: []string{}})
			indices[subject] = index
		}
		r := &out[index]
		r.Count++
		r.Keys = append(r.Keys, m.Key.String())
		if !m.Flags.Seen {
			r.Unread++
		}
		r.Flagged = r.Flagged || m.Flags.Flagged
		name := m.From.Name
		if name == "" {
			name = m.From.Email
		}
		exists := false
		for _, v := range r.Senders {
			if v == name {
				exists = true
			}
		}
		if !exists {
			r.Senders = append(r.Senders, name)
		}
		if r.Count > 1 {
			r.Group = &subject
		}
	}
	return out
}

// messageCommand 实现阅读、分页、附件和远端变更。
func (s *Session) messageCommand(id string, p Params) (any, error) {
	switch id {
	case "message.list":
		messages := s.view.Messages
		if str(p, "folder") != "" && str(p, "account") == "" {
			return nil, fmt.Errorf("folder 必须提供 account")
		}
		if account := str(p, "account"); account != "" {
			if _, e := s.account(account); e != nil {
				return nil, e
			}
			folder := str(p, "folder")
			if folder == "" {
				var e error
				folder, e = s.roleFolder(account, "inbox")
				if e != nil {
					return nil, e
				}
			}
			idx, e := s.Store.Index(account, folder)
			if e != nil {
				return nil, e
			}
			messages = idx.Messages
		}
		filtered := []mail.Message{}
		for _, m := range messages {
			if !boolean(p, "unreadOnly", false) || !m.Flags.Seen {
				filtered = append(filtered, protocol.DecodeMetadata(m))
			}
		}
		offset := number(p, "offset", 0)
		limit := number(p, "limit", 100)
		if limit > 10000 {
			limit = 10000
		}
		if boolean(p, "group", false) {
			rows := groupedRows(filtered)
			total := len(rows)
			if offset > total {
				offset = total
			}
			end := offset + limit
			if end > total {
				end = total
			}
			return Params{"total": total, "messageTotal": len(filtered), "rows": rows[offset:end]}, nil
		}
		total := len(filtered)
		if offset > total {
			offset = total
		}
		end := offset + limit
		if end > total {
			end = total
		}
		return Params{"total": total, "messages": filtered[offset:end]}, nil
	case "message.get", "message.open", "message.raw", "message.saveAttachment":
		k, e := s.focusedKey(p)
		if e != nil {
			return nil, e
		}
		if _, e = s.findMeta(k); e != nil {
			return nil, e
		}
		switch id {
		case "message.get":
			return s.messageResult(k.String()), nil
		case "message.open":
			return nil, s.openBody(k)
		case "message.raw":
			raw, e := s.Store.Raw(k)
			if e != nil {
				return nil, e
			}
			return string(raw), nil
		case "message.saveAttachment":
			result := s.messageResult(k.String())
			if result["state"] != "ready" {
				return nil, fmt.Errorf("请先下载邮件正文")
			}
			b, ok := result["body"].(mail.Body)
			if !ok {
				return nil, fmt.Errorf("正文无效")
			}
			index := number(p, "index", -1)
			if index < 0 || index >= len(b.Attachments) {
				return nil, fmt.Errorf("附件索引无效")
			}
			path, e := required(p, "path")
			if e != nil {
				return nil, e
			}
			return Params{"path": path}, store.WriteAtomic(path, b.Attachments[index].Data)
		}
	}
	keys, e := s.selectedKeys(p)
	if e != nil {
		return nil, e
	}
	actions := []mutation{}
	accounts := map[string]mail.Account{}
	secrets := map[string]string{}
	// 每个文件夹只建一次 UID 索引，避免批量操作时逐封线性查找
	folders := map[string]map[uint32]mail.Flags{}
	for _, k := range keys {
		folderKey := k.Account + "\x00" + k.Folder
		flagsByUID, ok := folders[folderKey]
		if !ok {
			idx, err := s.Store.Index(k.Account, k.Folder)
			if err != nil {
				return nil, err
			}
			flagsByUID = make(map[uint32]mail.Flags, len(idx.Messages))
			for _, m := range idx.Messages {
				flagsByUID[m.Key.UID] = m.Flags
			}
			folders[folderKey] = flagsByUID
		}
		flags, ok := flagsByUID[k.UID]
		if !ok {
			return nil, fmt.Errorf("邮件不存在")
		}
		// 凭据每个账户只读一次；钥匙串访问较慢且在会话锁内
		if _, ok := accounts[k.Account]; !ok {
			a, err := s.account(k.Account)
			if err != nil {
				return nil, err
			}
			password, err := s.secrets.Get(a.ID)
			if err != nil {
				return nil, err
			}
			accounts[a.ID] = a
			secrets[a.ID] = password
		}
		op := ""
		var value any
		switch id {
		case "message.markRead":
			op = "seen"
			value = boolean(p, "read", true)
		case "message.toggleRead":
			op = "seen"
			value = !flags.Seen
		case "message.flag":
			op = "flagged"
			value = boolean(p, "flagged", !flags.Flagged)
		case "message.move":
			op = "move"
			value, e = required(p, "folder")
			if e != nil {
				return nil, e
			}
		case "message.junk":
			op = "move"
			value, e = s.roleFolder(k.Account, "junk")
			if e != nil {
				return nil, e
			}
		case "message.notJunk":
			// 误判的邮件移回各自账户的收件箱；已在收件箱的跳过
			op = "move"
			value, e = s.roleFolder(k.Account, "inbox")
			if e != nil {
				return nil, e
			}
			if value == k.Folder {
				continue
			}
		case "message.delete":
			op = "move"
			trash, err := s.roleFolder(k.Account, "trash")
			if err != nil {
				return nil, err
			}
			if k.Folder == trash {
				op = "delete"
			} else {
				value = trash
			}
		default:
			return nil, fmt.Errorf("未知邮件操作")
		}
		if op == "seen" && value == flags.Seen || op == "flagged" && value == flags.Flagged {
			continue
		}
		actions = append(actions, mutation{Key: k, Op: op, Value: value})
	}
	if len(actions) == 0 {
		return nil, nil
	}
	// 已读和红旗先写本地并刷新视图，界面立即响应；服务器在后台同步，失败时回滚
	if changed, e := s.applyFlags(actions, false); e != nil {
		return nil, e
	} else if changed {
		if e = s.refresh(); e != nil {
			return nil, e
		}
	}
	accountIDs := []string{}
	for id := range accounts {
		accountIDs = append(accountIDs, id)
	}
	return nil, s.launch(fmt.Sprintf("message.mutate:%d", s.revision), func(ctx context.Context) (any, error) {
		// 同一文件夹的标记或移动合并为一次连接，避免逐封登录触发服务器限流
		type batch struct {
			account, folder, op string
			value               bool
			dest                string
		}
		batches := map[batch][]int{}
		order := []batch{}
		for i, a := range actions {
			b := batch{account: a.Key.Account, folder: a.Key.Folder, op: a.Op}
			switch a.Op {
			case "seen", "flagged":
				b.value, _ = a.Value.(bool)
			case "move":
				b.dest, _ = a.Value.(string)
			default:
				// 永久删除较少见，仍逐封执行
				actions[i].Err = s.remote.Change(ctx, accounts[a.Key.Account], secrets[a.Key.Account], a.Key, a.Op, a.Value)
				continue
			}
			if _, ok := batches[b]; !ok {
				order = append(order, b)
			}
			batches[b] = append(batches[b], i)
		}
		for _, b := range order {
			uids := make([]uint32, 0, len(batches[b]))
			for _, i := range batches[b] {
				uids = append(uids, actions[i].Key.UID)
			}
			var err error
			if b.op == "move" {
				err = s.remote.MoveMany(ctx, accounts[b.account], secrets[b.account], b.folder, uids, b.dest)
			} else {
				err = s.remote.SetFlags(ctx, accounts[b.account], secrets[b.account], b.folder, uids, b.op, b.value)
			}
			for _, i := range batches[b] {
				actions[i].Err = err
			}
		}
		return actions, nil
	}, func(v any) error {
		results, ok := v.([]mutation)
		if !ok {
			return fmt.Errorf("变更结果无效")
		}
		failed := 0
		reasons := []string{}
		seenReason := map[string]bool{}
		confirmed := []mutation{}
		reverted := []mutation{}
		removed := map[string]map[uint32]bool{}
		for _, a := range results {
			if a.Err != nil {
				// 批量失败通常是同一原因，去重后再提示，避免错误文本无限变长
				failed++
				if reason := a.Err.Error(); !seenReason[reason] && len(reasons) < 3 {
					seenReason[reason] = true
					reasons = append(reasons, reason)
				}
				if a.Op == "seen" || a.Op == "flagged" {
					reverted = append(reverted, a)
				}
				continue
			}
			if a.Op == "seen" || a.Op == "flagged" {
				confirmed = append(confirmed, a)
			}
			if a.Op == "move" || a.Op == "delete" {
				folderKey := a.Key.Account + "\x00" + a.Key.Folder
				if removed[folderKey] == nil {
					removed[folderKey] = map[uint32]bool{}
				}
				removed[folderKey][a.Key.UID] = true
				delete(s.bodies, a.Key.String())
				if err := s.Store.RemoveRaw(a.Key); err != nil {
					return err
				}
			}
		}
		// 再写一次成功结果，防止期间完成的同步用旧标记覆盖本地
		applied, err := s.applyFlags(confirmed, false)
		if err != nil {
			return err
		}
		undone, err := s.applyFlags(reverted, true)
		if err != nil {
			return err
		}
		changedAccounts := map[string]bool{}
		for folderKey, uids := range removed {
			account, folder, _ := strings.Cut(folderKey, "\x00")
			idx, err := s.Store.Index(account, folder)
			if err != nil {
				return err
			}
			kept := idx.Messages[:0]
			for _, m := range idx.Messages {
				if !uids[m.Key.UID] {
					kept = append(kept, m)
				}
			}
			idx.Messages = kept
			if err = s.Store.SaveIndex(account, folder); err != nil {
				return err
			}
			changedAccounts[account] = true
		}
		if applied || undone || len(removed) > 0 {
			if err := s.refresh(); err != nil {
				return err
			}
		}
		for account := range changedAccounts {
			if _, err := s.startSync(account); err != nil {
				return err
			}
		}
		if failed > 0 {
			return fmt.Errorf("%d 封邮件操作失败: %s", failed, strings.Join(reasons, "; "))
		}
		return nil
	}, accountIDs...)
}

// applyFlags 将已读/红旗变更写入本地索引，revert 为真时恢复原值；返回是否写入，由调用方刷新视图。
func (s *Session) applyFlags(actions []mutation, revert bool) (bool, error) {
	byFolder := map[string]map[uint32]mutation{}
	for _, a := range actions {
		if a.Op != "seen" && a.Op != "flagged" {
			continue
		}
		folderKey := a.Key.Account + "\x00" + a.Key.Folder
		if byFolder[folderKey] == nil {
			byFolder[folderKey] = map[uint32]mutation{}
		}
		byFolder[folderKey][a.Key.UID] = a
	}
	if len(byFolder) == 0 {
		return false, nil
	}
	for folderKey, changes := range byFolder {
		account, folder, _ := strings.Cut(folderKey, "\x00")
		idx, err := s.Store.Index(account, folder)
		if err != nil {
			return false, err
		}
		for i, m := range idx.Messages {
			a, ok := changes[m.Key.UID]
			if !ok {
				continue
			}
			value, _ := a.Value.(bool)
			if revert {
				value = !value
			}
			if a.Op == "seen" {
				idx.Messages[i].Flags.Seen = value
			} else {
				idx.Messages[i].Flags.Flagged = value
			}
		}
		if err = s.Store.SaveIndex(account, folder); err != nil {
			return false, err
		}
	}
	return true, nil
}

// loadBody 获取用于回复/转发的本地 MIME 正文。
func (s *Session) loadBody(k mail.Key) (mail.Body, error) {
	_ = s.messageResult(k.String())
	b, ok := s.bodies[k.String()]
	if !ok {
		return mail.Body{}, fmt.Errorf("正文尚未下载，请先打开邮件")
	}
	return b, nil
}

// attachmentFromPath 读取有界附件，禁止目录和超大文件。
func attachmentFromPath(path string) ([]byte, error) {
	f, e := os.Open(path)
	if e != nil {
		return nil, e
	}
	defer f.Close()
	info, e := f.Stat()
	if e != nil {
		return nil, e
	}
	if !info.Mode().IsRegular() || info.Size() > 50<<20 {
		return nil, fmt.Errorf("附件必须是小于 50 MiB 的普通文件")
	}
	return readLimited(f, 50<<20)
}
