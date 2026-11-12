package engine

import (
	"context"
	"fmt"
	"github.com/openfabrica/mailfabrica/internal/mail"
	"github.com/openfabrica/mailfabrica/internal/protocol"
	"testing"
)

// TestFolderMessagesScope 验证搜索/选择不影响文件夹批量范围及远端同步后的结果。
func TestFolderMessagesScope(t *testing.T) {
	for _, tc := range []struct{ command, dest string }{
		{"folder.markAllRead", "INBOX"},
		{"folder.deleteMessages", "Trash"},
		{"folder.junk", "Junk"},
	} {
		t.Run(tc.command, func(t *testing.T) {
			s := testSession(t)
			const account = "demo-mailfabrica-dev"
			executeTest(t, s, "message.move", Params{"keys": []any{account + "/1/INBOX"}, "folder": "Archive"})
			executeTest(t, s, "view.scope", Params{"account": account, "folder": "Archive"})
			executeTest(t, s, "view.search", Params{"query": "不存在的搜索"})
			executeTest(t, s, tc.command, Params{"account": account, "folder": "INBOX"})
			s.mu.Lock()
			defer s.mu.Unlock()
			inbox, err := s.Store.Index(account, "INBOX")
			if err != nil {
				t.Fatal(err)
			}
			dest, err := s.Store.Index(account, tc.dest)
			if err != nil {
				t.Fatal(err)
			}
			if len(dest.Messages) != 10 {
				t.Fatalf("destination has %d messages", len(dest.Messages))
			}
			if tc.command == "folder.markAllRead" {
				for _, m := range inbox.Messages {
					if !m.Flags.Seen {
						t.Fatal("unread message remained")
					}
				}
			} else if len(inbox.Messages) != 0 {
				t.Fatal("source not empty")
			}
			archive, err := s.Store.Index(account, "Archive")
			if err != nil || len(archive.Messages) != 1 || archive.Messages[0].Flags.Seen {
				t.Fatalf("unrelated folder changed: %v", err)
			}
			if len(s.status.Errors) > 0 {
				t.Fatal(s.status.Errors)
			}
		})
	}
}

// TestFolderMessagesValidation 验证非法范围、移入自身、空目录和已删除中的永久删除。
func TestFolderMessagesValidation(t *testing.T) {
	s := testSession(t)
	const account = "demo-mailfabrica-dev"
	for _, command := range []string{"folder.deleteMessages", "folder.junk"} {
		for _, p := range []Params{{}, {"account": account}, {"account": "missing", "folder": "INBOX"}, {"account": account, "folder": "missing"}} {
			if _, err := s.Execute(command, marshalParams(p)); err == nil {
				t.Fatalf("accepted %s %v", command, p)
			}
		}
		executeTest(t, s, command, Params{"account": account, "folder": "Archive"})
	}
	if _, err := s.Execute("folder.junk", marshalParams(Params{"account": account, "folder": "Junk"})); err == nil {
		t.Fatal("accepted self move")
	}
	executeTest(t, s, "folder.deleteMessages", Params{"account": account, "folder": "INBOX"})
	executeTest(t, s, "folder.deleteMessages", Params{"account": account, "folder": "Trash"})
	s.mu.Lock()
	defer s.mu.Unlock()
	idx, err := s.Store.Index(account, "Trash")
	if err != nil || len(idx.Messages) != 0 {
		t.Fatalf("trash not empty: %v", err)
	}
	if len(s.status.Errors) > 0 {
		t.Fatal(s.status.Errors)
	}
}

// folderFailureRemote 模拟服务器已处理部分邮件后断开连接。
type folderFailureRemote struct{ protocol.Remote }

// ChangeFolder 处理一封后返回失败，检验引擎是否保留错误并同步部分结果。
func (r folderFailureRemote) ChangeFolder(ctx context.Context, a mail.Account, p, folder, op, dest string) error {
	if err := r.Remote.Change(ctx, a, p, mail.Key{Account: a.ID, Folder: folder, UID: 1}, "move", dest); err != nil {
		return err
	}
	return fmt.Errorf("模拟连接中断")
}

// TestFolderMessagesUncachedAndFailure 验证未缓存的旧邮件也被处理，部分失败后以服务器为准。
func TestFolderMessagesUncachedAndFailure(t *testing.T) {
	for _, command := range []string{"folder.markAllRead", "folder.junk", "folder.deleteMessages"} {
		t.Run(command, func(t *testing.T) {
			s := testSession(t)
			const account = "demo-mailfabrica-dev"
			remote := s.remote.(*protocol.Memory)
			remote.Put(account, "INBOX", 100, []byte("From: a@example.com\r\nSubject: uncached\r\n\r\nbody"), mail.Flags{})
			executeTest(t, s, command, Params{"account": account, "folder": "INBOX"})
			s.mu.Lock()
			defer s.mu.Unlock()
			folder := "INBOX"
			if command == "folder.junk" {
				folder = "Junk"
			}
			if command == "folder.deleteMessages" {
				folder = "Trash"
			}
			idx, err := s.Store.Index(account, folder)
			if err != nil || len(idx.Messages) != 12 {
				t.Fatalf("uncached message missing: %v", err)
			}
			if command == "folder.markAllRead" {
				for _, m := range idx.Messages {
					if !m.Flags.Seen {
						t.Fatal("old mail not marked")
					}
				}
			}
		})
	}
	s := testSession(t)
	s.remote = folderFailureRemote{s.remote}
	executeTest(t, s, "folder.junk", Params{"account": "demo-mailfabrica-dev", "folder": "INBOX"})
	s.mu.Lock()
	defer s.mu.Unlock()
	inbox, err := s.Store.Index("demo-mailfabrica-dev", "INBOX")
	if err != nil || len(inbox.Messages) != 10 {
		t.Fatalf("partial result not synchronized: %v", err)
	}
	if len(s.status.Errors) == 0 {
		t.Fatal("failure was hidden")
	}
}
