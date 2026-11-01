package engine

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"path/filepath"
	"strings"
	"time"

	"github.com/openfabrica/mailfabrica/internal/mail"
	"github.com/openfabrica/mailfabrica/internal/protocol"
	"github.com/openfabrica/mailfabrica/internal/store"
)

// dispatch 将全部稳定命令分发到领域实现。
func (s *Session) dispatch(id string, p Params) (any, error) {
	switch {
	case strings.HasPrefix(id, "account."):
		return s.accountCommand(id, p)
	case strings.HasPrefix(id, "view.") || strings.HasPrefix(id, "folder.") || id == "mail.sync":
		return s.viewCommand(id, p)
	case strings.HasPrefix(id, "message."):
		return s.messageCommand(id, p)
	case strings.HasPrefix(id, "compose."):
		return s.composeCommand(id, p)
	case strings.HasPrefix(id, "ai."):
		return s.aiCommand(id, p)
	case strings.HasPrefix(id, "invoice."):
		return s.invoiceCommand(id, p)
	case strings.HasPrefix(id, "calendar."):
		return s.calendarCommand(id, p)
	case strings.HasPrefix(id, "update."):
		return s.updateCommand(id, p)
	case strings.HasPrefix(id, "contacts."):
		return s.contactsCommand(id, p)
	}
	switch id {
	case "app.status":
		return s.status, nil
	case "app.prefs":
		if len(p) == 0 {
			return s.prefs, nil
		}
		next := Params{}
		for k, v := range s.prefs {
			next[k] = v
		}
		for k, v := range p {
			old, ok := next[k]
			if !ok {
				return nil, fmt.Errorf("未知偏好: %s", k)
			}
			switch old.(type) {
			case []any:
				items, ok := v.([]any)
				if !ok || len(items) > 2000 {
					return nil, fmt.Errorf("%s 必须是字符串数组", k)
				}
				for _, item := range items {
					if _, ok := item.(string); !ok {
						return nil, fmt.Errorf("%s 必须是字符串数组", k)
					}
				}
			case bool:
				if _, ok := v.(bool); !ok {
					return nil, fmt.Errorf("%s 必须是布尔值", k)
				}
			case string:
				if _, ok := v.(string); !ok {
					return nil, fmt.Errorf("%s 必须是字符串", k)
				}
			case float64:
				n, ok := v.(float64)
				if !ok || n < 0 || n > 10000 || n != float64(int(n)) {
					return nil, fmt.Errorf("%s 数值无效", k)
				}
			}
			next[k] = v
		}
		if e := store.WriteJSON(filepath.Join(s.Store.Root, "prefs.json"), next); e != nil {
			return nil, e
		}
		s.prefs = next
		// 列表行只受 groupBySubject 影响；宽度、折叠等偏好不必重算全部邮件，避免拖动边栏后卡顿
		if _, ok := p["groupBySubject"]; !ok {
			return s.prefs, nil
		}
		return s.prefs, s.refresh()
	case "app.dialog":
		name, e := required(p, "name")
		if e != nil {
			return nil, e
		}
		switch name {
		case "settings", "contacts", "about", "updates":
		default:
			return nil, fmt.Errorf("未知对话框")
		}
		s.requests = append(s.requests, UIRequest{Kind: "dialog", Name: name})
		return nil, nil
	}
	return nil, fmt.Errorf("未知命令 %s", id)
}

// accountCommand 实现账户检测和持久化，凭据不进入返回对象。
func (s *Session) accountCommand(id string, p Params) (any, error) {
	switch id {
	case "account.list":
		return s.Store.Accounts, nil
	case "account.detect":
		email, e := required(p, "email")
		if e != nil {
			return nil, e
		}
		candidates, e := mail.ServerCandidates(email, str(p, "provider"))
		if e != nil {
			return nil, e
		}
		return Params{"imap": candidates[0][0], "smtp": candidates[0][1]}, nil
	case "account.remove":
		key, e := required(p, "id")
		if e != nil {
			return nil, e
		}
		if _, e = s.account(key); e != nil {
			return nil, e
		}
		if s.accountBusy[key] > 0 {
			return nil, fmt.Errorf("请等待账户任务结束")
		}
		if e = s.secrets.Set(key, ""); e != nil {
			return nil, e
		}
		if e = s.Store.RemoveAccount(key); e != nil {
			return nil, e
		}
		return nil, s.refresh()
	}
	if len(p) == 0 {
		name := "accountAdd"
		if id == "account.update" {
			name = "accountEdit"
		}
		s.requests = append(s.requests, UIRequest{Kind: "dialog", Name: name})
		return nil, nil
	}
	var a mail.Account
	var e error
	if id == "account.add" {
		email, err := required(p, "email")
		if err != nil {
			return nil, err
		}
		addresses, err := mail.ParseAddresses(email)
		if err != nil || len(addresses) != 1 {
			return nil, fmt.Errorf("邮箱地址无效")
		}
		candidates, err := mail.ServerCandidates(email, str(p, "provider"))
		if err != nil {
			return nil, err
		}
		i, t := candidates[0][0], candidates[0][1]
		a = mail.Account{ID: mail.AccountID(email), Email: email, Login: email, Name: str(p, "name"), IMAP: i, SMTP: t}
		if _, err = s.account(a.ID); err == nil {
			return nil, fmt.Errorf("账户已经存在")
		}
	} else {
		key, err := required(p, "id")
		if err != nil {
			return nil, err
		}
		a, e = s.account(key)
		if e != nil {
			return nil, e
		}
	}
	if s.accountBusy[a.ID] > 0 {
		return nil, fmt.Errorf("账户正在处理任务")
	}
	for k, dest := range map[string]*string{"name": &a.Name, "login": &a.Login, "signature": &a.Signature, "imapHost": &a.IMAP.Host, "smtpHost": &a.SMTP.Host, "imapSecurity": &a.IMAP.Security, "smtpSecurity": &a.SMTP.Security} {
		if v, ok := p[k]; ok {
			value, ok := v.(string)
			if !ok {
				return nil, fmt.Errorf("%s 必须是字符串", k)
			}
			*dest = value
		}
	}
	for k, dest := range map[string]*uint16{"imapPort": &a.IMAP.Port, "smtpPort": &a.SMTP.Port} {
		if _, ok := p[k]; ok {
			n := number(p, k, 0)
			if n < 1 || n > 65535 {
				return nil, fmt.Errorf("端口无效")
			}
			*dest = uint16(n)
		}
	}
	for _, v := range []mail.Server{a.IMAP, a.SMTP} {
		if strings.TrimSpace(v.Host) == "" {
			return nil, fmt.Errorf("服务器不能为空")
		}
		if v.Security != "tls" && v.Security != "starttls" && v.Security != "none" {
			return nil, fmt.Errorf("加密方式无效")
		}
	}
	password, hasPassword := p["password"]
	secret := ""
	if hasPassword {
		var ok bool
		secret, ok = password.(string)
		if !ok {
			return nil, fmt.Errorf("password 必须是字符串")
		}
	} else {
		secret, e = s.secrets.Get(a.ID)
		if e != nil {
			return nil, e
		}
	}
	save := func() error {
		if hasPassword {
			if e := s.secrets.Set(a.ID, secret); e != nil {
				return e
			}
		}
		found := false
		for i, old := range s.Store.Accounts {
			if old.ID == a.ID {
				s.Store.Accounts[i] = a
				found = true
				break
			}
		}
		if !found {
			s.Store.Accounts = append(s.Store.Accounts, a)
		}
		if e := s.Store.SaveAccounts(); e != nil {
			return e
		}
		s.requests = append(s.requests, UIRequest{Kind: "accountAdded", Name: a.ID})
		if err := s.refresh(); err != nil {
			return err
		}
		if id == "account.add" {
			_, err := s.startSync(a.ID)
			return err
		}
		return nil
	}
	if id == "account.add" && boolean(p, "verify", true) {
		if secret == "" {
			return nil, fmt.Errorf("请输入授权码或密码")
		}
		candidates, err := mail.ServerCandidates(a.Email, str(p, "provider"))
		if err != nil {
			return nil, err
		}
		if a.IMAP != candidates[0][0] {
			candidates = [][2]mail.Server{{a.IMAP, a.SMTP}}
		}
		e = s.launch(a.ID, func(ctx context.Context) (any, error) {
			var last error
			for _, servers := range candidates {
				candidate := a
				candidate.IMAP = servers[0]
				if str(p, "smtpHost") == "" {
					candidate.SMTP = servers[1]
				}
				last = s.remote.Verify(ctx, candidate, secret)
				if last == nil {
					return candidate, nil
				}
				var network net.Error
				if !errors.As(last, &network) && !errors.Is(last, io.EOF) && !errors.Is(last, io.ErrUnexpectedEOF) {
					break
				}
			}
			return nil, last
		}, func(v any) error {
			chosen, ok := v.(mail.Account)
			if !ok {
				return fmt.Errorf("账户验证结果无效")
			}
			a = chosen
			return save()
		}, a.ID)
		if e != nil {
			return nil, e
		}
	} else {
		if e = save(); e != nil {
			return nil, e
		}
	}
	return Params{"id": a.ID}, nil
}

// contactsCommand 维护按邮箱去重的地址簿。
func (s *Session) contactsCommand(id string, p Params) (any, error) {
	if id == "contacts.timeline" {
		return s.contactTimeline(p)
	}
	if id == "contacts.suggest" {
		return s.suggestContacts(p)
	}
	if id == "contacts.list" {
		q := strings.ToLower(str(p, "query"))
		out := []mail.Contact{}
		for _, c := range s.Store.Contacts {
			c.Name = protocol.DecodeHeader(c.Name)
			if q == "" || strings.Contains(strings.ToLower(c.Name+" "+c.Email), q) {
				out = append(out, c)
			}
		}
		return out, nil
	}
	email, e := required(p, "email")
	if e != nil {
		return nil, e
	}
	a, e := mail.ParseAddresses(email)
	if e != nil || len(a) != 1 {
		return nil, fmt.Errorf("邮箱地址无效")
	}
	index := -1
	for i, c := range s.Store.Contacts {
		if strings.EqualFold(c.Email, a[0].Email) {
			index = i
			break
		}
	}
	if id == "contacts.remove" {
		if index < 0 {
			return nil, fmt.Errorf("联系人不存在")
		}
		s.Store.Contacts = append(s.Store.Contacts[:index], s.Store.Contacts[index+1:]...)
	} else {
		c := mail.Contact{Email: a[0].Email, Name: str(p, "name"), Manual: true, Last: time.Now().Unix()}
		if index < 0 {
			s.Store.Contacts = append(s.Store.Contacts, c)
		} else {
			c.Count = s.Store.Contacts[index].Count
			s.Store.Contacts[index] = c
		}
	}
	return nil, s.Store.SaveContacts()
}

// marshalParams 用于内部组合既有命令参数。
func marshalParams(p Params) json.RawMessage { b, _ := json.Marshal(p); return b }
