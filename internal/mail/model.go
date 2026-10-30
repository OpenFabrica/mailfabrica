// Package mail 定义不依赖桌面框架的邮件领域类型与规则。
package mail

import (
	"fmt"
	"net/mail"
	"regexp"
	"strconv"
	"strings"
)

// Address 与旧版地址 JSON 保持一致。
type Address struct {
	Name  string `json:"name"`
	Email string `json:"email"`
}

// String 返回可供 MIME 使用的格式化地址。
func (a Address) String() string { return (&mail.Address{Name: a.Name, Address: a.Email}).String() }

// Display 返回界面使用的 Unicode 地址，避免显示 MIME 编码头。
func (a Address) Display() string {
	if a.Name == "" {
		return a.Email
	}
	return strconv.Quote(a.Name) + " <" + a.Email + ">"
}

// ParseAddresses 校验并解析用户输入的地址列表。
func ParseAddresses(s string) ([]Address, error) {
	out := []Address{}
	if strings.TrimSpace(s) == "" {
		return out, nil
	}
	list, err := mail.ParseAddressList(s)
	if err != nil {
		return nil, fmt.Errorf("邮箱地址无效: %w", err)
	}
	for _, a := range list {
		out = append(out, Address{Name: a.Name, Email: a.Address})
	}
	return out, nil
}

// Server 是 IMAP 或 SMTP 的连接配置。
type Server struct {
	Host     string `json:"host"`
	Port     uint16 `json:"port"`
	Security string `json:"security"`
}

// Account 不包含密码；凭据由系统适配层保存。
type Account struct {
	ID        string `json:"id"`
	Name      string `json:"name"`
	Email     string `json:"email"`
	Login     string `json:"login"`
	IMAP      Server `json:"imap"`
	SMTP      Server `json:"smtp"`
	Signature string `json:"signature"`
}

// Key 通过账户、文件夹和 UID 标识一封邮件。
type Key struct {
	Account string `json:"account"`
	Folder  string `json:"folder"`
	UID     uint32 `json:"uid"`
}

// String 保留账户/uid/文件夹格式，允许文件夹包含斜线。
func (k Key) String() string {
	return k.Account + "/" + strconv.FormatUint(uint64(k.UID), 10) + "/" + k.Folder
}

// ParseKey 解析外部命令使用的稳定消息键。
func ParseKey(s string) (Key, error) {
	p := strings.SplitN(s, "/", 3)
	if len(p) != 3 || !ValidAccountID(p[0]) || p[2] == "" {
		return Key{}, fmt.Errorf("邮件键无效")
	}
	uid, e := strconv.ParseUint(p[1], 10, 32)
	if e != nil || uid == 0 {
		return Key{}, fmt.Errorf("邮件 UID 无效")
	}
	return Key{Account: p[0], Folder: p[2], UID: uint32(uid)}, nil
}

// Flags 是持久化的 IMAP 系统标记。
type Flags struct {
	Seen     bool `json:"seen"`
	Flagged  bool `json:"flagged"`
	Answered bool `json:"answered"`
	Draft    bool `json:"draft"`
	Deleted  bool `json:"deleted"`
}

// Message 保存列表元数据，不包含附件内容。
type Message struct {
	Key            Key       `json:"key"`
	MessageID      string    `json:"messageId"`
	Subject        string    `json:"subject"`
	From           Address   `json:"from"`
	To             []Address `json:"to"`
	CC             []Address `json:"cc"`
	Date           int64     `json:"date"`
	Size           uint32    `json:"size"`
	Flags          Flags     `json:"flags"`
	HasAttachments bool      `json:"hasAttachments"`
	Preview        string    `json:"preview"`
	InReplyTo      *string   `json:"inReplyTo"`
	References     []string  `json:"references"`
}

// Folder 表示解码后的 IMAP 文件夹及缓存计数。
type Folder struct {
	Account     string  `json:"account"`
	Path        string  `json:"path"`
	Delimiter   *string `json:"delimiter"`
	Role        string  `json:"role"`
	Unread      uint32  `json:"unread"`
	Total       uint32  `json:"total"`
	UIDValidity uint32  `json:"uidvalidity"`
	Selectable  bool    `json:"selectable"`
}

// Attachment 描述 MIME 附件，内容只在后端内存中传递。
type Attachment struct {
	Index     int     `json:"index"`
	Name      string  `json:"name"`
	MIME      string  `json:"mime"`
	Size      int     `json:"size"`
	Inline    bool    `json:"inline"`
	ContentID *string `json:"contentId"`
	Data      []byte  `json:"-"`
}

// Body 是供阅读与回复使用的 MIME 视图。
type Body struct {
	MessageID   string       `json:"messageId"`
	Subject     string       `json:"subject"`
	From        []Address    `json:"from"`
	To          []Address    `json:"to"`
	CC          []Address    `json:"cc"`
	ReplyTo     []Address    `json:"replyTo"`
	Date        int64        `json:"date"`
	Text        string       `json:"text"`
	HTML        *string      `json:"html"`
	Attachments []Attachment `json:"attachments"`
	InReplyTo   *string      `json:"inReplyTo"`
	References  []string     `json:"references"`
}

// Contact 兼容旧版联系人记录。
type Contact struct {
	Name   string `json:"name"`
	Email  string `json:"email"`
	Count  uint32 `json:"count"`
	Last   int64  `json:"last"`
	Manual bool   `json:"manual"`
}

// Outgoing 是已经校验收件人、可交给传输层的邮件。
type Outgoing struct {
	// Date 为 Unix 秒；零值使用当前时间，演示/导入可保留原始日期。
	Date                int64
	From                Address
	To, CC, BCC         []Address
	Subject, Text, HTML string
	Attachments         []Attachment
	InReplyTo           *string
	References          []string
}

// ValidAccountID 阻止账户目录逃出数据根目录。
func ValidAccountID(s string) bool {
	if s == "" || strings.HasPrefix(s, ".") {
		return false
	}
	for _, r := range s {
		if !(r >= 'a' && r <= 'z' || r >= 'A' && r <= 'Z' || r >= '0' && r <= '9' || r == '-' || r == '_' || r == '.') {
			return false
		}
	}
	return true
}

// AccountID 使用旧版邮箱到账户 id 的转换规则。
func AccountID(s string) string {
	var b strings.Builder
	for _, r := range strings.TrimSpace(s) {
		if r >= 'A' && r <= 'Z' {
			r += 32
		}
		if r >= 'a' && r <= 'z' || r >= '0' && r <= '9' {
			b.WriteRune(r)
		} else {
			b.WriteByte('-')
		}
	}
	out := strings.Trim(b.String(), "-")
	if out == "" {
		return "account"
	}
	return out
}

// DetectServers 为常用邮箱生成默认连接配置。
func DetectServers(email string) (Server, Server) {
	parts := strings.Split(strings.ToLower(strings.TrimSpace(email)), "@")
	host := ""
	if len(parts) == 2 {
		host = parts[1]
	}
	base := host
	switch host {
	case "foxmail.com", "vip.qq.com":
		base = "qq.com"
	case "googlemail.com":
		base = "gmail.com"
	case "hotmail.com", "live.com", "msn.com":
		base = "outlook.com"
	case "me.com", "mac.com":
		base = "icloud.com"
	case "sina.cn":
		base = "sina.com"
	}
	i := Server{Host: "imap." + base, Port: 993, Security: "tls"}
	s := Server{Host: "smtp." + base, Port: 465, Security: "tls"}
	switch base {
	case "outlook.com":
		i.Host = "outlook.office365.com"
		s = Server{Host: "smtp.office365.com", Port: 587, Security: "starttls"}
	case "icloud.com":
		i.Host = "imap.mail.me.com"
		s = Server{Host: "smtp.mail.me.com", Port: 587, Security: "starttls"}
	case "zoho.com":
		s.Port = 587
		s.Security = "starttls"
	}
	return i, s
}

// FolderRole 从 SPECIAL-USE 属性或常见名称识别文件夹角色。
func FolderRole(path string, attrs []string) string {
	for _, a := range attrs {
		switch strings.ToLower(a) {
		case "\\sent":
			return "sent"
		case "\\drafts":
			return "drafts"
		case "\\trash":
			return "trash"
		case "\\junk":
			return "junk"
		case "\\archive", "\\all":
			return "archive"
		}
	}
	if strings.EqualFold(path, "INBOX") {
		return "inbox"
	}
	leaf := strings.ToLower(path)
	if i := strings.LastIndexAny(leaf, "/."); i >= 0 {
		leaf = leaf[i+1:]
	}
	switch leaf {
	case "drafts", "draft", "草稿箱", "草稿":
		return "drafts"
	case "sent", "sent messages", "sent items", "sent mail", "已发送", "已发送邮件":
		return "sent"
	case "trash", "deleted", "deleted items", "deleted messages", "bin", "已删除", "已删除邮件":
		return "trash"
	case "junk", "spam", "junk e-mail", "junk email", "垃圾邮件", "垃圾箱":
		return "junk"
	case "archive", "archives", "all mail", "归档":
		return "archive"
	}
	return "other"
}

var subjectPrefix = regexp.MustCompile(`(?i)^\s*(?:(?:re|fwd?|回复|转发|答复)(?:\[\d+\])?\s*[:：])\s*`)

// SubjectKey 去除连续回复/转发前缀并规范化空白。
func SubjectKey(s string) string {
	for {
		next := subjectPrefix.ReplaceAllString(s, "")
		if next == s {
			break
		}
		s = next
	}
	return strings.ToLower(strings.Join(strings.Fields(s), " "))
}

// IsInvoice 判断适用于发票视图的主题和发件人。
func IsInvoice(m Message) bool {
	s := strings.ToLower(m.Subject + " " + m.From.Name + " " + m.From.Email)
	return strings.Contains(s, "发票") || strings.Contains(s, "invoice")
}

// ServerCandidates 为企业邮箱和未知域名提供与旧版一致的有序候选。
func ServerCandidates(email, provider string) ([][2]Server, error) {
	i, s := DetectServers(email)
	tls := func(host string, port uint16) Server { return Server{Host: host, Port: port, Security: "tls"} }
	switch provider {
	case "exmail":
		return [][2]Server{{tls("imap.exmail.qq.com", 993), tls("smtp.exmail.qq.com", 465)}}, nil
	case "qiye163":
		return [][2]Server{{tls("imap.qiye.163.com", 993), tls("smtp.qiye.163.com", 465)}, {tls("imaphz.qiye.163.com", 993), tls("smtphz.qiye.163.com", 465)}, {tls("hwhzimap.qiye.163.com", 993), tls("hwhzsmtp.qiye.163.com", 465)}}, nil
	case "aliqiye":
		return [][2]Server{{tls("imap.qiye.aliyun.com", 993), tls("smtp.qiye.aliyun.com", 465)}}, nil
	case "", "qq", "netease", "gmail", "icloud", "other":
	default:
		return nil, fmt.Errorf("未知邮箱类型")
	}
	domain := ""
	if _, d, ok := strings.Cut(email, "@"); ok {
		domain = strings.ToLower(strings.TrimSpace(d))
	}
	known := map[string]bool{"qq.com": true, "foxmail.com": true, "vip.qq.com": true, "163.com": true, "126.com": true, "yeah.net": true, "sina.com": true, "sina.cn": true, "aliyun.com": true, "139.com": true, "gmail.com": true, "googlemail.com": true, "outlook.com": true, "hotmail.com": true, "live.com": true, "msn.com": true, "icloud.com": true, "me.com": true, "mac.com": true, "yahoo.com": true, "zoho.com": true}
	out := [][2]Server{{i, s}}
	if !known[domain] && domain != "" {
		out = append(out, [2]Server{tls("mail."+domain, 993), tls("mail."+domain, 465)}, [2]Server{tls(domain, 993), tls(domain, 465)})
	}
	return out, nil
}
