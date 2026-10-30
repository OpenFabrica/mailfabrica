package engine

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"testing"

	"github.com/openfabrica/mailfabrica/internal/mail"
	"github.com/openfabrica/mailfabrica/internal/protocol"
)

// TestGroupNavigation 区分组内上下导航与列表导航，并在边界保持当前邮件。
func TestGroupNavigation(t *testing.T) {
	s := testSession(t)
	executeTest(t, s, "view.selectGroup", Params{"subject": "下周产品评审安排"})
	executeTest(t, s, "view.select", Params{"key": "demo-mailfabrica-dev/1/INBOX"})
	executeTest(t, s, "view.groupNext", Params{})
	s.mu.Lock()
	focused := *s.view.Focused
	s.mu.Unlock()
	if focused != "demo-mailfabrica-dev/8/INBOX" {
		t.Fatal(focused)
	}
	if _, err := s.Execute("view.groupNext", marshalParams(Params{})); err == nil {
		t.Fatal("next beyond group boundary")
	}
	var commands []Command
	if err := json.Unmarshal(s.Commands("view.group"), &commands); err != nil {
		t.Fatal(err)
	}
	for _, c := range commands {
		if c.ID == "view.groupNext" && c.Enabled {
			t.Fatal("next must be disabled at last message")
		}
	}
	executeTest(t, s, "view.groupPrev", Params{})
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.view.Group == nil || s.view.Focused == nil || *s.view.Focused != "demo-mailfabrica-dev/1/INBOX" {
		t.Fatal("group navigation lost context")
	}
}

// TestGroupSummaryReadsFullBodies 使用本地 AI 接口验证缓存与远端正文均参与汇总、且不标记已读。
func TestGroupSummaryReadsFullBodies(t *testing.T) {
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
		_, _ = fmt.Fprint(w, `{"choices":[{"message":{"content":"结论：确认评审时间；待办：准备评审议程。[邮件 1、2]"}}]}`)
	}))
	defer server.Close()
	executeTest(t, s, "ai.config", Params{"provider": "openai", "baseUrl": server.URL, "model": "test-only"})
	// 删除一封本地原文，证明一键汇总会取回正文而非只发送摘要预览。
	key := mail.Key{Account: "demo-mailfabrica-dev", Folder: "INBOX", UID: 8}
	s.mu.Lock()
	delete(s.bodies, key.String())
	dir, err := s.Store.FolderDir(key.Account, key.Folder)
	s.mu.Unlock()
	if err != nil {
		t.Fatal(err)
	}
	if err = os.Remove(fmt.Sprintf("%s/%d.eml", dir, key.UID)); err != nil {
		t.Fatal(err)
	}
	executeTest(t, s, "ai.summarizeGroup", Params{"group": mail.SubjectKey("下周产品评审安排")})
	s.mu.Lock()
	defer s.mu.Unlock()
	result := s.aiGroupSummary
	if result == nil || result.Busy || result.Error != nil || result.Text == nil || len(result.Keys) != 2 {
		t.Fatalf("summary %+v", result)
	}
	prompt := <-prompts
	if !strings.Contains(prompt, "--- 邮件 1 ---") || !strings.Contains(prompt, "--- 邮件 2 ---") || !strings.Contains(prompt, "会议时间：下周三") || !strings.Contains(prompt, "欢迎提出你的想法") {
		t.Fatal("summary omitted full group bodies")
	}
	if s.view.Focused != nil {
		t.Fatal("summary changed selection")
	}
	index, err := s.Store.Index(key.Account, key.Folder)
	if err != nil {
		t.Fatal(err)
	}
	for _, m := range index.Messages {
		if m.Key.UID == 1 && m.Flags.Seen {
			t.Fatal("summary marked message read")
		}
	}
}

// TestGroupSummaryFailure 后端失败时结束加载并显示错误，不返回虚构摘要。
func TestGroupSummaryFailure(t *testing.T) {
	s := testSession(t)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { w.WriteHeader(503) }))
	defer server.Close()
	executeTest(t, s, "ai.config", Params{"provider": "openai", "baseUrl": server.URL, "model": "test-only"})
	executeTest(t, s, "ai.summarizeGroup", Params{"group": mail.SubjectKey("下周产品评审安排")})
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.aiGroupSummary == nil || s.aiGroupSummary.Busy || s.aiGroupSummary.Error == nil || s.aiGroupSummary.Text != nil {
		t.Fatalf("summary %+v", s.aiGroupSummary)
	}
}

// recipientCapture 捕获实际发送信封，委托原有内存远端完成发送。
type recipientCapture struct {
	protocol.Remote
	outgoing mail.Outgoing
}

// Send 保存三个地址列表，便于验证 BCC 在信封中而不泄露到邮件头。
func (r *recipientCapture) Send(ctx context.Context, a mail.Account, password string, out mail.Outgoing) ([]byte, error) {
	r.outgoing = out
	return r.Remote.Send(ctx, a, password, out)
}

// TestMultiRecipientsSend 验证多个收件人、抄送与密送均进入发送信封，且密送不出现在 MIME 头。
func TestMultiRecipientsSend(t *testing.T) {
	s := testSession(t)
	capture := &recipientCapture{Remote: s.remote}
	s.remote = capture
	var compose struct{ ID uint64 }
	if err := json.Unmarshal(executeTest(t, s, "compose.new", Params{}), &compose); err != nil {
		t.Fatal(err)
	}
	executeTest(t, s, "compose.set", Params{"id": float64(compose.ID), "to": `"产品, 团队" <a@example.com>, b@example.com`, "cc": "c@example.com, d@example.com", "bcc": "e@example.com, f@example.com", "body": "多个地址验证"})
	executeTest(t, s, "compose.send", Params{"id": float64(compose.ID)})
	if len(capture.outgoing.To) != 2 || len(capture.outgoing.CC) != 2 || len(capture.outgoing.BCC) != 2 {
		t.Fatal("recipient lists lost")
	}
	raw, err := protocol.Build(capture.outgoing)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(raw), "e@example.com") || strings.Contains(string(raw), "f@example.com") {
		t.Fatal("BCC leaked into MIME")
	}
}
