package engine

import (
	"encoding/json"
	"github.com/openfabrica/mailfabrica/internal/mail"
	"testing"
)

// TestContactTimeline 验证跨账户、双向与抄送记录、去重、分页、排除目录及只读行为。
func TestContactTimeline(t *testing.T) {
	s := testSession(t)
	const account = "demo-mailfabrica-dev"
	const email = "friend@example.com"
	s.mu.Lock()
	second := s.Store.Accounts[0]
	second.ID = "second"
	second.Email = "second@example.com"
	s.Store.Accounts = append(s.Store.Accounts, second)
	if err := s.Store.SaveAccounts(); err != nil {
		s.mu.Unlock()
		t.Fatal(err)
	}
	if err := s.Store.SetFolders("second", []mail.Folder{{Account: "second", Path: "INBOX", Role: "inbox", Selectable: true}}); err != nil {
		s.mu.Unlock()
		t.Fatal(err)
	}
	put := func(a, folder string, uid uint32, id string, date int64, from string, to, cc []mail.Address, flags mail.Flags) {
		idx, err := s.Store.Index(a, folder)
		if err != nil {
			t.Fatal(err)
		}
		idx.Messages = append(idx.Messages, mail.Message{Key: mail.Key{Account: a, Folder: folder, UID: uid}, MessageID: id, Date: date, Subject: "往来测试", From: mail.Address{Email: from}, To: to, CC: cc, Flags: flags})
		if err := s.Store.SaveIndex(a, folder); err != nil {
			t.Fatal(err)
		}
	}
	own := []mail.Address{{Email: "demo@mailfabrica.dev"}}
	friend := []mail.Address{{Email: email}}
	put(account, "INBOX", 101, "<one>", 300, email, own, nil, mail.Flags{})
	put(account, "Archive", 201, "<one>", 300, email, own, nil, mail.Flags{})
	put(account, "Sent", 102, "<two>", 200, own[0].Email, friend, nil, mail.Flags{})
	put(account, "INBOX", 105, "<two>", 200, own[0].Email, friend, nil, mail.Flags{})
	put(account, "INBOX", 103, "<three>", 100, "other@example.com", own, friend, mail.Flags{})
	put("second", "INBOX", 301, "<four>", 400, email, []mail.Address{{Email: second.Email}}, nil, mail.Flags{})
	put(account, "Drafts", 110, "<draft>", 500, email, own, nil, mail.Flags{})
	put(account, "Junk", 111, "<junk>", 600, email, own, nil, mail.Flags{})
	put(account, "INBOX", 112, "<deleted>", 700, email, own, nil, mail.Flags{Deleted: true})
	revision := s.Store.Revision()
	s.mu.Unlock()
	var result ContactTimeline
	read := func(p Params) {
		t.Helper()
		if err := json.Unmarshal(executeTest(t, s, "contacts.timeline", p), &result); err != nil {
			t.Fatal(err)
		}
	}
	read(Params{"email": "FRIEND@example.com", "limit": float64(2)})
	if result.Total != 4 || result.Received != 2 || result.Sent != 1 || result.Related != 1 || !result.HasMore || len(result.Items) != 2 {
		t.Fatalf("timeline %+v", result)
	}
	if result.Items[0].Message.Key.Account != "second" || len(result.Items[1].Locations) != 2 || result.Items[1].Message.Key.Folder != "INBOX" {
		t.Fatal("wrong order or duplicate locations")
	}
	read(Params{"email": email, "offset": float64(2), "limit": float64(2)})
	if result.HasMore || len(result.Items) != 2 || result.Items[0].Direction != "outgoing" || result.Items[0].Message.Key.Folder != "Sent" || result.Items[1].Direction != "related" {
		t.Fatal("wrong second page")
	}
	read(Params{"email": email, "account": account})
	if result.Total != 3 {
		t.Fatal("account filter ignored")
	}
	read(Params{"email": email, "offset": float64(999)})
	if len(result.Items) != 0 || result.HasMore {
		t.Fatal("out of range page")
	}
	read(Params{"email": "nobody@example.com"})
	if result.Total != 0 || len(result.Items) != 0 {
		t.Fatal("empty contact has entries")
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	idx, err := s.Store.Index(account, "INBOX")
	if err != nil {
		t.Fatal(err)
	}
	for _, m := range idx.Messages {
		if m.Key.UID == 101 && m.Flags.Seen {
			t.Fatal("timeline marked message read")
		}
	}
	if s.Store.Revision() != revision || s.view.Focused != nil || len(s.Store.Contacts) != 0 {
		t.Fatal("timeline changed persisted or selected state")
	}
}

// TestContactTimelineInput 验证无效地址、账户与分页参数在命令入口被拒绝。
func TestContactTimelineInput(t *testing.T) {
	s := testSession(t)
	for _, p := range []Params{{}, {"email": "invalid"}, {"email": "a@example.com,b@example.com"}, {"email": "a@example.com", "account": true}, {"email": "a@example.com", "account": "unknown"}, {"email": "a@example.com", "limit": float64(0)}, {"email": "a@example.com", "limit": float64(101)}, {"email": "a@example.com", "offset": float64(-1)}, {"email": "a@example.com", "offset": float64(1.5)}, {"email": "a@example.com", "offset": "1"}} {
		if _, err := s.Execute("contacts.timeline", marshalParams(p)); err == nil {
			t.Fatalf("accepted %+v", p)
		}
	}
}
