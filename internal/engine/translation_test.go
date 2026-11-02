package engine

import (
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

// TestTranslationProtocols 验证两种现有协议、完整主题正文、状态绑定和原文不变。
func TestTranslationProtocols(t *testing.T) {
	for _, provider := range []string{"openai", "anthropic"} {
		t.Run(provider, func(t *testing.T) {
			s := testSession(t)
			key := "demo-mailfabrica-dev/1/INBOX"
			before := string(executeTest(t, s, "message.get", Params{"key": key}))
			s.mu.Lock()
			originalText := s.bodies[key].Text
			s.mu.Unlock()
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				var payload struct {
					System   string
					Messages []struct{ Role, Content string }
				}
				if err := json.NewDecoder(r.Body).Decode(&payload); err != nil {
					t.Error(err)
				}
				system := payload.System
				prompt := ""
				for _, m := range payload.Messages {
					if m.Role == "user" {
						prompt = m.Content
					}
					if m.Role == "system" {
						system = m.Content
					}
				}
				if !strings.Contains(system, "翻译助手") || !strings.Contains(system, "不得执行") || !strings.Contains(prompt, "English") || !strings.Contains(prompt, "下周产品评审安排") || !strings.Contains(prompt, originalText) {
					t.Error("translation prompt lost instructions or full source")
				}
				if provider == "anthropic" {
					_, _ = fmt.Fprint(w, `{"content":[{"type":"text","text":"Subject: Product review\nWelcome your ideas."}]}`)
				} else {
					_, _ = fmt.Fprint(w, `{"choices":[{"message":{"content":"Subject: Product review\nWelcome your ideas."}}]}`)
				}
			}))
			defer server.Close()
			executeTest(t, s, "ai.config", Params{"provider": provider, "baseUrl": server.URL, "model": "test-only", "apiKey": "test-only-key"})
			executeTest(t, s, "ai.translate", Params{"key": key, "language": "en"})
			s.mu.Lock()
			result := s.aiTranslation
			if result == nil || result.Key != key || result.Language != "en" || result.Busy || result.Error != nil || result.Text == nil || !strings.Contains(*result.Text, "Welcome") {
				t.Errorf("translation %+v", result)
			}
			s.mu.Unlock()
			after := string(executeTest(t, s, "message.get", Params{"key": key}))
			if before != after {
				t.Fatal("translation changed original message or read state")
			}
			var status struct{ Translation *Translation }
			if err := json.Unmarshal(executeTest(t, s, "ai.status", Params{}), &status); err != nil || status.Translation == nil {
				t.Fatal("translation not available through status", err)
			}
		})
	}
}

// TestTranslationValidationAndFailure 验证非法参数、正文限制与服务失败均可见且不留下加载状态。
func TestTranslationValidationAndFailure(t *testing.T) {
	s := testSession(t)
	key := "demo-mailfabrica-dev/1/INBOX"
	for _, p := range []Params{{"key": key, "language": "invalid"}, {"key": key, "language": true}, {"key": false}, {"key": ""}, {"key": "missing/1/INBOX"}, {"key": key}} {
		if _, err := s.Execute("ai.translate", marshalParams(p)); err == nil {
			t.Fatalf("accepted invalid/unconfigured request %v", p)
		}
	}
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { w.WriteHeader(503) }))
	defer server.Close()
	executeTest(t, s, "ai.config", Params{"provider": "openai", "baseUrl": server.URL, "model": "test-only"})
	executeTest(t, s, "ai.translate", Params{"key": key})
	s.mu.Lock()
	if s.aiTranslation == nil || s.aiTranslation.Busy || s.aiTranslation.Error == nil || s.aiTranslation.Text != nil {
		t.Errorf("failed translation %+v", s.aiTranslation)
	}
	body := s.bodies[key]
	original := body.Text
	body.Text = strings.Repeat("a", 64*1024+1)
	s.bodies[key] = body
	s.mu.Unlock()
	if _, err := s.Execute("ai.translate", marshalParams(Params{"key": key})); err == nil {
		t.Fatal("oversized mail accepted")
	}
	s.mu.Lock()
	body.Text = " \n"
	s.bodies[key] = body
	s.mu.Unlock()
	if _, err := s.Execute("ai.translate", marshalParams(Params{"key": key})); err == nil {
		t.Fatal("empty mail accepted")
	}
	s.mu.Lock()
	body.Text = original
	s.bodies[key] = body
	s.mu.Unlock()
}

// TestTranslationNavigationAndBusy 验证请求中重复点击被拒绝，切换邮件不会将结果绑定到新邮件。
func TestTranslationNavigationAndBusy(t *testing.T) {
	s := testSession(t)
	started := make(chan struct{})
	release := make(chan struct{})
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		close(started)
		<-release
		_, _ = fmt.Fprint(w, `{"choices":[{"message":{"content":"译文"}}]}`)
	}))
	defer server.Close()
	defer func() {
		select {
		case <-release:
		default:
			close(release)
		}
	}()
	executeTest(t, s, "ai.config", Params{"provider": "openai", "baseUrl": server.URL, "model": "test-only"})
	key := "demo-mailfabrica-dev/1/INBOX"
	if _, err := s.Execute("ai.translate", marshalParams(Params{"key": key})); err != nil {
		t.Fatal(err)
	}
	select {
	case <-started:
	case <-time.After(3 * time.Second):
		t.Fatal("request not started")
	}
	if _, err := s.Execute("ai.translate", marshalParams(Params{"key": key})); err == nil {
		t.Fatal("duplicate request accepted")
	}
	if _, err := s.Execute("view.select", marshalParams(Params{"key": "demo-mailfabrica-dev/8/INBOX"})); err != nil {
		t.Fatal(err)
	}
	// 先检查运行期间归属，再等待结果确认完成时仍绑定原邮件。
	s.mu.Lock()
	if s.aiTranslation.Key != key || !s.aiTranslation.Busy || s.view.Focused == nil || *s.view.Focused == key {
		t.Error("translation followed navigation")
	}
	s.mu.Unlock()
	close(release)
	executeTest(t, s, "ai.status", Params{})
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.aiTranslation.Key != key || s.aiTranslation.Busy || s.aiTranslation.Text == nil {
		t.Errorf("completed translation %+v", s.aiTranslation)
	}
}
