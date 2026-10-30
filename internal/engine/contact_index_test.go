package engine

import (
	"encoding/json"
	"fmt"
	"mime"
	"testing"

	"github.com/openfabrica/mailfabrica/internal/mail"
)

// TestWeightedContactPrefixes 验证按往来时间而非字母排序，多检索词仍按邮箱去重。
func TestWeightedContactPrefixes(t *testing.T) {
	index := buildContactIndex(0, []mail.Contact{
		{Name: "Richard", Email: "richard@example.com", Last: 100, Count: 50},
		{Name: "Rachael", Email: "rachael@example.com", Last: 200},
		{Name: "Jane Smith", Email: "jane.smith@example.com", Last: 300},
		{Name: "张三", Email: "zhang@example.com", Last: 400},
	}, nil, nil)
	for _, test := range []struct {
		query string
		limit int
		want  []string
	}{
		{"r", 2, []string{"rachael@example.com", "richard@example.com"}},
		{"sm", 8, []string{"jane.smith@example.com"}},
		{"ja sm", 8, []string{"jane.smith@example.com"}},
		{"三", 8, []string{"zhang@example.com"}},
		{"EXAMPLE", 2, []string{"zhang@example.com", "jane.smith@example.com"}},
		{"@example.com", 2, []string{"zhang@example.com", "jane.smith@example.com"}},
		{"missing", 8, []string{}},
	} {
		got := index.search(test.query, test.limit)
		if len(got) != len(test.want) {
			t.Fatalf("query %q: %+v", test.query, got)
		}
		for i, want := range test.want {
			if got[i].Email != want {
				t.Fatalf("query %q: %+v", test.query, got)
			}
		}
	}
}

// TestContactIndexInvalidation 验证输入草稿不重建索引，地址簿变化后才失效。
func TestContactIndexInvalidation(t *testing.T) {
	s := testSession(t)
	executeTest(t, s, "contacts.suggest", Params{"query": "林"})
	previous := s.contactIndex
	var compose struct{ ID uint64 }
	if err := json.Unmarshal(executeTest(t, s, "compose.new", Params{}), &compose); err != nil {
		t.Fatal(err)
	}
	executeTest(t, s, "compose.set", Params{"id": float64(compose.ID), "to": "new@example.com"})
	executeTest(t, s, "contacts.suggest", Params{"query": "林"})
	if s.contactIndex != previous {
		t.Fatal("typing rebuilt contact index")
	}
	executeTest(t, s, "contacts.add", Params{"name": "Newest Person", "email": "newest@example.com"})
	executeTest(t, s, "contacts.suggest", Params{"query": "newest"})
	if s.contactIndex == previous {
		t.Fatal("changed contacts did not invalidate index")
	}
}

// TestLegacyEncodedMetadata 验证已有索引无需重新下载即可在列表、阅读头与联系人查询中正确显示。
func TestLegacyEncodedMetadata(t *testing.T) {
	s := testSession(t)
	encoded := mime.BEncoding.Encode("UTF-8", "张三")
	s.mu.Lock()
	index, err := s.Store.Index("demo-mailfabrica-dev", "INBOX")
	if err != nil {
		s.mu.Unlock()
		t.Fatal(err)
	}
	index.Messages[0].From.Name = encoded
	index.Messages[0].Subject = mime.BEncoding.Encode("UTF-8", "中文标题")
	index.Messages[0].To[0].Name = encoded
	key := index.Messages[0].Key.String()
	err = s.Store.SaveIndex("demo-mailfabrica-dev", "INBOX")
	s.mu.Unlock()
	if err != nil {
		t.Fatal(err)
	}
	var message struct{ Meta mail.Message }
	if err = json.Unmarshal(executeTest(t, s, "message.get", Params{"key": key}), &message); err != nil {
		t.Fatal(err)
	}
	if message.Meta.From.Name != "张三" || message.Meta.Subject != "中文标题" || message.Meta.To[0].Name != "张三" {
		t.Fatalf("metadata %+v", message.Meta)
	}
	var contacts []mail.Contact
	if err = json.Unmarshal(executeTest(t, s, "contacts.suggest", Params{"query": "张三"}), &contacts); err != nil {
		t.Fatal(err)
	}
	if len(contacts) == 0 || contacts[0].Name != "张三" {
		t.Fatal(contacts)
	}
}

// BenchmarkWeightedContactPrefix 使用一万条虚构联系人测量热索引的限量查询。
func BenchmarkWeightedContactPrefix(b *testing.B) {
	values := make([]mail.Contact, 10000)
	for i := range values {
		values[i] = mail.Contact{Name: fmt.Sprintf("Person %d", i), Email: fmt.Sprintf("person.%d@example.com", i), Last: int64(i)}
	}
	index := buildContactIndex(0, values, nil, nil)
	index.search("person", 12)
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		index.search("person", 12)
	}
}
