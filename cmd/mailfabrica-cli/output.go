package main

import (
	"encoding/json"
	"fmt"
	"github.com/openfabrica/mailfabrica/internal/engine"
	"github.com/openfabrica/mailfabrica/internal/mail"
	"io"
	"strings"
	"time"
)

// writeText 保留面向人的默认 CLI 输出，--json 和 exec 仍返回原始结构。
func writeText(w io.Writer, command string, raw json.RawMessage) error {
	switch command {
	case "accounts":
		var accounts []mail.Account
		if e := json.Unmarshal(raw, &accounts); e != nil {
			return e
		}
		for _, a := range accounts {
			fmt.Fprintf(w, "%s\t%s\t%s\tIMAP %s:%d\tSMTP %s:%d\n", a.ID, a.Email, a.Name, a.IMAP.Host, a.IMAP.Port, a.SMTP.Host, a.SMTP.Port)
		}
	case "folders":
		var folders []mail.Folder
		if e := json.Unmarshal(raw, &folders); e != nil {
			return e
		}
		for _, f := range folders {
			fmt.Fprintf(w, "%s\t%s\t%d/%d\n", f.Path, f.Role, f.Unread, f.Total)
		}
	case "list", "search":
		var result struct {
			Messages []mail.Message `json:"messages"`
			Rows     []engine.Row   `json:"rows"`
		}
		if e := json.Unmarshal(raw, &result); e != nil {
			return e
		}
		messages := result.Messages
		if result.Rows != nil {
			for _, r := range result.Rows {
				m := r.Message
				m.Flags.Seen = r.Unread == 0
				m.Flags.Flagged = r.Flagged
				if r.Count > 1 {
					m.From.Name = strings.Join(r.Senders, ", ") + fmt.Sprintf(" (%d)", r.Count)
				}
				messages = append(messages, m)
			}
		}
		for _, m := range messages {
			flags := []byte("----")
			if !m.Flags.Seen {
				flags[0] = 'U'
			}
			if m.Flags.Flagged {
				flags[1] = 'F'
			}
			if m.Flags.Answered {
				flags[2] = 'R'
			}
			if m.HasAttachments {
				flags[3] = 'A'
			}
			sender := m.From.Name
			if sender == "" {
				sender = m.From.Email
			}
			fmt.Fprintf(w, "%s\t%s\t%s\t%s\t%s\n", m.Key.String(), string(flags), time.Unix(m.Date, 0).In(time.Local).Format("2006-01-02 15:04"), sender, m.Subject)
		}
	case "read":
		var message struct {
			Meta  mail.Message `json:"meta"`
			Body  *mail.Body   `json:"body"`
			State string       `json:"state"`
		}
		if e := json.Unmarshal(raw, &message); e != nil {
			return e
		}
		fmt.Fprintf(w, "Subject: %s\nFrom: %s\nDate: %s\n\n", message.Meta.Subject, message.Meta.From.Display(), time.Unix(message.Meta.Date, 0).Format(time.RFC1123))
		if message.Body == nil {
			return fmt.Errorf("正文未就绪: %s", message.State)
		}
		fmt.Fprintln(w, message.Body.Text)
		for _, a := range message.Body.Attachments {
			fmt.Fprintf(w, "[%d] %s (%d bytes)\n", a.Index, a.Name, a.Size)
		}
	case "commands":
		var commands []engine.Command
		if e := json.Unmarshal(raw, &commands); e != nil {
			return e
		}
		for _, c := range commands {
			enabled := "disabled"
			if c.Enabled {
				enabled = "enabled"
			}
			fmt.Fprintf(w, "%s\t%s\t%s\n", c.ID, enabled, c.Params)
		}
	case "send":
		fmt.Fprintln(w, "Sent.")
	case "sync":
		fmt.Fprintln(w, "Sync complete.")
	default:
		_, e := fmt.Fprintln(w, string(raw))
		return e
	}
	return nil
}
