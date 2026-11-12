package engine

import (
	"github.com/openfabrica/mailfabrica/internal/mail"
	"github.com/openfabrica/mailfabrica/internal/protocol"
	"testing"
)

// TestBulkSelectionDoesNotOpenOrRead 验证勾选整组、重复键及取消部分选择不会打开或标记邮件。
func TestBulkSelectionDoesNotOpenOrRead(t *testing.T) {
	s := testSession(t)
	keys := []any{"demo-mailfabrica-dev/1/INBOX", "demo-mailfabrica-dev/8/INBOX"}
	s.mu.Lock()
	before := s.counts.Unread
	s.mu.Unlock()
	executeTest(t, s, "view.select", Params{"keys": keys, "open": false})
	s.mu.Lock()
	if s.counts.Unread != before {
		s.mu.Unlock()
		t.Fatal("initial checkbox marked mail read")
	}
	if len(s.view.Selected) != 2 || s.view.Focused != nil || s.view.Group != nil {
		s.mu.Unlock()
		t.Fatal("selection opened a message")
	}
	s.mu.Unlock()
	executeTest(t, s, "view.select", Params{"keys": []any{keys[0], keys[0]}, "extend": true, "selected": true, "open": false})
	executeTest(t, s, "view.select", Params{"keys": keys, "extend": true, "selected": true, "open": false})
	s.mu.Lock()
	if len(s.view.Selected) != 2 || s.counts.Unread != before {
		s.mu.Unlock()
		t.Fatal("checkbox changed read state or duplicated keys")
	}
	s.mu.Unlock()
	executeTest(t, s, "view.select", Params{"keys": keys, "extend": true, "selected": false, "open": false})
	s.mu.Lock()
	defer s.mu.Unlock()
	if len(s.view.Selected) != 0 {
		t.Fatal("group deselection incomplete")
	}
}

// TestBulkActionsAffectOnlySelected 验证已读和删除仅作用于勾选邮件，删除后清理选择。
func TestBulkActionsAffectOnlySelected(t *testing.T) {
	for _, command := range []string{"message.markRead", "message.delete"} {
		t.Run(command, func(t *testing.T) {
			s := testSession(t)
			keys := []any{"demo-mailfabrica-dev/1/INBOX", "demo-mailfabrica-dev/2/INBOX"}
			executeTest(t, s, "view.select", Params{"keys": keys, "open": false})
			executeTest(t, s, command, Params{"keys": keys, "read": true})
			s.mu.Lock()
			defer s.mu.Unlock()
			idx, err := s.Store.Index("demo-mailfabrica-dev", "INBOX")
			if err != nil {
				t.Fatal(err)
			}
			if command == "message.delete" {
				if len(idx.Messages) != 9 || len(s.view.Selected) != 0 {
					t.Fatal("delete scope or selection invalid")
				}
			} else {
				for _, m := range idx.Messages {
					if m.Key.UID <= 2 && !m.Flags.Seen {
						t.Fatal("selected message not read")
					}
					if m.Key.UID == 3 && m.Flags.Seen {
						t.Fatal("unselected message read")
					}
				}
			}
			if len(s.status.Errors) > 0 {
				t.Fatal(s.status.Errors)
			}
		})
	}
}

// TestBulkSelectionValidationAndSearch 验证非法选择不污染现有状态，搜索后不保留隐藏邮件选择。
func TestBulkSelectionValidationAndSearch(t *testing.T) {
	s := testSession(t)
	executeTest(t, s, "view.select", Params{"key": "demo-mailfabrica-dev/1/INBOX", "open": false})
	for _, p := range []Params{{"keys": []any{"demo-mailfabrica-dev/999/INBOX"}, "open": false}, {"keys": []any{}, "open": "false"}, {"keys": []any{}, "selected": "true"}} {
		if _, err := s.Execute("view.select", marshalParams(p)); err == nil {
			t.Fatal("invalid selection accepted")
		}
	}
	s.mu.Lock()
	count := len(s.view.Selected)
	s.mu.Unlock()
	if count != 1 {
		t.Fatal("invalid input changed selection")
	}
	executeTest(t, s, "view.search", Params{"query": "不存在的主题"})
	s.mu.Lock()
	defer s.mu.Unlock()
	if len(s.view.Selected) != 0 {
		t.Fatal("hidden selection retained")
	}
}

// TestBulkDeleteAcrossAccounts 验证全部收件箱中的跨账户选择分别移到各自的已删除目录。
func TestBulkDeleteAcrossAccounts(t *testing.T) {
	s := testSession(t)
	executeTest(t, s, "account.add", Params{"email": "second@example.com", "password": "demo-only", "verify": true})
	account := mail.AccountID("second@example.com")
	for _, folder := range []string{"INBOX", "Trash"} {
		executeTest(t, s, "folder.create", Params{"account": account, "name": folder})
	}
	s.remote.(*protocol.Memory).Put(account, "INBOX", 1, []byte("Subject: second account\r\n\r\nbody"), mail.Flags{})
	executeTest(t, s, "mail.sync", Params{})
	keys := []any{"demo-mailfabrica-dev/1/INBOX", account + "/1/INBOX"}
	executeTest(t, s, "view.select", Params{"keys": keys, "open": false})
	executeTest(t, s, "message.delete", Params{"keys": keys})
	s.mu.Lock()
	defer s.mu.Unlock()
	for _, id := range []string{"demo-mailfabrica-dev", account} {
		idx, err := s.Store.Index(id, "Trash")
		if err != nil || len(idx.Messages) != 1 {
			t.Fatalf("wrong account destination: %s %v", id, err)
		}
	}
	if len(s.view.Selected) != 0 || len(s.status.Errors) > 0 {
		t.Fatal("selection or errors remain", s.status.Errors)
	}
}
