package protocol

import (
	"bytes"
	"context"
	"crypto/tls"
	"fmt"
	"io"
	"net"
	"net/smtp"
	"strconv"
	"strings"
	"time"

	"github.com/emersion/go-imap"
	"github.com/emersion/go-imap/client"
	"github.com/openfabrica/mailfabrica/internal/mail"
)

// FolderResult 是后台同步的纯数据结果，由引擎负责持久化。
type FolderResult struct {
	Folder   mail.Folder
	Messages []mail.Message
	UIDs     []uint32
	Raw      map[uint32][]byte
}

// KnownFolder 是引擎传入的只读同步基准，不包含存储引用。
type KnownFolder struct {
	UIDValidity uint32
	Messages    map[uint32]mail.Message
}

// Remote 使真实协议和内存演示使用相同的业务入口。
type Remote interface {
	Verify(context.Context, mail.Account, string) error
	Sync(context.Context, mail.Account, string, uint32, uint32, ...map[string]KnownFolder) ([]FolderResult, error)
	Fetch(context.Context, mail.Account, string, mail.Key) ([]byte, error)
	Change(context.Context, mail.Account, string, mail.Key, string, any) error
	Create(context.Context, mail.Account, string, string) error
	Append(context.Context, mail.Account, string, string, []byte, []string) error
	Send(context.Context, mail.Account, string, mail.Outgoing) ([]byte, error)
}

// Network 为每个串行账户任务建立有界连接。
type Network struct{}

// command 封装 IMAP 扩展命令，不拼接未转义字符串。
type command struct {
	Name string
	Args []interface{}
}

// Command 返回 go-imap 的结构化命令。
func (c command) Command() *imap.Command { return &imap.Command{Name: c.Name, Arguments: c.Args} }

// dialIMAP 设置 TLS、超时、取消和网易 ID，然后认证。
func dialIMAP(ctx context.Context, a mail.Account, password string) (*client.Client, func(), error) {
	if e := validateServer(a.IMAP); e != nil {
		return nil, nil, e
	}
	addr := net.JoinHostPort(a.IMAP.Host, strconv.Itoa(int(a.IMAP.Port)))
	d := net.Dialer{Timeout: 20 * time.Second}
	conn, e := d.DialContext(ctx, "tcp", addr)
	if e != nil {
		return nil, nil, e
	}
	_ = conn.SetDeadline(time.Now().Add(2 * time.Minute))
	stop := context.AfterFunc(ctx, func() { _ = conn.Close() })
	cleanup := func() { stop(); _ = conn.Close() }
	if a.IMAP.Security == "tls" {
		tc := tls.Client(conn, &tls.Config{ServerName: a.IMAP.Host, MinVersion: tls.VersionTLS12})
		if e = tc.HandshakeContext(ctx); e != nil {
			cleanup()
			return nil, nil, e
		}
		conn = tc
	}
	c, e := client.New(conn)
	if e != nil {
		cleanup()
		return nil, nil, e
	}
	c.Timeout = 45 * time.Second
	if a.IMAP.Security == "starttls" {
		if e = c.StartTLS(&tls.Config{ServerName: a.IMAP.Host, MinVersion: tls.VersionTLS12}); e != nil {
			cleanup()
			return nil, nil, e
		}
	}
	if ok, _ := c.Support("ID"); ok {
		status, err := c.Execute(command{Name: "ID", Args: []interface{}{[]interface{}{"name", "MailFabrica", "version", "0.1.6", "vendor", "OpenFabrica"}}}, nil)
		if err != nil {
			cleanup()
			return nil, nil, err
		}
		if status != nil && status.Err() != nil {
			cleanup()
			return nil, nil, status.Err()
		}
	}
	if e = c.Login(a.Login, password); e != nil {
		cleanup()
		return nil, nil, fmt.Errorf("IMAP 登录失败: %w", e)
	}
	return c, cleanup, nil
}

// Verify 校验 IMAP 凭据，SMTP 在实际发送时认证。
func (Network) Verify(ctx context.Context, a mail.Account, p string) error {
	_, close, e := dialIMAP(ctx, a, p)
	if e == nil {
		close()
	}
	return e
}

// Sync 同步有限数量的最新邮件头，同时取得完整 UID 集合用于删除检测。
func (Network) Sync(ctx context.Context, a mail.Account, password string, limit, prefetch uint32, known ...map[string]KnownFolder) ([]FolderResult, error) {
	c, close, e := dialIMAP(ctx, a, password)
	if e != nil {
		return nil, e
	}
	defer close()
	ch := make(chan *imap.MailboxInfo, 32)
	done := make(chan error, 1)
	go func() { done <- c.List("", "*", ch) }()
	list := []*imap.MailboxInfo{}
	for f := range ch {
		if len(list) >= 2000 {
			c.Terminate()
			return nil, fmt.Errorf("文件夹超过 2000 个")
		}
		list = append(list, f)
	}
	if e = <-done; e != nil {
		return nil, e
	}
	out := []FolderResult{}
	for _, f := range list {
		if e = ctx.Err(); e != nil {
			return nil, e
		}
		selectable := true
		for _, attr := range f.Attributes {
			if strings.EqualFold(attr, imap.NoSelectAttr) {
				selectable = false
			}
		}
		delim := f.Delimiter
		fr := FolderResult{Folder: mail.Folder{Account: a.ID, Path: f.Name, Delimiter: &delim, Role: mail.FolderRole(f.Name, f.Attributes), Selectable: selectable}, Messages: []mail.Message{}, UIDs: []uint32{}, Raw: map[uint32][]byte{}}
		if !selectable {
			out = append(out, fr)
			continue
		}
		status, err := c.Select(f.Name, false)
		if err != nil {
			return nil, err
		}
		fr.Folder.UIDValidity = status.UidValidity
		fr.Folder.Total = status.Messages
		ids, err := c.UidSearch(imap.NewSearchCriteria())
		if err != nil {
			return nil, err
		}
		fr.UIDs = ids
		if len(ids) > 2000000 {
			return nil, fmt.Errorf("文件夹邮件过多")
		}
		start := len(ids) - int(limit)
		if start < 0 {
			start = 0
		}
		if len(ids) == 0 {
			out = append(out, fr)
			continue
		}
		newIDs := append([]uint32{}, ids[start:]...)
		prior := KnownFolder{}
		if len(known) > 0 {
			prior = known[0][f.Name]
		}
		if prior.UIDValidity == status.UidValidity && len(prior.Messages) > 0 {
			lastUID := uint32(0)
			for uid := range prior.Messages {
				if uid > lastUID {
					lastUID = uid
				}
			}
			newIDs = []uint32{}
			for _, uid := range ids {
				if uid > lastUID {
					newIDs = append(newIDs, uid)
				}
			}
			// 已知邮件仅刷新标记；新邮件才获取 Envelope 和正文，避免反复下载。
			knownIDs := []uint32{}
			for _, uid := range ids {
				if _, ok := prior.Messages[uid]; ok {
					knownIDs = append(knownIDs, uid)
				}
			}
			for offset := 0; offset < len(knownIDs); offset += 2000 {
				end := offset + 2000
				if end > len(knownIDs) {
					end = len(knownIDs)
				}
				flagsSet := new(imap.SeqSet)
				flagsSet.AddNum(knownIDs[offset:end]...)
				flagMessages := make(chan *imap.Message, 32)
				go func() { done <- c.UidFetch(flagsSet, []imap.FetchItem{imap.FetchUid, imap.FetchFlags}, flagMessages) }()
				for m := range flagMessages {
					meta, ok := prior.Messages[m.Uid]
					if ok {
						meta.Flags = flagsFromIMAP(m.Flags)
						fr.Messages = append(fr.Messages, meta)
					}
				}
				if e = <-done; e != nil {
					return nil, e
				}
			}
		}
		if len(newIDs) > int(limit) && limit > 0 {
			newIDs = newIDs[len(newIDs)-int(limit):]
		}
		if len(newIDs) == 0 {
			out = append(out, fr)
			continue
		}
		set := new(imap.SeqSet)
		set.AddNum(newIDs...)
		messages := make(chan *imap.Message, 32)
		go func() {
			done <- c.UidFetch(set, []imap.FetchItem{imap.FetchUid, imap.FetchEnvelope, imap.FetchFlags, imap.FetchRFC822Size}, messages)
		}()
		for m := range messages {
			if m.Envelope == nil {
				continue
			}
			env := m.Envelope
			convert := func(a []*imap.Address) []mail.Address {
				out := []mail.Address{}
				for _, v := range a {
					out = append(out, mail.Address{Name: DecodeHeader(v.PersonalName), Email: v.Address()})
				}
				return out
			}
			from := mail.Address{}
			if v := convert(env.From); len(v) > 0 {
				from = v[0]
			}
			meta := mail.Message{Key: mail.Key{Account: a.ID, Folder: f.Name, UID: m.Uid}, Subject: DecodeHeader(env.Subject), MessageID: env.MessageId, From: from, To: convert(env.To), CC: convert(env.Cc), Date: env.Date.Unix(), Size: m.Size, References: []string{}}
			for _, v := range m.Flags {
				switch strings.ToLower(v) {
				case "\\seen":
					meta.Flags.Seen = true
				case "\\flagged":
					meta.Flags.Flagged = true
				case "\\answered":
					meta.Flags.Answered = true
				case "\\draft":
					meta.Flags.Draft = true
				case "\\deleted":
					meta.Flags.Deleted = true
				}
			}
			if !meta.Flags.Seen {
				fr.Folder.Unread++
			}
			fr.Messages = append(fr.Messages, meta)
		}
		if e = <-done; e != nil {
			return nil, e
		}
		n := int(prefetch)
		if n > len(newIDs) {
			n = len(newIDs)
		}
		total := 0
		for _, uid := range newIDs[len(newIDs)-n:] {
			raw, err := fetch(c, uid)
			if err != nil {
				return nil, err
			}
			total += len(raw)
			if total > 200<<20 {
				return nil, fmt.Errorf("单文件夹预取超过 200 MiB")
			}
			fr.Raw[uid] = raw
		}
		out = append(out, fr)
	}
	return out, nil
}

// fetch 使用 PEEK 下载原文，避免阅读前改变已读标记。
func fetch(c *client.Client, uid uint32) ([]byte, error) {
	set := new(imap.SeqSet)
	set.AddNum(uid)
	section := &imap.BodySectionName{Peek: true}
	ch := make(chan *imap.Message, 1)
	done := make(chan error, 1)
	go func() { done <- c.UidFetch(set, []imap.FetchItem{section.FetchItem()}, ch) }()
	var raw []byte
	var readErr error
	for m := range ch {
		if r := m.GetBody(section); r != nil {
			raw, readErr = io.ReadAll(io.LimitReader(r, MaxMessage+1))
			if len(raw) > MaxMessage {
				readErr = fmt.Errorf("邮件超过 100 MiB")
				c.Terminate()
			}
		}
	}
	if e := <-done; e != nil {
		return nil, e
	}
	if readErr != nil {
		return nil, readErr
	}
	if raw == nil {
		return nil, fmt.Errorf("远端邮件不存在")
	}
	return raw, nil
}

// Fetch 从指定文件夹获取邮件原文。
func (Network) Fetch(ctx context.Context, a mail.Account, p string, k mail.Key) ([]byte, error) {
	c, close, e := dialIMAP(ctx, a, p)
	if e != nil {
		return nil, e
	}
	defer close()
	if _, e = c.Select(k.Folder, false); e != nil {
		return nil, e
	}
	return fetch(c, k.UID)
}

// Change 执行标记、移动或永久删除；不使用会删除其他邮件的全局 EXPUNGE 回退。
func (Network) Change(ctx context.Context, a mail.Account, p string, k mail.Key, op string, value any) error {
	c, close, e := dialIMAP(ctx, a, p)
	if e != nil {
		return e
	}
	defer close()
	if _, e = c.Select(k.Folder, false); e != nil {
		return e
	}
	set := new(imap.SeqSet)
	set.AddNum(k.UID)
	if op == "move" {
		dest, ok := value.(string)
		if !ok {
			return fmt.Errorf("移动目录无效")
		}
		move, _ := c.Support("MOVE")
		plus, _ := c.Support("UIDPLUS")
		if !move && !plus {
			return fmt.Errorf("服务器不支持安全移动（需要 MOVE 或 UIDPLUS）")
		}
		if move {
			return c.UidMove(set, dest)
		}
		if e = c.UidCopy(set, dest); e != nil {
			return e
		}
		if e = c.UidStore(set, imap.FormatFlagsOp(imap.AddFlags, true), []interface{}{imap.DeletedFlag}, nil); e != nil {
			return e
		}
		return uidExpunge(c, set)
	}
	if op == "delete" {
		if ok, _ := c.Support("UIDPLUS"); !ok {
			return fmt.Errorf("服务器不支持指定 UID 永久删除")
		}
		if e = c.UidStore(set, imap.FormatFlagsOp(imap.AddFlags, true), []interface{}{imap.DeletedFlag}, nil); e != nil {
			return e
		}
		return uidExpunge(c, set)
	}
	v, ok := value.(bool)
	if !ok {
		return fmt.Errorf("标记值无效")
	}
	flag := ""
	switch op {
	case "seen":
		flag = imap.SeenFlag
	case "flagged":
		flag = imap.FlaggedFlag
	case "answered":
		flag = imap.AnsweredFlag
	default:
		return fmt.Errorf("未知标记")
	}
	var kind imap.FlagsOp = imap.RemoveFlags
	if v {
		kind = imap.AddFlags
	}
	return c.UidStore(set, imap.FormatFlagsOp(kind, true), []interface{}{flag}, nil)
}

// uidExpunge 将删除范围限定为当前请求的 UID。
func uidExpunge(c *client.Client, set *imap.SeqSet) error {
	s, e := c.Execute(command{Name: "UID EXPUNGE", Args: []interface{}{set}}, nil)
	if e != nil {
		return e
	}
	return s.Err()
}

// Create 创建远端文件夹。
func (Network) Create(ctx context.Context, a mail.Account, p, name string) error {
	c, close, e := dialIMAP(ctx, a, p)
	if e != nil {
		return e
	}
	defer close()
	return c.Create(name)
}

// Append 保存已发送邮件或草稿。
func (Network) Append(ctx context.Context, a mail.Account, p, folder string, raw []byte, flags []string) error {
	c, close, e := dialIMAP(ctx, a, p)
	if e != nil {
		return e
	}
	defer close()
	return c.Append(folder, flags, time.Now(), bytes.NewReader(raw))
}

// loginAuth 支持只提供 AUTH LOGIN 的邮件服务器。
type loginAuth struct{ user, password string }

// Start 开始 LOGIN 认证，TLS 限制由发送连接负责。
func (a loginAuth) Start(*smtp.ServerInfo) (string, []byte, error) { return "LOGIN", nil, nil }

// Next 按服务器挑战发送用户名或密码。
func (a loginAuth) Next(challenge []byte, more bool) ([]byte, error) {
	if !more {
		return nil, nil
	}
	s := strings.ToLower(string(challenge))
	if strings.Contains(s, "user") {
		return []byte(a.user), nil
	}
	if strings.Contains(s, "pass") {
		return []byte(a.password), nil
	}
	return nil, fmt.Errorf("未知 SMTP LOGIN 挑战")
}

// Send 经 TLS 或 STARTTLS 发送 MIME 并保留独立 BCC 信封。
func (Network) Send(ctx context.Context, a mail.Account, p string, o mail.Outgoing) ([]byte, error) {
	if err := validateServer(a.SMTP); err != nil {
		return nil, err
	}
	raw, e := Build(o)
	if e != nil {
		return nil, e
	}
	addr := net.JoinHostPort(a.SMTP.Host, strconv.Itoa(int(a.SMTP.Port)))
	d := net.Dialer{Timeout: 20 * time.Second}
	conn, e := d.DialContext(ctx, "tcp", addr)
	if e != nil {
		return nil, e
	}
	defer conn.Close()
	stop := context.AfterFunc(ctx, func() { _ = conn.Close() })
	defer stop()
	_ = conn.SetDeadline(time.Now().Add(2 * time.Minute))
	config := &tls.Config{ServerName: a.SMTP.Host, MinVersion: tls.VersionTLS12}
	if a.SMTP.Security == "tls" {
		tc := tls.Client(conn, config)
		if e = tc.HandshakeContext(ctx); e != nil {
			return nil, e
		}
		conn = tc
	}
	c, e := smtp.NewClient(conn, a.SMTP.Host)
	if e != nil {
		return nil, e
	}
	defer c.Close()
	if a.SMTP.Security == "starttls" {
		if e = c.StartTLS(config); e != nil {
			return nil, e
		}
	}

	if p != "" {
		_, mechanisms := c.Extension("AUTH")
		var auth smtp.Auth = smtp.PlainAuth("", a.Login, p, a.SMTP.Host)
		if !strings.Contains(strings.ToUpper(mechanisms), "PLAIN") && strings.Contains(strings.ToUpper(mechanisms), "LOGIN") {
			auth = loginAuth{a.Login, p}
		}
		if e = c.Auth(auth); e != nil {
			return nil, fmt.Errorf("SMTP 认证失败: %w", e)
		}
	}
	if e = c.Mail(o.From.Email); e != nil {
		return nil, e
	}
	seen := map[string]bool{}
	for _, v := range append(append(append([]mail.Address{}, o.To...), o.CC...), o.BCC...) {
		key := strings.ToLower(v.Email)
		if seen[key] {
			continue
		}
		seen[key] = true
		if e = c.Rcpt(v.Email); e != nil {
			return nil, e
		}
	}
	w, e := c.Data()
	if e != nil {
		return nil, e
	}
	if _, e = w.Write(raw); e != nil {
		return nil, e
	}
	if e = w.Close(); e != nil {
		return nil, e
	}
	// DATA 已被接受后，QUIT 失败不能将成功发送误报为可重试失败。
	_ = c.Quit()
	return raw, nil
}

// flagsFromIMAP 解析系统标记，忽略服务器自定义关键字。
func flagsFromIMAP(names []string) mail.Flags {
	f := mail.Flags{}
	for _, name := range names {
		switch strings.ToLower(name) {
		case "\\seen":
			f.Seen = true
		case "\\flagged":
			f.Flagged = true
		case "\\answered":
			f.Answered = true
		case "\\draft":
			f.Draft = true
		case "\\deleted":
			f.Deleted = true
		}
	}
	return f
}

// validateServer 在连接前检查持久化和外部参数中的端口、加密及明文范围。
func validateServer(server mail.Server) error {
	if strings.TrimSpace(server.Host) == "" || server.Port == 0 {
		return fmt.Errorf("服务器地址或端口无效")
	}
	switch server.Security {
	case "tls", "starttls":
		return nil
	case "none":
		ip := net.ParseIP(server.Host)
		if server.Host == "localhost" || ip != nil && ip.IsLoopback() {
			return nil
		}
		return fmt.Errorf("明文连接仅限本机测试")
	default:
		return fmt.Errorf("服务器加密方式无效")
	}
}
