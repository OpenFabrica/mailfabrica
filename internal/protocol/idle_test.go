package protocol

import (
	"bufio"
	"context"
	"errors"
	"fmt"
	"github.com/openfabrica/mailfabrica/internal/mail"
	"net"
	"strings"
	"testing"
	"time"
)

// idleServer 提供最小真实 TCP IMAP 交互，允许控制通知、断线及能力声明。
func idleServer(t *testing.T, support bool, trigger <-chan struct{}, disconnect bool) (mail.Account, <-chan struct{}) {
	t.Helper()
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = listener.Close() })
	ready := make(chan struct{})
	go func() {
		conn, err := listener.Accept()
		if err != nil {
			return
		}
		defer conn.Close()
		_ = conn.SetDeadline(time.Now().Add(5 * time.Second))
		_, _ = fmt.Fprint(conn, "* OK test ready\r\n")
		scanner := bufio.NewScanner(conn)
		for scanner.Scan() {
			fields := strings.Fields(scanner.Text())
			if len(fields) < 2 {
				return
			}
			tag, cmd := fields[0], strings.ToUpper(fields[1])
			switch cmd {
			case "CAPABILITY":
				cap := "IMAP4rev1"
				if support {
					cap += " IDLE"
				}
				_, _ = fmt.Fprintf(conn, "* CAPABILITY %s\r\n%s OK capability\r\n", cap, tag)
			case "LOGIN":
				_, _ = fmt.Fprintf(conn, "%s OK login\r\n", tag)
			case "EXAMINE", "SELECT":
				_, _ = fmt.Fprintf(conn, "* FLAGS (\\Seen)\r\n* 1 EXISTS\r\n* OK [UIDVALIDITY 1]\r\n%s OK selected\r\n", tag)
			case "IDLE":
				_, _ = fmt.Fprint(conn, "+ idling\r\n")
				close(ready)
				<-trigger
				if disconnect {
					return
				}
				_, _ = fmt.Fprint(conn, "* 2 EXISTS\r\n")
				for scanner.Scan() {
					if strings.EqualFold(scanner.Text(), "DONE") {
						_, _ = fmt.Fprintf(conn, "%s OK idle done\r\n", tag)
						return
					}
				}
				return
			default:
				_, _ = fmt.Fprintf(conn, "%s BAD unexpected\r\n", tag)
			}
		}
	}()
	return mail.Account{ID: "test", Login: "test", IMAP: mail.Server{Host: "127.0.0.1", Port: uint16(listener.Addr().(*net.TCPAddr).Port), Security: "none"}}, ready
}

// TestNetworkIdleNotificationAndCancel 验证服务端主动通知会被收到，取消能关闭长连接且不挂起。
func TestNetworkIdleNotificationAndCancel(t *testing.T) {
	trigger := make(chan struct{})
	a, ready := idleServer(t, true, trigger, false)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	notifications := make(chan struct{}, 16)
	done := make(chan error, 1)
	go func() { done <- (Network{}).WatchInbox(ctx, a, "p", "INBOX", func() { notifications <- struct{}{} }) }()
	select {
	case <-ready:
	case <-time.After(time.Second):
		t.Fatal("IDLE not entered")
	}
	// 首次建立连接主动核对一次，之后接受服务器 EXISTS 通知。
	select {
	case <-notifications:
	case <-time.After(time.Second):
		t.Fatal("initial check missing")
	}
	close(trigger)
	select {
	case <-notifications:
	case <-time.After(time.Second):
		t.Fatal("new-mail notification missing")
	}
	cancel()
	select {
	case <-done:
	case <-time.After(time.Second):
		t.Fatal("watcher did not stop")
	}
}

// TestNetworkIdleUnsupportedAndDisconnect 验证能力不支持时回退、连接中断时返回供上层重连。
func TestNetworkIdleUnsupportedAndDisconnect(t *testing.T) {
	trigger := make(chan struct{})
	close(trigger)
	a, _ := idleServer(t, false, trigger, false)
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	if err := (Network{}).WatchInbox(ctx, a, "p", "INBOX", func() {}); !errors.Is(err, ErrIdleUnsupported) {
		t.Fatalf("unsupported: %v", err)
	}
	a, _ = idleServer(t, true, trigger, true)
	if err := (Network{}).WatchInbox(ctx, a, "p", "INBOX", func() {}); err == nil {
		t.Fatal("disconnect lost")
	}
}
