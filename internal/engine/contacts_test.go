package engine

import (
	"encoding/json"
	"testing"

	"github.com/openfabrica/mailfabrica/internal/mail"
)

// TestContactSuggestions 覆盖历史往来、联系人去重、姓名/邮箱匹配和查询上限，查询不写地址簿。
func TestContactSuggestions(t *testing.T) {
	s := testSession(t)
	executeTest(t, s, "contacts.add", Params{"name": "林晨（已保存）", "email": "sender0@example.com"})
	var list []mail.Contact
	if err := json.Unmarshal(executeTest(t, s, "contacts.suggest", Params{"query": "林", "limit": float64(8)}), &list); err != nil {
		t.Fatal(err)
	}
	if len(list) != 2 {
		t.Fatalf("expected saved plus history contacts, got %+v", list)
	}
	if list[0].Name != "林晨（已保存）" {
		t.Fatal("saved contact name overwritten")
	}
	if err := json.Unmarshal(executeTest(t, s, "contacts.suggest", Params{"query": "SENDER7@EXAMPLE.COM", "limit": float64(1)}), &list); err != nil {
		t.Fatal(err)
	}
	if len(list) != 1 || list[0].Email != "sender7@example.com" {
		t.Fatal(list)
	}
	for _, p := range []Params{{"query": true}, {"limit": float64(0)}, {"limit": float64(101)}} {
		if _, err := s.Execute("contacts.suggest", marshalParams(p)); err == nil {
			t.Fatal("accepted invalid suggestion parameters")
		}
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if len(s.Store.Contacts) != 1 {
		t.Fatal("suggestions modified address book")
	}
}
