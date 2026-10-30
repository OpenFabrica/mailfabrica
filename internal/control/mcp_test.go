package control

import (
	"bytes"
	"encoding/json"
	"github.com/openfabrica/mailfabrica/internal/engine"
	"net/http/httptest"
	"strings"
	"testing"
)

// TestMCPMailContract 验证真实会话工具发现、字符串邮件键与数组收件人发送。
func TestMCPMailContract(t *testing.T) {
	session, e := engine.New(t.TempDir(), true)
	if e != nil {
		t.Fatal(e)
	}
	defer session.Close()
	backend := Backend{Session: session}
	raw, e := backend.callTool("mail_list", map[string]any{})
	if e != nil {
		t.Fatal(e)
	}
	var list struct {
		Messages []struct {
			Key string `json:"key"`
		}
	}
	if e = json.Unmarshal(raw, &list); e != nil || len(list.Messages) == 0 || !strings.Contains(list.Messages[0].Key, "/INBOX") {
		t.Fatalf("list contract %s %v", raw, e)
	}
	sent, e := backend.callTool("mail_send", map[string]any{"to": []any{"one@example.com", "two@example.com"}, "subject": "MCP 测试", "body": "本地假服务器消息"})
	if e != nil || !strings.Contains(string(sent), `"sent":true`) {
		t.Fatalf("send %s %v", sent, e)
	}
	if _, e = backend.Execute("compose.new", map[string]any{}); e != nil {
		t.Fatal(e)
	}
	if _, e = backend.Execute("compose.set", map[string]any{"id": float64(2), "to": "bad address"}); e != nil {
		t.Fatal(e)
	}
	if _, e = backend.Execute("compose.send", map[string]any{"id": float64(2)}); e == nil {
		t.Fatal("invalid recipient accepted")
	}
	if _, e = backend.Execute("account.list", map[string]any{}); e != nil {
		t.Fatalf("old failure poisoned later query: %v", e)
	}
}

// TestMCPStdio 验证初始化、工具目录及协议错误响应可被逐行读取。
func TestMCPStdio(t *testing.T) {
	session, e := engine.New(t.TempDir(), true)
	if e != nil {
		t.Fatal(e)
	}
	defer session.Close()
	input := strings.NewReader("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"mail_accounts\",\"arguments\":{}}}\n")
	var output bytes.Buffer
	if e = ServeMCP(input, &output, Backend{Session: session}); e != nil {
		t.Fatal(e)
	}
	lines := strings.Split(strings.TrimSpace(output.String()), "\n")
	if len(lines) != 3 {
		t.Fatal(output.String())
	}
	for _, line := range lines {
		if !json.Valid([]byte(line)) {
			t.Fatal(line)
		}
	}
	if !strings.Contains(output.String(), "mail_execute") || !strings.Contains(output.String(), "protocolVersion") {
		t.Fatal(output.String())
	}
}

// TestHTTPRejectsCrossOrigin 阻止任意网页向本地验收服务提交业务命令。
func TestHTTPRejectsCrossOrigin(t *testing.T) {
	handler := HTTPHandler(nil)
	request := httptest.NewRequest("POST", "http://127.0.0.1:7982/api/Execute", strings.NewReader(`[]`))
	request.Header.Set("Origin", "https://untrusted.example")
	response := httptest.NewRecorder()
	handler.ServeHTTP(response, request)
	if response.Code != 403 {
		t.Fatalf("status %d", response.Code)
	}
}
