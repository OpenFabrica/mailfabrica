package engine

import (
	"fmt"
	"github.com/openfabrica/mailfabrica/internal/mail"
	"github.com/openfabrica/mailfabrica/internal/protocol"
	"html"
	"strings"
	"time"
)

// seedDemo 构造明确标为虚构的演示邮箱，绝不读取真实凭据。
func (s *Session) seedDemo() error {
	const account = "demo-mailfabrica-dev"
	for _, a := range s.Store.Accounts {
		if a.ID != account {
			return fmt.Errorf("演示模式不能使用含真实账户的数据目录")
		}
	}
	remote := protocol.NewMemory()
	s.remote = remote
	a := mail.Account{ID: account, Name: "演示用户", Email: "demo@mailfabrica.dev", Login: "demo@mailfabrica.dev", Signature: "演示用户\nMailFabrica"}
	a.IMAP, a.SMTP = mail.DetectServers(a.Email)
	if len(s.Store.Accounts) == 0 {
		s.Store.Accounts = append(s.Store.Accounts, a)
		if e := s.Store.SaveAccounts(); e != nil {
			return e
		}
	}
	folders := []mail.Folder{}
	for _, p := range []string{"INBOX", "Drafts", "Sent", "Trash", "Junk", "Archive", "工作/产品"} {
		delim := "/"
		folders = append(folders, mail.Folder{Account: account, Path: p, Delimiter: &delim, Role: mail.FolderRole(p, nil), UIDValidity: 1, Selectable: true})
	}
	remote.Folders[account] = folders
	for _, f := range folders {
		idx, e := s.Store.Index(account, f.Path)
		if e != nil {
			return e
		}
		for _, m := range idx.Messages {
			raw, e := s.Store.Raw(m.Key)
			if e != nil {
				return e
			}
			remote.Put(account, f.Path, m.Key.UID, raw, m.Flags)
		}
	}
	idx, e := s.Store.Index(account, "INBOX")
	if e != nil {
		return e
	}
	if len(idx.Messages) == 0 {
		subjects := []string{"下周产品评审安排", "设计稿评审意见汇总", "Q3 运营数据周报", "发票开具事宜", "产品路线图更新", "用户调研访谈安排", "版本发布计划", "Re: 下周产品评审安排", "十月费用报销说明", "团队例会纪要", "欢迎使用 MailFabrica"}
		names := []string{"林晨", "王雨桐", "张伟", "陈思", "产品团队", "李娜", "技术团队", "林晨", "财务部", "市场部", "MailFabrica"}
		for i, subject := range subjects {
			body := "Hi 团队，\n\n下周的产品评审会议安排如下，请大家提前做好准备：\n\n会议时间：下周三 14:00 – 16:00\n会议地点：3 号楼 5 楼，会议室 A\n\n我们将一起讨论核心功能方案、用户反馈和下一阶段的版本规划。\n\n会议议程详见附件。如有补充议题，请在本周内告知。\n\n谢谢！\n" + names[i]
			if i > 0 {
				body = subject + "\n\n这是演示模式的虚构邮件，用于体验阅读、回复、移动、红旗和搜索功能。\n\n欢迎提出你的想法。\n\n" + names[i]
			}
			attachment := []mail.Attachment{}
			if i == 0 {
				attachment = append(attachment, mail.Attachment{Name: "评审议程.txt", MIME: "text/plain", Data: []byte("产品方案评审\n用户反馈\n下一阶段规划")})
			}
			if i == 3 {
				attachment = append(attachment, mail.Attachment{Name: "invoice.xml", MIME: "application/xml", Data: []byte("<invoice><amount>1280.00</amount></invoice>")})
			}
			out := mail.Outgoing{Date: time.Now().Add(-time.Duration(i*5) * time.Hour).Unix(), From: mail.Address{Name: names[i], Email: fmt.Sprintf("sender%d@example.com", i)}, To: []mail.Address{{Name: a.Name, Email: a.Email}}, Subject: subject, Text: body, HTML: demoHTML(body), Attachments: attachment}
			raw, e := protocol.Build(out)
			if e != nil {
				return e
			}
			parsed, e := protocol.Parse(raw)
			if e != nil {
				return e
			}
			key := mail.Key{Account: account, Folder: "INBOX", UID: uint32(i + 1)}
			flags := mail.Flags{Seen: i > 4, Flagged: i == 2}
			meta := protocol.Metadata(key, parsed, len(raw), flags)
			idx.Messages = append(idx.Messages, meta)
			s.bodies[key.String()] = parsed
			if e = s.Store.SaveRaw(key, raw); e != nil {
				return e
			}
			remote.Put(account, "INBOX", key.UID, raw, flags)
		}
		idx.UIDValidity = 1
		if e = s.Store.SaveIndex(account, "INBOX"); e != nil {
			return e
		}
	}
	for i := range folders {
		if folders[i].Path == "INBOX" {
			folders[i].Total = uint32(len(idx.Messages))
			for _, m := range idx.Messages {
				if !m.Flags.Seen {
					folders[i].Unread++
				}
			}
		}
	}
	if e = s.Store.SetFolders(account, folders); e != nil {
		return e
	}
	return s.secrets.Set(account, "demo")
}

// demoHTML 使用正常段落间距，避免双换行被渲染成过大的空白。
func demoHTML(text string) string {
	paragraphs := strings.Split(text, "\n\n")
	var out strings.Builder
	for _, p := range paragraphs {
		out.WriteString("<p style=\"margin:0 0 10px;line-height:1.6\">")
		out.WriteString(strings.ReplaceAll(html.EscapeString(p), "\n", "<br>"))
		out.WriteString("</p>")
	}
	return out.String()
}
