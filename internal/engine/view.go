package engine

import (
	"context"
	"fmt"
	"github.com/openfabrica/mailfabrica/internal/mail"
	"strings"
)

// viewCommand 维护视图、主题组及文件夹操作。
func (s *Session) viewCommand(id string, p Params) (any, error) {
	switch id {
	case "mail.sync":
		return s.startSync(str(p, "account"))
	case "folder.list":
		out := []mail.Folder{}
		for account, folders := range s.Store.Folders {
			if str(p, "account") == "" || str(p, "account") == account {
				out = append(out, folders...)
			}
		}
		return out, nil
	case "folder.create":
		account, e := required(p, "account")
		if e != nil {
			return nil, e
		}
		name, e := required(p, "name")
		if e != nil {
			return nil, e
		}
		a, e := s.account(account)
		if e != nil {
			return nil, e
		}
		password, e := s.secrets.Get(account)
		if e != nil {
			return nil, e
		}
		return nil, s.launch("folder.create:"+account, func(ctx context.Context) (any, error) { return nil, s.remote.Create(ctx, a, password, name) }, func(any) error {
			folders := append([]mail.Folder{}, s.Store.Folders[account]...)
			folders = append(folders, mail.Folder{Account: account, Path: name, Role: mail.FolderRole(name, nil), Selectable: true})
			return s.Store.SetFolders(account, folders)
		}, account)
	case "folder.deleteMessages", "folder.junk":
		return s.folderMessages(id, p)
	case "folder.markAllRead":
		account := str(p, "account")
		folder := str(p, "folder")
		if account == "" {
			account = s.view.Scope.Account
		}
		if folder == "" {
			folder = s.view.Scope.Folder
		}
		if account != "" && folder != "" {
			return s.folderMessages(id, Params{"account": account, "folder": folder})
		}
		return s.markAllRead(account, folder)
	case "view.scope":
		scope := Scope{Kind: str(p, "kind")}
		if str(p, "account") != "" {
			scope = Scope{Kind: "folder", Account: str(p, "account"), Folder: str(p, "folder")}
			if scope.Folder == "" {
				return nil, fmt.Errorf("缺少 folder")
			}
			if _, e := s.account(scope.Account); e != nil {
				return nil, e
			}
		}
		switch scope.Kind {
		case "folder", "allInboxes", "unread", "flagged", "invoices":
		default:
			return nil, fmt.Errorf("未知邮件范围")
		}
		s.view.Scope = scope
		s.view.Query = ""
		s.view.Selected = []string{}
		s.view.Focused = nil
		s.view.Group = nil
		return nil, s.refresh()
	case "view.search":
		query, v := p["query"].(string)
		if !v {
			return nil, fmt.Errorf("query 必须是字符串")
		}
		s.view.Query = query
		return nil, s.refresh()
	case "view.select":
		keys := []string{}
		if a, ok := p["keys"].([]any); ok {
			for _, v := range a {
				k, ok := v.(string)
				if !ok {
					return nil, fmt.Errorf("keys 必须是字符串数组")
				}
				if _, e := mail.ParseKey(k); e != nil {
					return nil, e
				}
				keys = append(keys, k)
			}
		} else {
			key, e := required(p, "key")
			if e != nil {
				return nil, e
			}
			if _, e = mail.ParseKey(key); e != nil {
				return nil, e
			}
			keys = append(keys, key)
		}

		if len(keys) > 10000 {
			return nil, fmt.Errorf("单次最多选择 10000 封邮件")
		}
		if !boolean(p, "open", true) {
			visible := map[string]bool{}
			for _, m := range s.view.Messages {
				visible[m.Key.String()] = true
			}
			for _, key := range keys {
				if !visible[key] {
					return nil, fmt.Errorf("只能选择当前列表中的邮件")
				}
			}
		}

		selection := []string{}
		if boolean(p, "extend", false) {
			selection = append(selection, s.view.Selected...)
		}
		membership := map[string]bool{}
		for _, key := range selection {
			membership[key] = true
		}
		seen := map[string]bool{}
		wanted, explicit := p["selected"].(bool)
		for _, key := range keys {
			if seen[key] {
				continue
			}
			seen[key] = true
			if explicit {
				membership[key] = wanted
			} else if boolean(p, "extend", false) {
				membership[key] = !membership[key]
			} else {
				membership[key] = true
			}
		}
		// 全选与主题组选择按键集合计算，避免大列表逐封线性查找产生平方耗时。
		ordered := append(selection, keys...)
		selection = []string{}
		added := map[string]bool{}
		for _, key := range ordered {
			if membership[key] && !added[key] {
				selection = append(selection, key)
				added[key] = true
			}
		}
		if len(selection) > 10000 {
			return nil, fmt.Errorf("单次最多选择 10000 封邮件")
		}
		s.view.Selected = selection
		if !boolean(p, "open", true) {
			return nil, nil
		}
		oldGroup := s.view.Group
		s.view.Group = nil
		s.view.Focused = nil
		if len(keys) > 0 {
			k := keys[len(keys)-1]
			s.view.Focused = &k
			if oldGroup != nil {
				for _, r := range s.view.Rows {
					if r.Group != nil && *r.Group == *oldGroup {
						for _, member := range r.Keys {
							if member == k {
								s.view.Group = oldGroup
							}
						}
					}
				}
			}
			parsed, e := mail.ParseKey(k)
			if e != nil {
				return nil, e
			}
			if e = s.openBody(parsed); e != nil {
				return nil, e
			}
			if boolean(s.prefs, "markReadOnOpen", true) {
				_, _ = s.messageCommand("message.markRead", Params{"keys": []any{k}, "read": true})
			}
		}
		return nil, nil
	case "view.groups":
		groups := []Row{}
		for _, r := range s.view.Rows {
			if r.Group != nil {
				groups = append(groups, r)
			}
		}
		return Params{"enabled": boolean(s.prefs, "groupBySubject", true), "rows": len(s.view.Rows), "open": s.view.Group, "groups": groups}, nil
	case "view.selectGroup":
		group := str(p, "group")
		if group == "" {
			group = mail.SubjectKey(str(p, "subject"))
		}
		for _, r := range s.view.Rows {
			match := r.Group != nil && *r.Group == group
			if key := str(p, "key"); key != "" {
				for _, k := range r.Keys {
					if k == key && r.Group != nil {
						match = true
					}
				}
			}
			if match {
				s.view.Group = r.Group
				s.view.Focused = nil
				s.view.Selected = append([]string{}, r.Keys...)
				return r, nil
			}
		}
		return nil, fmt.Errorf("主题组不存在")
	case "view.next", "view.prev":
		index := -1
		for i, r := range s.view.Rows {
			for _, k := range r.Keys {
				if s.view.Focused != nil && k == *s.view.Focused || s.view.Group != nil && r.Group != nil && *r.Group == *s.view.Group {
					index = i
				}
			}
		}
		if strings.HasSuffix(id, "prev") {
			index--
		} else {
			index++
		}
		if index < 0 || index >= len(s.view.Rows) {
			return nil, fmt.Errorf("已到列表边界")
		}
		r := s.view.Rows[index]
		if r.Group != nil {
			return s.viewCommand("view.selectGroup", Params{"group": *r.Group})
		}
		return s.viewCommand("view.select", Params{"key": r.Keys[0]})
	case "view.groupNext", "view.groupPrev":
		if s.view.Group == nil || s.view.Focused == nil {
			return nil, fmt.Errorf("没有打开主题组邮件")
		}
		for _, r := range s.view.Rows {
			if r.Group == nil || *r.Group != *s.view.Group {
				continue
			}
			for i, k := range r.Keys {
				if k != *s.view.Focused {
					continue
				}
				if id == "view.groupNext" {
					i++
				} else {
					i--
				}
				if i < 0 || i >= len(r.Keys) {
					return nil, fmt.Errorf("已到主题组边界")
				}
				group := s.view.Group
				result, e := s.viewCommand("view.select", Params{"key": r.Keys[i]})
				s.view.Group = group
				return result, e
			}
		}
		return nil, fmt.Errorf("主题组不存在")
	}
	return nil, fmt.Errorf("未知视图命令")
}

// markAllRead 将账户/文件夹内全部未读邮件标为已读；空字符串表示不限。
func (s *Session) markAllRead(account, folder string) (any, error) {
	keys := []any{}
	all, e := s.allMessages()
	if e != nil {
		return nil, e
	}
	for _, m := range all {
		if (account == "" || m.Key.Account == account) && (folder == "" || m.Key.Folder == folder) && !m.Flags.Seen {
			keys = append(keys, m.Key.String())
		}
	}
	if len(keys) == 0 {
		return nil, nil
	}
	return s.messageCommand("message.markRead", Params{"keys": keys, "read": true})
}

// folderMessages 对显式指定的可选文件夹批量处理，不依赖当前搜索、分页和选择。
func (s *Session) folderMessages(id string, p Params) (any, error) {
	account, err := required(p, "account")
	if err != nil {
		return nil, err
	}
	folder, err := required(p, "folder")
	if err != nil {
		return nil, err
	}
	a, err := s.account(account)
	if err != nil {
		return nil, err
	}
	found := false
	for _, f := range s.Store.Folders[account] {
		if f.Path == folder && f.Selectable {
			found = true
			break
		}
	}
	if !found {
		return nil, fmt.Errorf("文件夹不存在或不可选择")
	}

	op, dest := "seen", ""
	if id == "folder.junk" || id == "folder.deleteMessages" {
		role := "junk"
		if id == "folder.deleteMessages" {
			role = "trash"
		}
		dest, err = s.roleFolder(account, role)
		if err != nil {
			return nil, err
		}
		op = "move"
		if dest == folder {
			if id == "folder.junk" {
				return nil, fmt.Errorf("邮件已在垃圾邮件文件夹中")
			}
			op = "delete"
		}
	}
	password, err := s.secrets.Get(account)
	if err != nil {
		return nil, err
	}
	return nil, s.launch("folder.mutate:"+account, func(ctx context.Context) (any, error) {
		// 将远端错误作为结果传回，使部分成功时仍能重新同步真实状态。
		return s.remote.ChangeFolder(ctx, a, password, folder, op, dest), nil
	}, func(result any) error {
		_, syncErr := s.startSyncFolders(account, false, []string{folder, dest})
		if remoteErr, ok := result.(error); ok {
			return remoteErr
		}
		return syncErr
	}, account)
}
