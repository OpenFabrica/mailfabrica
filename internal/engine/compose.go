package engine

import (
	"context"
	"encoding/base64"
	"fmt"
	"html"
	"io"
	"mime"
	"path/filepath"
	"strings"

	"github.com/openfabrica/mailfabrica/internal/mail"
	"github.com/openfabrica/mailfabrica/internal/protocol"
)

// readLimited 拒绝超过指定大小的文件或响应。
func readLimited(r io.Reader, max int64) ([]byte, error) {
	b, e := io.ReadAll(io.LimitReader(r, max+1))
	if e != nil {
		return nil, e
	}
	if int64(len(b)) > max {
		return nil, fmt.Errorf("内容超过大小限制")
	}
	return b, nil
}

// composeByID 校验写信窗口存在且当前可编辑。
func (s *Session) composeByID(p Params) (*Compose, error) {
	id := uint64(number(p, "id", 0))
	c, ok := s.composes[id]
	if !ok {
		return nil, fmt.Errorf("写信窗口不存在")
	}
	if c.Sending {
		return nil, fmt.Errorf("邮件正在发送或保存")
	}
	return c, nil
}

// addressesText 格式化收件人输入值。
func addressesText(addresses []mail.Address) string {
	out := []string{}
	for _, a := range addresses {
		out = append(out, a.Display())
	}
	return strings.Join(out, ", ")
}

// createCompose 建立独立写信状态，回复引用始终与可编辑正文分离。
func (s *Session) createCompose(id string, p Params) (any, error) {
	account := str(p, "account")
	if account == "" && len(s.Store.Accounts) > 0 {
		account = s.Store.Accounts[0].ID
	}
	var source *mail.Key
	var body mail.Body
	var e error
	if id != "compose.new" {
		k, err := s.focusedKey(p)
		if err != nil {
			return nil, err
		}
		source = &k
		account = k.Account
		body, e = s.loadBody(k)
		if e != nil {
			return nil, e
		}
	}
	a, e := s.account(account)
	if e != nil {
		return nil, e
	}
	s.nextID++
	c := &Compose{Background: boolean(p, "background", false), ID: s.nextID, Mode: strings.TrimPrefix(id, "compose."), Account: account, To: str(p, "to"), CC: str(p, "cc"), BCC: str(p, "bcc"), Subject: str(p, "subject"), Body: str(p, "body"), Rich: boolean(s.prefs, "composeRich", true), Source: source, Attachments: []mail.Attachment{}, References: []string{}, Dirty: false}
	if id == "compose.new" {
		c.Mode = "new"
	}
	if source != nil {
		c.Subject = body.Subject
		c.InReplyTo = &body.MessageID
		c.References = append(append([]string{}, body.References...), "<"+body.MessageID+">")
		switch id {
		case "compose.editDraft":
			c.Mode = "draft"
			c.Draft = source
			c.Source = nil
			c.To = addressesText(body.To)
			c.CC = addressesText(body.CC)
			c.Body = body.Text
			if body.HTML != nil {
				sourceHTML := *body.HTML
				marker := strings.Index(sourceHTML, "data-mf-quote")
				if marker >= 0 {
					start := strings.LastIndex(sourceHTML[:marker], "<div")
					if start >= 0 {
						c.Quote = &Quote{HTML: sourceHTML[start:], Text: protocol.PlainText(sourceHTML[start:]), Inline: []mail.Attachment{}}
						sourceHTML = sourceHTML[:start]
					}
				}
				c.Doc, e = docFromHTML(sourceHTML)
				if e != nil {
					return nil, e
				}
				c.Body = c.Doc.Text
				c.Rich = true
			}
			c.Attachments = append(c.Attachments, body.Attachments...)
		case "compose.forward":
			c.Subject = "Fwd: " + body.Subject
			for _, a := range body.Attachments {
				c.Attachments = append(c.Attachments, a)
			}
		default:
			if !strings.HasPrefix(strings.ToLower(c.Subject), "re:") {
				c.Subject = "Re: " + c.Subject
			}
			recipients := body.ReplyTo
			if len(recipients) == 0 {
				recipients = body.From
			}
			c.To = addressesText(recipients)
			if id == "compose.replyAll" {
				seen := map[string]bool{strings.ToLower(a.Email): true}
				to := []mail.Address{}
				cc := []mail.Address{}
				for _, v := range append(recipients, body.To...) {
					key := strings.ToLower(v.Email)
					if !seen[key] {
						seen[key] = true
						to = append(to, v)
					}
				}
				for _, v := range body.CC {
					key := strings.ToLower(v.Email)
					if !seen[key] {
						seen[key] = true
						cc = append(cc, v)
					}
				}
				c.To = addressesText(to)
				c.CC = addressesText(cc)
			}
		}
		if id != "compose.editDraft" && (id == "compose.forward" || boolean(s.prefs, "quoteOnReply", true)) {
			original := "<pre style=\"white-space:pre-wrap;font:inherit\">" + html.EscapeString(body.Text) + "</pre>"
			if body.HTML != nil {
				original = *body.HTML
			}
			head := fmt.Sprintf("%s 写道：", addressesText(body.From))
			c.Quote = &Quote{HTML: "<div data-mf-quote=\"true\"><p>" + html.EscapeString(head) + "</p><blockquote>" + original + "</blockquote></div>", Text: "\n\n" + head + "\n" + body.Text, Inline: []mail.Attachment{}}
			for _, a := range body.Attachments {
				if a.Inline {
					c.Quote.Inline = append(c.Quote.Inline, a)
				}
			}
		}
	}
	if id != "compose.editDraft" && a.Signature != "" {
		c.Body += "\n\n-- \n" + a.Signature
	}
	if c.Doc.Blocks == nil {
		c.Doc = newDoc(c.Body)
	}
	c.HTML = c.Doc.HTML()
	s.composes[c.ID] = c
	if !c.Background {
		s.requests = append(s.requests, UIRequest{Kind: "openCompose", ID: c.ID})
	}
	return Params{"id": c.ID}, nil
}

// composeCommand 实现编辑、格式、附件、保存与发送。
func (s *Session) composeCommand(id string, p Params) (any, error) {
	switch id {
	case "compose.new", "compose.reply", "compose.replyAll", "compose.forward", "compose.editDraft":
		return s.createCompose(id, p)
	}
	c, e := s.composeByID(p)
	if e != nil {
		return nil, e
	}
	switch id {
	case "compose.discard":
		delete(s.composes, c.ID)
		s.requests = append(s.requests, UIRequest{Kind: "closeCompose", ID: c.ID})
		return nil, nil
	case "compose.set":
		for k, target := range map[string]*string{"account": &c.Account, "to": &c.To, "cc": &c.CC, "bcc": &c.BCC, "subject": &c.Subject, "body": &c.Body} {
			if v, ok := p[k]; ok {
				text, ok := v.(string)
				if !ok {
					return nil, fmt.Errorf("%s 必须是字符串", k)
				}
				if len([]rune(text)) > 1000000 {
					return nil, fmt.Errorf("内容过长")
				}
				*target = text
			}
		}
		if _, e = s.account(c.Account); e != nil {
			return nil, e
		}
		c.Doc.replace(c.Body, plainStyle())
	case "compose.setFormat":
		c.Rich = boolean(p, "rich", false)
		if !c.Rich {
			c.Doc = newDoc(c.Body)
		}
	case "compose.edit":
		text, ok := p["text"].(string)
		if !ok || len([]rune(text)) > 1000000 {
			return nil, fmt.Errorf("text 必须是不超过一百万字符的字符串")
		}
		style := plainStyle()
		if value, ok := p["insertStyle"].(map[string]any); ok {
			style, e = patchStyle(style, Params(value))
			if e != nil {
				return nil, e
			}
		}
		c.Doc.replace(text, style)
		c.Body = text
	case "compose.setHtml":
		source, ok := p["html"].(string)
		if !ok || len(source) > 10<<20 {
			return nil, fmt.Errorf("html 无效或过大")
		}
		c.Doc, e = docFromHTML(source)
		if e != nil {
			return nil, e
		}
		c.Body = c.Doc.Text
		c.Rich = true
	case "compose.attach":
		var data []byte
		name := str(p, "name")
		if path := str(p, "path"); path != "" {
			data, e = attachmentFromPath(path)
			name = filepath.Base(path)
		} else {
			encoded, e1 := required(p, "base64")
			if e1 != nil {
				return nil, e1
			}
			if len(encoded) > 70<<20 {
				return nil, fmt.Errorf("附件超过 50 MiB")
			}
			data, e = base64.StdEncoding.DecodeString(encoded)
		}
		if e != nil {
			return nil, e
		}
		if name == "" || len(data) > 50<<20 {
			return nil, fmt.Errorf("附件名称或大小无效")
		}
		c.Attachments = append(c.Attachments, mail.Attachment{Name: filepath.Base(name), MIME: mime.TypeByExtension(filepath.Ext(name)), Size: len(data), Data: data})
	case "compose.removeAttachment":
		i := number(p, "index", -1)
		if i < 0 || i >= len(c.Attachments) {
			return nil, fmt.Errorf("附件索引无效")
		}
		c.Attachments = append(c.Attachments[:i], c.Attachments[i+1:]...)
	case "compose.removeQuote":
		c.Quote = nil
	case "compose.send", "compose.saveDraft":
		return s.sendCompose(c, id == "compose.saveDraft")
	default:
		if !c.Rich {
			return nil, fmt.Errorf("请切换到富文本模式")
		}
		start, end := number(p, "start", -1), number(p, "end", -1)
		if start < 0 || end < 0 {
			return nil, fmt.Errorf("需要 start/end 字符偏移")
		}
		length := len([]rune(c.Body))
		if start > length {
			start = length
		}
		if end > length {
			end = length
		}
		if end < start {
			start, end = end, start
		}
		if id == "compose.block" {
			block := str(p, "block")
			switch block {
			case "paragraph", "bullet", "numbered", "quote", "heading":
			default:
				return nil, fmt.Errorf("块样式无效")
			}
			runes := []rune(c.Body)
			line := 0
			for i := 0; i <= len(runes); i++ {
				if i >= start && i <= end && line < len(c.Doc.Blocks) {
					c.Doc.Blocks[line] = block
				}
				if i < len(runes) && runes[i] == '\n' {
					line++
				}
			}
		} else {
			styles := c.Doc.styles()
			styleName := str(p, "style")
			if id == "compose.bold" || id == "compose.italic" || id == "compose.underline" {
				styleName = strings.TrimPrefix(id, "compose.")
			}
			toggle := id == "compose.toggle" || id == "compose.bold" || id == "compose.italic" || id == "compose.underline"
			turnOn := false
			if toggle {
				if styleName != "bold" && styleName != "italic" && styleName != "underline" && styleName != "strike" {
					return nil, fmt.Errorf("切换样式无效")
				}
				all := true
				for _, st := range styles[start:end] {
					on := st.Bold
					if styleName == "italic" {
						on = st.Italic
					} else if styleName == "underline" {
						on = st.Underline
					} else if styleName == "strike" {
						on = st.Strike
					}
					all = all && on
				}
				turnOn = !all
			}
			for i := start; i < end; i++ {
				switch {
				case id == "compose.clearFormat":
					styles[i] = plainStyle()
				case toggle:
					styles[i], e = patchStyle(styles[i], Params{styleName: turnOn})
				default:
					styles[i], e = patchStyle(styles[i], p)
				}
				if e != nil {
					return nil, e
				}
			}
			c.Doc.setStyles(styles)
		}
	}
	c.HTML = c.Doc.HTML()
	c.Dirty = true
	return nil, nil
}

// sendCompose 校验收件人并在后台发送；保存已发送失败不允许重复发送。
func (s *Session) sendCompose(c *Compose, draft bool) (any, error) {
	a, e := s.account(c.Account)
	if e != nil {
		return nil, e
	}
	to, e := mail.ParseAddresses(c.To)
	if e != nil {
		return nil, e
	}
	cc, e := mail.ParseAddresses(c.CC)
	if e != nil {
		return nil, e
	}
	bcc, e := mail.ParseAddresses(c.BCC)
	if e != nil {
		return nil, e
	}
	if !draft && len(to)+len(cc)+len(bcc) == 0 {
		return nil, fmt.Errorf("请填写收件人")
	}
	out := mail.Outgoing{From: mail.Address{Name: a.Name, Email: a.Email}, To: to, CC: cc, BCC: bcc, Subject: c.Subject, Text: c.Body, Attachments: append([]mail.Attachment{}, c.Attachments...), InReplyTo: c.InReplyTo, References: c.References}
	if c.Rich {
		out.HTML = c.Doc.HTML()
	}
	if c.Quote != nil {
		out.Text += c.Quote.Text
		if c.Rich {
			out.HTML += c.Quote.HTML
		}
		out.Attachments = append(out.Attachments, c.Quote.Inline...)
	}
	password, e := s.secrets.Get(a.ID)
	if e != nil {
		return nil, e
	}
	folder := ""
	saveSent := boolean(s.prefs, "saveSent", true)
	if draft {
		folder, e = s.roleFolder(a.ID, "drafts")
	} else if saveSent {
		folder, e = s.roleFolder(a.ID, "sent")
	}
	if e != nil {
		return nil, e
	}
	source := c.Source
	oldDraft := c.Draft
	c.Sending = true
	c.Error = nil
	err := s.launch(fmt.Sprintf("compose:%d", c.ID), func(ctx context.Context) (any, error) {
		var raw []byte
		var err error
		if draft {
			raw, err = protocol.Build(out)
		} else {
			raw, err = s.remote.Send(ctx, a, password, out)
		}
		warning := ""
		if err == nil && folder != "" {
			flags := []string{"\\Seen"}
			if draft {
				flags = append(flags, "\\Draft")
			}
			saveErr := s.remote.Append(ctx, a, password, folder, raw, flags)
			if draft {
				err = saveErr
			} else if saveErr != nil {
				warning = "邮件已发送，但保存已发送副本失败: " + saveErr.Error()
			}
		}
		if err == nil && oldDraft != nil {
			if deleteErr := s.remote.Change(ctx, a, password, *oldDraft, "delete", nil); deleteErr != nil {
				warning += " 草稿清理失败: " + deleteErr.Error()
			}
		}
		if err == nil && !draft && source != nil {
			if flagErr := s.remote.Change(ctx, a, password, *source, "answered", true); flagErr != nil {
				warning += " 原邮件回复标记失败: " + flagErr.Error()
			}
		}
		if err != nil {
			s.mu.Lock()
			if current, ok := s.composes[c.ID]; ok {
				current.Sending = false
				msg := err.Error()
				current.Error = &msg
				if current.Background {
					current.Background = false
					s.requests = append(s.requests, UIRequest{Kind: "openCompose", ID: current.ID})
				}
			}
			s.mu.Unlock()
		}
		return warning, err
	}, func(v any) error {
		delete(s.composes, c.ID)
		s.requests = append(s.requests, UIRequest{Kind: "closeCompose", ID: c.ID})
		for _, address := range append(append(to, cc...), bcc...) {
			exists := false
			for i, contact := range s.Store.Contacts {
				if strings.EqualFold(contact.Email, address.Email) {
					s.Store.Contacts[i].Count++
					exists = true
					break
				}
			}
			if !exists {
				s.Store.Contacts = append(s.Store.Contacts, mail.Contact{Name: address.Name, Email: address.Email, Count: 1})
			}
		}
		if e := s.Store.SaveContacts(); e != nil {
			return e
		}
		if warning, ok := v.(string); ok && warning != "" {
			s.status.Errors = append(s.status.Errors, [2]string{a.ID, warning})
		}
		_, err := s.startSync(a.ID)
		return err
	}, a.ID)
	if err != nil {
		c.Sending = false
	}
	return nil, err
}
