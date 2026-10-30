package engine

import (
	"context"
	"fmt"
	"github.com/openfabrica/mailfabrica/internal/invoice"
	"github.com/openfabrica/mailfabrica/internal/mail"
	"github.com/openfabrica/mailfabrica/internal/protocol"
)

// InvoiceState 是可轮询的发票导出进度。
type InvoiceState struct {
	Busy   bool    `json:"busy"`
	Dir    string  `json:"dir"`
	Total  int     `json:"total"`
	Done   int     `json:"done"`
	Files  int     `json:"files"`
	Failed int     `json:"failed"`
	CSV    *string `json:"csv"`
	Error  *string `json:"error"`
}

// invoiceCommand 将筛选和导出统一暴露给桌面与自动化。
func (s *Session) invoiceCommand(id string, p Params) (any, error) {
	if id == "invoice.status" {
		return s.invoice, nil
	}
	a, b, e := dateRange(p, true)
	if e != nil {
		return nil, e
	}
	all, e := s.allMessages()
	if e != nil {
		return nil, e
	}
	messages := []mail.Message{}
	for _, m := range all {
		role := mail.FolderRole(m.Key.Folder, nil)
		for _, f := range s.Store.Folders[m.Key.Account] {
			if f.Path == m.Key.Folder {
				role = f.Role
			}
		}
		if mail.IsInvoice(m) && role != "sent" && role != "drafts" && m.Date >= a.Unix() && m.Date < b.Unix() {
			messages = append(messages, m)
		}
	}
	if id == "invoice.list" {
		return Params{"total": len(messages), "messages": messages}, nil
	}
	if s.invoice.Busy {
		return nil, fmt.Errorf("发票正在导出")
	}
	dir, e := required(p, "dir")
	if e != nil {
		return nil, e
	}
	if _, ok := p["keys"]; ok {
		keys, e := s.selectedKeys(p)
		if e != nil {
			return nil, e
		}
		messages = []mail.Message{}
		for _, k := range keys {
			m, e := s.findMeta(k)
			if e != nil {
				return nil, e
			}
			messages = append(messages, m)
		}
	}
	accounts := map[string]mail.Account{}
	passwords := map[string]string{}
	raws := map[string][]byte{}
	for _, m := range messages {
		a, e := s.account(m.Key.Account)
		if e != nil {
			return nil, e
		}
		accounts[a.ID] = a
		password, e := s.secrets.Get(a.ID)
		if e != nil {
			return nil, e
		}
		passwords[a.ID] = password
		if raw, e := s.Store.Raw(m.Key); e == nil {
			raws[m.Key.String()] = raw
		}
	}
	s.invoice = InvoiceState{Busy: true, Dir: dir, Total: len(messages)}
	e = s.launch("invoice", func(ctx context.Context) (value any, taskErr error) {
		defer func() {
			if taskErr != nil {
				s.mu.Lock()
				s.invoice.Busy = false
				message := taskErr.Error()
				s.invoice.Error = &message
				s.mu.Unlock()
			}
		}()
		items := []invoice.Item{}
		for _, m := range messages {
			if e := ctx.Err(); e != nil {
				return nil, e
			}
			raw, ok := raws[m.Key.String()]
			var err error
			if !ok {
				raw, err = s.remote.Fetch(ctx, accounts[m.Key.Account], passwords[m.Key.Account], m.Key)
			}
			item := invoice.Item{Meta: m}
			if err == nil {
				item.Body, err = protocol.Parse(raw)
			}
			if err != nil {
				item.Error = err.Error()
			}
			items = append(items, item)
		}
		result, err := invoice.Export(dir, items, func(done, files int) {
			s.mu.Lock()
			s.invoice.Done = done
			s.invoice.Files = files
			s.touch()
			s.mu.Unlock()
		})
		if err != nil {
			s.mu.Lock()
			s.invoice.Busy = false
			msg := err.Error()
			s.invoice.Error = &msg
			s.mu.Unlock()
		}
		return result, err
	}, func(v any) error {
		result, ok := v.(invoice.Result)
		if !ok {
			return fmt.Errorf("导出结果无效")
		}
		s.invoice.Busy = false
		s.invoice.Files = len(result.Files)
		s.invoice.Failed = result.Failed
		s.invoice.CSV = &result.CSV
		return nil
	})
	if e != nil {
		s.invoice.Busy = false
	}
	return s.invoice, e
}
