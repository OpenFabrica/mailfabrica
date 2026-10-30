package desktop

import (
	"strings"
	"testing"
)

// TestExternalLinkValidation 验证活动协议和无主机 URL 在到达系统浏览器前被拒绝。
func TestExternalLinkValidation(t *testing.T) {
	service := &Service{}
	for _, raw := range []string{"javascript:alert(1)", "data:text/html,test", "file:///etc/passwd", "https:", "http:relative", "custom://app"} {
		_, err := service.OpenExternal(raw)
		if err == nil || strings.Contains(err.Error(), "当前入口没有系统浏览器") {
			t.Fatalf("unsafe URL passed validation: %q", raw)
		}
	}
	for _, raw := range []string{"https://example.com/unsubscribe?id=demo", "http://127.0.0.1:7988/unsubscribe", "mailto:contact@example.com"} {
		_, err := service.OpenExternal(raw)
		if err == nil || !strings.Contains(err.Error(), "当前入口没有系统浏览器") {
			t.Fatalf("valid URL rejected before browser: %q, %v", raw, err)
		}
	}
}
