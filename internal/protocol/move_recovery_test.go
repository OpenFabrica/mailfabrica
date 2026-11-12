package protocol

import (
	"bufio"
	"context"
	"fmt"
	"net"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/emersion/go-imap"
	"github.com/openfabrica/mailfabrica/internal/mail"
)

// interruptedMoveServer 模拟 MOVE 部分成功但应答前断线，重连后源目录只剩未移动 UID。
func interruptedMoveServer(t *testing.T, supportMove bool) (mail.Account, func() []string) {
	t.Helper()
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = listener.Close() })
	var mu sync.Mutex
	remaining := map[uint32]bool{1: true, 2: true, 3: true}
	calls := []string{}
	disconnected := false
	go func() {
		for {
			conn, err := listener.Accept()
			if err != nil {
				return
			}
			go func() {
				defer conn.Close()
				_ = conn.SetDeadline(time.Now().Add(5 * time.Second))
				_, _ = fmt.Fprint(conn, "* OK recovery fixture\r\n")
				scanner := bufio.NewScanner(conn)
				for scanner.Scan() {
					fields := strings.Fields(scanner.Text())
					if len(fields) < 2 {
						return
					}
					tag, cmd := fields[0], strings.ToUpper(fields[1])
					switch cmd {
					case "CAPABILITY":
						caps := "IMAP4rev1 UIDPLUS"
						if supportMove {
							caps += " MOVE"
						}
						_, _ = fmt.Fprintf(conn, "* CAPABILITY %s\r\n%s OK capability\r\n", caps, tag)
					case "LOGIN":
						_, _ = fmt.Fprintf(conn, "%s OK login\r\n", tag)
					case "SELECT":
						_, _ = fmt.Fprintf(conn, "* FLAGS (\\Seen)\r\n* 3 EXISTS\r\n* OK [UIDVALIDITY 1]\r\n%s OK selected\r\n", tag)
					case "UID":
						if len(fields) < 4 {
							return
						}
						op := strings.ToUpper(fields[2])
						mu.Lock()
						switch op {
						case "MOVE", "COPY":
							calls = append(calls, op+" "+fields[3])
							if !disconnected {
								disconnected = true
								if op == "MOVE" {
									delete(remaining, 1)
									delete(remaining, 2)
								}
								mu.Unlock()
								return
							}
							set, err := imap.ParseSeqSet(fields[3])
							if err != nil {
								mu.Unlock()
								return
							}
							for uid := range remaining {
								if set.Contains(uid) {
									delete(remaining, uid)
								}
							}
							_, _ = fmt.Fprintf(conn, "%s OK moved\r\n", tag)
						case "SEARCH":
							_, _ = fmt.Fprint(conn, "* SEARCH")
							for uid := uint32(1); uid <= 3; uid++ {
								if remaining[uid] {
									_, _ = fmt.Fprintf(conn, " %d", uid)
								}
							}
							_, _ = fmt.Fprintf(conn, "\r\n%s OK search\r\n", tag)
						default:
							_, _ = fmt.Fprintf(conn, "%s BAD unexpected UID\r\n", tag)
						}
						mu.Unlock()
					default:
						_, _ = fmt.Fprintf(conn, "%s BAD unexpected\r\n", tag)
					}
				}
			}()
		}
	}()
	return mail.Account{ID: "test", Login: "test", IMAP: mail.Server{Host: "127.0.0.1", Port: uint16(listener.Addr().(*net.TCPAddr).Port), Security: "none"}}, func() []string { mu.Lock(); defer mu.Unlock(); return append([]string{}, calls...) }
}

// TestMoveManyRecoversInterruptedMOVE 验证断线后先核对源 UID，再只重试未移动邮件，避免重复操作已成功部分。
func TestMoveManyRecoversInterruptedMOVE(t *testing.T) {
	a, calls := interruptedMoveServer(t, true)
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	if err := (Network{}).MoveMany(ctx, a, "p", "INBOX", []uint32{1, 2, 3}, "Trash"); err != nil {
		t.Fatal(err)
	}
	got := calls()
	if len(got) != 2 || got[1] != "MOVE 3" {
		t.Fatalf("重试范围错误: %v", got)
	}
}

// TestMoveManyDoesNotReplayUncertainCOPY 验证 COPY 回退断线时不盲目重试，防止目标文件夹重复副本。
func TestMoveManyDoesNotReplayUncertainCOPY(t *testing.T) {
	a, calls := interruptedMoveServer(t, false)
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	if err := (Network{}).MoveMany(ctx, a, "p", "INBOX", []uint32{1, 2, 3}, "Trash"); err == nil {
		t.Fatal("未报告 COPY 结果不确定")
	}
	if got := calls(); len(got) != 1 {
		t.Fatalf("重复 COPY: %v", got)
	}
}
