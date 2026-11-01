package protocol

import (
	"bufio"
	"context"
	"fmt"
	"github.com/emersion/go-imap/backend/memory"
	"github.com/emersion/go-imap/server"
	"github.com/openfabrica/mailfabrica/internal/mail"
	"io"
	"net"
	"strings"
	"testing"
	"time"
)

// testIMAP 启动真正 TCP IMAP 服务，所有账户与原文都只在内存。
func testIMAP(t *testing.T) (mail.Account, Network) {
	t.Helper()
	listener, e := net.Listen("tcp", "127.0.0.1:0")
	if e != nil {
		t.Fatal(e)
	}
	srv := server.New(memory.New())
	srv.AllowInsecureAuth = true
	go func() { _ = srv.Serve(listener) }()
	t.Cleanup(func() { _ = srv.Close(); _ = listener.Close() })
	a := mail.Account{ID: "test", Login: "username", Email: "username@example.com", IMAP: mail.Server{Host: "127.0.0.1", Port: uint16(listener.Addr().(*net.TCPAddr).Port), Security: "none"}}
	return a, Network{}
}

// TestNetworkIMAPRoundTrip 覆盖认证、UID、PEEK、标记、中文目录及 APPEND。
func TestNetworkIMAPRoundTrip(t *testing.T) {
	a, remote := testIMAP(t)
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	if e := remote.Verify(ctx, a, "password"); e != nil {
		t.Fatal(e)
	}
	if e := remote.Verify(ctx, a, "wrong"); e == nil {
		t.Fatal("accepted wrong password")
	}
	folders, e := remote.Sync(ctx, a, "password", 500, 10)
	if e != nil {
		t.Fatal(e)
	}
	if len(folders) != 1 || len(folders[0].Messages) != 1 || folders[0].Messages[0].Key.UID != 6 {
		t.Fatalf("sync: %+v", folders)
	}
	key := mail.Key{Account: a.ID, Folder: "INBOX", UID: 6}
	raw, e := remote.Fetch(ctx, a, "password", key)
	if e != nil || !strings.Contains(string(raw), "Hi there") {
		t.Fatalf("fetch %q %v", raw, e)
	}
	if e = remote.Change(ctx, a, "password", key, "flagged", true); e != nil {
		t.Fatal(e)
	}
	// 批量标记走同一个连接内的 UID STORE
	if e = remote.SetFlags(ctx, a, "password", "INBOX", []uint32{key.UID}, "seen", true); e != nil {
		t.Fatal(e)
	}
	if e = remote.Create(ctx, a, "password", "工作/草稿"); e != nil {
		t.Fatal(e)
	}
	out := mail.Outgoing{From: mail.Address{Email: a.Email}, To: []mail.Address{{Email: "b@example.com"}}, Subject: "中文草稿", Text: "保存正文"}
	built, e := Build(out)
	if e != nil {
		t.Fatal(e)
	}
	if e = remote.Append(ctx, a, "password", "工作/草稿", built, []string{"\\Draft"}); e != nil {
		t.Fatal(e)
	}
	folders, e = remote.Sync(ctx, a, "password", 500, 10)
	if e != nil {
		t.Fatal(e)
	}
	found, flagged, seen := false, false, false
	for _, f := range folders {
		if f.Folder.Path == "工作/草稿" && len(f.Messages) == 1 && f.Messages[0].Subject == "中文草稿" {
			found = true
		}
		if f.Folder.Path == "INBOX" {
			flagged = f.Messages[0].Flags.Flagged
			seen = f.Messages[0].Flags.Seen
		}
	}
	if !found || !flagged || !seen {
		t.Fatalf("folder/flag missing: %+v", folders)
	}
}

// TestIMAPCancellation 验证服务器不回 greeting 时上下文可以中止连接。
func TestIMAPCancellation(t *testing.T) {
	listener, e := net.Listen("tcp", "127.0.0.1:0")
	if e != nil {
		t.Fatal(e)
	}
	defer listener.Close()
	done := make(chan struct{})
	go func() {
		conn, e := listener.Accept()
		if e == nil {
			defer conn.Close()
			_, _ = io.Copy(io.Discard, conn)
		}
		close(done)
	}()
	ctx, cancel := context.WithTimeout(context.Background(), 50*time.Millisecond)
	defer cancel()
	a := mail.Account{Login: "x", IMAP: mail.Server{Host: "127.0.0.1", Port: uint16(listener.Addr().(*net.TCPAddr).Port), Security: "none"}}
	if e = (Network{}).Verify(ctx, a, "p"); e == nil {
		t.Fatal("expected canceled connection")
	}
	select {
	case <-done:
	case <-time.After(time.Second):
		t.Fatal("connection leaked")
	}
}

// TestSMTPAcceptedDespiteQuitFailure 确认 DATA 成功后结束会话断线不会诱发重复发送。
func TestSMTPAcceptedDespiteQuitFailure(t *testing.T) {
	listener, e := net.Listen("tcp", "127.0.0.1:0")
	if e != nil {
		t.Fatal(e)
	}
	defer listener.Close()
	captured := make(chan string, 1)
	go func() {
		conn, e := listener.Accept()
		if e != nil {
			return
		}
		defer conn.Close()
		reader := bufio.NewReader(conn)
		_, _ = fmt.Fprint(conn, "220 local ESMTP\r\n")
		for {
			line, e := reader.ReadString('\n')
			if e != nil {
				return
			}
			switch {
			case strings.HasPrefix(line, "EHLO"):
				_, _ = fmt.Fprint(conn, "250-local\r\n250 8BITMIME\r\n")
			case strings.HasPrefix(line, "MAIL"), strings.HasPrefix(line, "RCPT"):
				_, _ = fmt.Fprint(conn, "250 OK\r\n")
			case strings.HasPrefix(line, "DATA"):
				_, _ = fmt.Fprint(conn, "354 send\r\n")
				var data strings.Builder
				for {
					line, e = reader.ReadString('\n')
					if e != nil {
						return
					}
					if line == ".\r\n" {
						break
					}
					data.WriteString(line)
				}
				captured <- data.String()
				_, _ = fmt.Fprint(conn, "250 accepted\r\n")
			case strings.HasPrefix(line, "QUIT"):
				return
			default:
				_, _ = fmt.Fprint(conn, "500 unknown\r\n")
			}
		}
	}()
	a := mail.Account{Email: "a@example.com", SMTP: mail.Server{Host: "127.0.0.1", Port: uint16(listener.Addr().(*net.TCPAddr).Port), Security: "none"}}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	_, e = (Network{}).Send(ctx, a, "", mail.Outgoing{From: mail.Address{Email: a.Email}, To: []mail.Address{{Email: "b@example.com"}}, BCC: []mail.Address{{Email: "hidden@example.com"}}, Subject: "真实 SMTP 测试", Text: "hello"})
	if e != nil {
		t.Fatal(e)
	}
	raw := <-captured
	if strings.Contains(raw, "hidden@example.com") {
		t.Fatal("BCC leaked")
	}
	b, e := Parse([]byte(raw))
	if e != nil || b.Text != "hello" {
		t.Fatalf("SMTP content: %+v %v", b, e)
	}
}

// TestIncrementalSync 确认再次同步只刷新已知邮件标记，不重新下载其原文。
func TestIncrementalSync(t *testing.T) {
	a, remote := testIMAP(t)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	first, e := remote.Sync(ctx, a, "password", 100, 10)
	if e != nil {
		t.Fatal(e)
	}
	known := map[string]KnownFolder{}
	for _, folder := range first {
		messages := map[uint32]mail.Message{}
		for _, m := range folder.Messages {
			messages[m.Key.UID] = m
		}
		known[folder.Folder.Path] = KnownFolder{UIDValidity: folder.Folder.UIDValidity, Messages: messages}
	}
	second, e := remote.Sync(ctx, a, "password", 100, 10, known)
	if e != nil {
		t.Fatal(e)
	}
	if len(second) != 1 || len(second[0].Messages) != 1 || len(second[0].Raw) != 0 {
		t.Fatalf("unexpected incremental sync: %+v", second)
	}
}
