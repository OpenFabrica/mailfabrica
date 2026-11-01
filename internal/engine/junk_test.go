package engine

import (
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/openfabrica/mailfabrica/internal/mail"
)

// TestAccountMarkAllRead 验证按账户一键已读覆盖全部文件夹，且不依赖当前列表范围。
func TestAccountMarkAllRead(t *testing.T) {
	s := testSession(t)
	executeTest(t, s, "view.scope", Params{"account": "demo-mailfabrica-dev", "folder": "Archive"})
	s.mu.Lock()
	before := s.counts.Unread
	s.mu.Unlock()
	if before == 0 {
		t.Fatal("demo must start with unread mail")
	}
	if _, err := s.Execute("account.markAllRead", marshalParams(Params{"id": "missing"})); err == nil {
		t.Fatal("unknown account accepted")
	}
	executeTest(t, s, "account.markAllRead", Params{"id": "demo-mailfabrica-dev"})
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.counts.Unread != 0 {
		t.Fatalf("unread %d", s.counts.Unread)
	}
	idx, err := s.Store.Index("demo-mailfabrica-dev", "INBOX")
	if err != nil {
		t.Fatal(err)
	}
	for _, m := range idx.Messages {
		if !m.Flags.Seen {
			t.Fatalf("%s still unread", m.Key)
		}
	}
}

// TestAIJunkScanAndClean 用本地 AI 接口验证只发送元数据、校验编号，并在确认后移到垃圾邮件。
func TestAIJunkScanAndClean(t *testing.T) {
	s := testSession(t)
	prompts := make(chan string, 1)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var payload struct {
			Messages []struct{ Role, Content string }
		}
		if err := json.NewDecoder(r.Body).Decode(&payload); err != nil {
			t.Error(err)
		}
		for _, m := range payload.Messages {
			if m.Role == "user" {
				prompts <- m.Content
			}
		}
		// 代码块包裹、越界与重复编号都应被容忍或忽略
		answer, _ := json.Marshal("```json\n[{\"id\":1,\"reason\":\"营销广告\"},{\"id\":99,\"reason\":\"越界\"},{\"id\":1,\"reason\":\"重复\"}]\n```")
		_, _ = fmt.Fprintf(w, `{"choices":[{"message":{"content":%s}}]}`, answer)
	}))
	defer server.Close()
	executeTest(t, s, "ai.config", Params{"provider": "openai", "baseUrl": server.URL, "model": "test-only"})
	executeTest(t, s, "ai.scanJunk", Params{"account": "demo-mailfabrica-dev"})
	prompt := <-prompts
	if !strings.Contains(prompt, "[11]") || strings.Contains(prompt, "[12]") || strings.Contains(prompt, "下一阶段规划") {
		t.Fatal("prompt must list inbox metadata only")
	}
	s.mu.Lock()
	scan := s.aiJunk
	if scan == nil || scan.Busy || scan.Error != nil || scan.Scanned != 11 || len(scan.Candidates) != 1 {
		s.mu.Unlock()
		t.Fatalf("scan %+v", scan)
	}
	// 最新邮件编号为 1，对应演示收件箱 UID 1
	key := scan.Candidates[0].Key
	s.mu.Unlock()
	if key != "demo-mailfabrica-dev/1/INBOX" || scan.Candidates[0].Reason != "营销广告" {
		t.Fatalf("candidate %+v", scan.Candidates[0])
	}
	if _, err := s.Execute("ai.cleanJunk", marshalParams(Params{"keys": []any{"demo-mailfabrica-dev/2/INBOX"}})); err == nil {
		t.Fatal("non-candidate moved")
	}
	executeTest(t, s, "ai.cleanJunk", Params{})
	s.mu.Lock()
	defer s.mu.Unlock()
	if len(s.aiJunk.Candidates) != 0 {
		t.Fatal("moved candidate kept")
	}
	if _, err := s.findMeta(mail.Key{Account: "demo-mailfabrica-dev", Folder: "INBOX", UID: 1}); err == nil {
		t.Fatal("message still in inbox")
	}
	junk, err := s.Store.Index("demo-mailfabrica-dev", "Junk")
	if err != nil {
		t.Fatal(err)
	}
	if len(junk.Messages) != 1 {
		t.Fatalf("junk %d", len(junk.Messages))
	}
}
