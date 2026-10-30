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
	case "folder.markAllRead":
		keys := []any{}
		all, e := s.allMessages()
		if e != nil {
			return nil, e
		}
		account := str(p, "account")
		folder := str(p, "folder")
		if account == "" {
			account = s.view.Scope.Account
		}
		if folder == "" {
			folder = s.view.Scope.Folder
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
		if !boolean(p, "extend", false) {
			s.view.Selected = []string{}
		}
		for _, k := range keys {
			found := -1
			for i, old := range s.view.Selected {
				if old == k {
					found = i
					break
				}
			}
			if found >= 0 && boolean(p, "extend", false) {
				s.view.Selected = append(s.view.Selected[:found], s.view.Selected[found+1:]...)
			} else if found < 0 {
				s.view.Selected = append(s.view.Selected, k)
			}
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
