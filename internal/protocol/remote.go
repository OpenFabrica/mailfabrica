package protocol

import (
	"bytes"
	"context"
	"crypto/tls"
	"errors"
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
	SetFlags(context.Context, mail.Account, string, string, []uint32, string, bool) error
	MoveMany(context.Context, mail.Account, string, string, []uint32, string) error
	ChangeFolder(context.Context, mail.Account, string, string, string, string) error
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

// flagName 将命令中的标记名映射为 IMAP 系统标记。
func flagName(op string) (string, error) {
	switch op {
	case "seen":
		return imap.SeenFlag, nil
	case "flagged":
		return imap.FlaggedFlag, nil
	case "answered":
		return imap.AnsweredFlag, nil
	}
	return "", fmt.Errorf("未知标记")
}

// flagsOp 返回静默增加或移除标记的 STORE 操作。
func flagsOp(add bool) imap.StoreItem {
	if add {
		return imap.FormatFlagsOp(imap.AddFlags, true)
	}
	return imap.FormatFlagsOp(imap.RemoveFlags, true)
}

// moveSet 优先使用 MOVE；不支持时以 UIDPLUS 的 COPY + 指定 UID 删除代替。
func moveSet(c *client.Client, set *imap.SeqSet, dest string) error {
	move, _ := c.Support("MOVE")
	plus, _ := c.Support("UIDPLUS")
	if !move && !plus {
		return fmt.Errorf("服务器不支持安全移动（需要 MOVE 或 UIDPLUS）")
	}
	if move {
		return c.UidMove(set, dest)
	}
	if e := c.UidCopy(set, dest); e != nil {
		return e
	}
	if e := c.UidStore(set, imap.FormatFlagsOp(imap.AddFlags, true), []interface{}{imap.DeletedFlag}, nil); e != nil {
		return e
	}
	return uidExpunge(c, set)
}

// MoveMany 分批移动，MOVE 断线后最多重连一次并只重试源目录仍存在的 UID。
func (Network) MoveMany(ctx context.Context, a mail.Account, p, folder string, uids []uint32, dest string) error {
	if len(uids) == 0 {
		return nil
	}
	c, closeConn, err := dialIMAP(ctx, a, p)
	if err != nil {
		return err
	}
	status, err := c.Select(folder, false)
	if err != nil {
		closeConn()
		return err
	}
	return moveManySelected(ctx, a, p, folder, uids, dest, c, closeConn, status.UidValidity)
}

// moveManySelected 复用已选中的连接，列表删除和文件夹删除使用相同的断线恢复规则。
func moveManySelected(ctx context.Context, a mail.Account, p, folder string, uids []uint32, dest string, c *client.Client, closeConn func(), validity uint32) error {
	defer func() {
		if closeConn != nil {
			closeConn()
		}
	}()
	var err error
	for start := 0; start < len(uids); start += 100 {
		if err = ctx.Err(); err != nil {
			return err
		}
		batch := uids[start:min(start+100, len(uids))]
		set := new(imap.SeqSet)
		set.AddNum(batch...)
		supportsMove, _ := c.Support("MOVE")
		err = moveSet(c, set, dest)
		if err == nil {
			continue
		}
		// COPY + STORE 回退无法确认复制阶段是否成功，不能重放；由引擎核对并显示剩余邮件。
		var networkError net.Error
		disconnected := c.State() == imap.LogoutState || err.Error() == "imap: connection closed" || errors.Is(err, io.EOF) || (errors.As(err, &networkError) && networkError.Timeout())
		if !supportsMove || ctx.Err() != nil || !disconnected {
			return err
		}
		closeConn()
		c, closeConn, err = dialIMAP(ctx, a, p)
		if err != nil {
			return err
		}
		status, err := c.Select(folder, false)
		if err != nil {
			return err
		}
		if status.UidValidity != validity {
			return fmt.Errorf("文件夹 UIDVALIDITY 已改变，请同步后重试")
		}
		criteria := imap.NewSearchCriteria()
		criteria.Uid = set
		remaining, searchErr := c.UidSearch(criteria)
		if searchErr != nil {
			return searchErr
		}
		if len(remaining) == 0 {
			continue
		}
		retry := new(imap.SeqSet)
		for _, uid := range remaining {
			if set.Contains(uid) {
				retry.AddNum(uid)
			}
		}
		if len(retry.Set) > 0 {
			// 只重试原子 MOVE，禁止重连后悄悄降级为可能重复复制的 COPY。
			if ok, _ := c.Support("MOVE"); !ok {
				return fmt.Errorf("重连后服务器不支持 MOVE，请同步后重试")
			}
			if err = c.UidMove(retry, dest); err != nil {
				return err
			}
		}
	}
	return nil
}

// SetFlags 在一个连接内对同一文件夹的多封邮件批量改标记，避免逐封登录。
func (Network) SetFlags(ctx context.Context, a mail.Account, p, folder string, uids []uint32, op string, value bool) error {
	flag, e := flagName(op)
	if e != nil {
		return e
	}
	c, close, e := dialIMAP(ctx, a, p)
	if e != nil {
		return e
	}
	defer close()
	if _, e = c.Select(folder, false); e != nil {
		return e
	}
	// 分段发送，避免 UID 不连续时命令行过长
	for start := 0; start < len(uids); start += 500 {
		end := min(start+500, len(uids))
		set := new(imap.SeqSet)
		set.AddNum(uids[start:end]...)
		if e = c.UidStore(set, flagsOp(value), []interface{}{flag}, nil); e != nil {
			return e
		}
	}
	return nil
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
		return moveSet(c, set, dest)
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
	flag, e := flagName(op)
	if e != nil {
		return e
	}
	return c.UidStore(set, flagsOp(v), []interface{}{flag}, nil)
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

// ChangeFolder 在同一连接内处理服务器上的整个文件夹，包括尚未缓存的邮件；分批失败由调用方重新同步。
func (Network) ChangeFolder(ctx context.Context, a mail.Account, p, folder, op, dest string) error {
	if op != "seen" && op != "move" && op != "delete" {
		return fmt.Errorf("未知文件夹操作")
	}
	if op == "move" && (dest == "" || dest == folder) {
		return fmt.Errorf("移动目录无效")
	}
	c, close, err := dialIMAP(ctx, a, p)
	if err != nil {
		return err
	}
	defer close()
	status, err := c.Select(folder, false)
	if err != nil {
		return err
	}
	ids, err := c.UidSearch(imap.NewSearchCriteria())
	if err != nil {
		return err
	}
	if len(ids) > 2000000 {
		return fmt.Errorf("文件夹邮件过多")
	}
	if op == "move" {
		return moveManySelected(ctx, a, p, folder, ids, dest, c, close, status.UidValidity)
	}
	if op == "delete" && len(ids) > 0 {
		if ok, _ := c.Support("UIDPLUS"); !ok {
			return fmt.Errorf("服务器不支持指定 UID 永久删除")
		}
	}
	for start := 0; start < len(ids); start += 500 {
		if err = ctx.Err(); err != nil {
			return err
		}
		set := new(imap.SeqSet)
		set.AddNum(ids[start:min(start+500, len(ids))]...)
		switch op {
		case "seen":
			err = c.UidStore(set, flagsOp(true), []interface{}{imap.SeenFlag}, nil)
		case "delete":
			err = c.UidStore(set, flagsOp(true), []interface{}{imap.DeletedFlag}, nil)
			if err == nil {
				err = uidExpunge(c, set)
			}
		}
		if err != nil {
			return err
		}
	}
	return nil
}
