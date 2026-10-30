// Package protocol 封装邮件编码和远端传输，不持有本地存储。
package protocol

import (
	"bytes"
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"html"
	"io"
	"mime"
	stdmail "net/mail"
	"strings"
	"time"

	_ "github.com/emersion/go-message/charset"
	message "github.com/emersion/go-message/mail"
	"github.com/microcosm-cc/bluemonday"
	"github.com/openfabrica/mailfabrica/internal/mail"
)

const MaxMessage = 100 << 20

// addressList 将标准库地址转换为领域地址。
func addressList(a []*stdmail.Address) []mail.Address {
	out := []mail.Address{}
	for _, v := range a {
		out = append(out, mail.Address{Name: DecodeHeader(v.Name), Email: v.Address})
	}
	return out
}

// Parse 解析有界 MIME 原文，保留 HTML、附件和回复引用头。
func Parse(raw []byte) (mail.Body, error) {
	b := mail.Body{From: []mail.Address{}, To: []mail.Address{}, CC: []mail.Address{}, ReplyTo: []mail.Address{}, Attachments: []mail.Attachment{}, References: []string{}}
	if len(raw) > MaxMessage {
		return b, fmt.Errorf("邮件超过 100 MiB")
	}
	r, e := message.CreateReader(bytes.NewReader(raw))
	if e != nil && !strings.Contains(e.Error(), "charset") {
		return b, e
	}
	if r == nil {
		return b, fmt.Errorf("无法解析 MIME")
	}
	defer r.Close()
	b.Subject = DecodeHeader(r.Header.Get("Subject"))
	b.MessageID, _ = r.Header.MessageID()
	a, _ := r.Header.AddressList("From")
	b.From = addressList(a)
	a, _ = r.Header.AddressList("To")
	b.To = addressList(a)
	a, _ = r.Header.AddressList("Cc")
	b.CC = addressList(a)
	a, _ = r.Header.AddressList("Reply-To")
	b.ReplyTo = addressList(a)
	d, _ := r.Header.Date()
	b.Date = d.Unix()
	if v := r.Header.Get("In-Reply-To"); v != "" {
		v = strings.Trim(v, "<>")
		b.InReplyTo = &v
	}
	b.References = strings.Fields(r.Header.Get("References"))
	for n := 0; n < 1000; n++ {
		part, err := r.NextPart()
		if err == io.EOF {
			break
		}
		if err != nil {
			return b, err
		}
		data, err := io.ReadAll(io.LimitReader(part.Body, MaxMessage+1))
		if err != nil {
			return b, err
		}
		if len(data) > MaxMessage {
			return b, fmt.Errorf("MIME 部分过大")
		}
		ct, _, _ := mime.ParseMediaType(part.Header.Get("Content-Type"))
		disp, params, _ := mime.ParseMediaType(part.Header.Get("Content-Disposition"))
		cid := strings.Trim(part.Header.Get("Content-ID"), "<>")
		name := params["filename"]
		if name == "" {
			_, p, _ := mime.ParseMediaType(part.Header.Get("Content-Type"))
			name = p["name"]
		}
		if disp != "attachment" && name == "" && ct == "text/plain" {
			b.Text += string(data)
			continue
		}
		if disp != "attachment" && name == "" && ct == "text/html" {
			h := string(data)
			b.HTML = &h
			continue
		}
		if name == "" {
			name = "attachment"
		}
		if decoded, err := new(mime.WordDecoder).DecodeHeader(name); err == nil {
			name = decoded
		}
		at := mail.Attachment{Index: len(b.Attachments), Name: name, MIME: ct, Size: len(data), Inline: disp == "inline" || cid != "", Data: data}
		if cid != "" {
			at.ContentID = &cid
		}
		b.Attachments = append(b.Attachments, at)
	}
	if b.Text == "" && b.HTML != nil {
		b.Text = PlainText(*b.HTML)
	}
	return b, nil
}

// PlainText 为预览、纯文本发送和无 HTML 回退移除标签。
func PlainText(s string) string {
	return strings.TrimSpace(html.UnescapeString(bluemonday.StrictPolicy().Sanitize(strings.ReplaceAll(strings.ReplaceAll(s, "</p>", "</p>\n"), "<br>", "\n"))))
}

// Metadata 从 MIME 视图提取稳定索引数据。
func Metadata(key mail.Key, b mail.Body, size int, flags mail.Flags) mail.Message {
	from := mail.Address{}
	if len(b.From) > 0 {
		from = b.From[0]
	}
	r := []rune(strings.Join(strings.Fields(b.Text), " "))
	if len(r) > 160 {
		r = r[:160]
	}
	return mail.Message{Key: key, MessageID: b.MessageID, Subject: b.Subject, From: from, To: b.To, CC: b.CC, Date: b.Date, Size: uint32(size), Flags: flags, HasAttachments: len(b.Attachments) > 0, Preview: string(r), InReplyTo: b.InReplyTo, References: b.References}
}

// Build 生成包含纯文本/HTML/附件的 MIME；BCC 只放信封，不写进邮件头。
func Build(o mail.Outgoing) ([]byte, error) {
	if strings.ContainsAny(o.Subject, "\r\n") {
		return nil, fmt.Errorf("主题不能包含换行")
	}
	var h message.Header
	toStd := func(a []mail.Address) []*stdmail.Address {
		out := []*stdmail.Address{}
		for _, v := range a {
			out = append(out, &stdmail.Address{Name: v.Name, Address: v.Email})
		}
		return out
	}
	h.SetAddressList("From", toStd([]mail.Address{o.From}))
	h.SetAddressList("To", toStd(o.To))
	if len(o.CC) > 0 {
		h.SetAddressList("Cc", toStd(o.CC))
	}
	h.SetSubject(o.Subject)
	date := time.Now()
	if o.Date != 0 {
		date = time.Unix(o.Date, 0)
	}
	h.SetDate(date)
	id := make([]byte, 16)
	if _, e := rand.Read(id); e != nil {
		return nil, e
	}
	h.SetMessageID(hex.EncodeToString(id) + "@mailfabrica.local")
	if o.InReplyTo != nil {
		h.Set("In-Reply-To", "<"+strings.Trim(*o.InReplyTo, "<>")+">")
	}
	if len(o.References) > 0 {
		h.Set("References", strings.Join(o.References, " "))
	}
	var out bytes.Buffer
	w, e := message.CreateWriter(&out, h)
	if e != nil {
		return nil, e
	}
	iw, e := w.CreateInline()
	if e != nil {
		return nil, e
	}
	for _, p := range []struct{ kind, body string }{{"text/plain", o.Text}, {"text/html", o.HTML}} {
		if p.body == "" && p.kind == "text/html" {
			continue
		}
		var ph message.InlineHeader
		ph.Set("Content-Type", p.kind+"; charset=utf-8")
		pw, err := iw.CreatePart(ph)
		if err != nil {
			return nil, err
		}
		if _, err = io.WriteString(pw, p.body); err != nil {
			return nil, err
		}
		if err = pw.Close(); err != nil {
			return nil, err
		}
	}
	if e = iw.Close(); e != nil {
		return nil, e
	}
	for _, a := range o.Attachments {
		var ah message.AttachmentHeader
		ah.SetFilename(a.Name)
		kind := a.MIME
		if kind == "" {
			kind = "application/octet-stream"
		}
		ah.Set("Content-Type", kind)
		if a.ContentID != nil {
			ah.Set("Content-ID", "<"+*a.ContentID+">")
		}
		pw, err := w.CreateAttachment(ah)
		if err != nil {
			return nil, err
		}
		if _, err = pw.Write(a.Data); err != nil {
			return nil, err
		}
		if err = pw.Close(); err != nil {
			return nil, err
		}
	}
	if e = w.Close(); e != nil {
		return nil, e
	}
	return out.Bytes(), nil
}
