package ai

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

// TestCompleteProtocols 使用本地假接口验证两种协议，不调用付费模型或真实凭据。
func TestCompleteProtocols(t *testing.T) {
	for _, provider := range []string{"anthropic", "openai"} {
		t.Run(provider, func(t *testing.T) {
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				var body map[string]any
				if e := json.NewDecoder(r.Body).Decode(&body); e != nil {
					t.Error(e)
				}
				if body["model"] != "test-model" {
					t.Error("model missing")
				}
				w.Header().Set("Content-Type", "application/json")
				if provider == "anthropic" {
					if r.URL.Path != "/v1/messages" || r.Header.Get("x-api-key") != "test-only-key" {
						t.Error("anthropic request contract")
					}
					_, _ = fmt.Fprint(w, `{"content":[{"type":"text","text":"收到，谢谢。"}]}`)
				} else {
					if r.URL.Path != "/chat/completions" || r.Header.Get("Authorization") != "Bearer test-only-key" {
						t.Error("openai request contract")
					}
					_, _ = fmt.Fprint(w, `{"choices":[{"message":{"content":"收到，谢谢。"}}]}`)
				}
			}))
			defer server.Close()
			text, e := Complete(context.Background(), Config{Provider: provider, BaseURL: server.URL, Model: "test-model"}, "test-only-key", "请回复")
			if e != nil || text != "收到，谢谢。" {
				t.Fatalf("%q %v", text, e)
			}
		})
	}
}

// TestURLAndErrorPrivacy 验证 HTTP 限制、重定向与敏感响应隔离。
func TestURLAndErrorPrivacy(t *testing.T) {
	for _, url := range []string{"http://example.com", "file:///tmp/a", "https://user:password@example.com", "https://example.com?key=secret"} {
		if ValidateURL(url) == nil {
			t.Fatalf("accepted %s", url)
		}
	}
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(401)
		_, _ = fmt.Fprint(w, "sensitive-provider-response")
	}))
	defer server.Close()
	_, e := Complete(context.Background(), Config{Provider: "openai", BaseURL: server.URL, Model: "test"}, "test-key", "test")
	if e == nil || strings.Contains(e.Error(), "sensitive") {
		t.Fatalf("unsafe error: %v", e)
	}
}
