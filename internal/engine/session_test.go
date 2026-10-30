package engine

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
	"time"

	"github.com/openfabrica/mailfabrica/internal/mail"
)

// testSession 创建使用隔离目录的演示会话。
func testSession(t *testing.T) *Session {
	t.Helper()
	s, e := New(t.TempDir(), true)
	if e != nil {
		t.Fatal(e)
	}
	t.Cleanup(s.Close)
	return s
}

// executeTest 通过真正命令入口执行并等待后台任务。
func executeTest(t *testing.T, s *Session, id string, p Params) json.RawMessage {
	t.Helper()
	b, e := s.Execute(id, marshalParams(p))
	if e != nil {
		t.Fatalf("%s: %v", id, e)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if e = s.Wait(ctx); e != nil {
		t.Fatal(e)
	}
	return b
}

// TestDemoReadComposeSend 验证从列表、阅读、中文富文本到发送归档的完整闭环。
func TestDemoReadComposeSend(t *testing.T) {
	s := testSession(t)
	if len(s.commands) != 78 {
		t.Fatalf("commands %d", len(s.commands))
	}
	var list struct {
		Messages []struct {
			Key struct {
				Account, Folder string
				UID             uint32
			}
		}
	}
	if e := json.Unmarshal(executeTest(t, s, "message.list", Params{}), &list); e != nil {
		t.Fatal(e)
	}
	if len(list.Messages) < 5 {
		t.Fatal("demo messages missing")
	}
	key := "demo-mailfabrica-dev/1/INBOX"
	executeTest(t, s, "view.select", Params{"key": key})
	body := executeTest(t, s, "message.get", Params{"key": key})
	if !strings.Contains(string(body), `"state":"ready"`) {
		t.Fatal(string(body))
	}
	var created struct {
		ID uint64 `json:"id"`
	}
	if e := json.Unmarshal(executeTest(t, s, "compose.reply", Params{}), &created); e != nil {
		t.Fatal(e)
	}
	params := Params{"id": float64(created.ID), "body": "你好🙂，收到。"}
	executeTest(t, s, "compose.set", params)
	executeTest(t, s, "compose.bold", Params{"id": float64(created.ID), "start": float64(0), "end": float64(3)})
	s.mu.Lock()
	c := s.composes[created.ID]
	if !strings.Contains(c.HTML, "你好🙂") || !strings.Contains(c.HTML, "font-weight:700") {
		t.Errorf("rich html %s", c.HTML)
	}
	s.mu.Unlock()
	executeTest(t, s, "compose.send", Params{"id": float64(created.ID)})
	s.mu.Lock()
	defer s.mu.Unlock()
	if len(s.composes) != 0 {
		t.Fatal("composer did not close")
	}
	idx, e := s.Store.Index("demo-mailfabrica-dev", "Sent")
	if e != nil || len(idx.Messages) != 1 {
		t.Fatalf("sent: %v %v", idx, e)
	}
	if len(s.status.Errors) > 0 {
		t.Fatal(s.status.Errors)
	}
}

// TestPreferencesAndInvalidInput 验证参数错误不会静默变成零值。
func TestPreferencesAndInvalidInput(t *testing.T) {
	s := testSession(t)
	for _, tc := range []struct{ id, params string }{{"compose.new", `[]`}, {"view.scope", `{"kind":"unknown"}`}, {"message.list", `{"limit":-1}`}, {"app.prefs", `{"remoteImages":"false"}`}, {"account.remove", `{"id":"../bad"}`}} {
		if _, e := s.Execute(tc.id, json.RawMessage(tc.params)); e == nil {
			t.Errorf("accepted %s %s", tc.id, tc.params)
		}
	}
	executeTest(t, s, "app.prefs", Params{"remoteImages": false})
	s2, e := New(s.Store.Root, true)
	if e != nil {
		t.Fatal(e)
	}
	defer s2.Close()
	if boolean(s2.prefs, "remoteImages", true) {
		t.Fatal("preference not persisted")
	}
}

// TestGroupAndInvoiceExport 验证主题分组、批量标记和真实导出产物。
func TestGroupAndInvoiceExport(t *testing.T) {
	s := testSession(t)
	executeTest(t, s, "view.selectGroup", Params{"subject": "Re: 下周产品评审安排"})
	s.mu.Lock()
	n := len(s.view.Selected)
	s.mu.Unlock()
	if n != 2 {
		t.Fatalf("group size %d", n)
	}
	executeTest(t, s, "message.flag", Params{"flagged": true})
	dir := t.TempDir()
	executeTest(t, s, "invoice.export", Params{"dir": dir})
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.invoice.Busy || s.invoice.Files != 1 || s.invoice.CSV == nil {
		t.Fatalf("invoice: %+v", s.invoice)
	}
}

// TestRichRoundTrip 验证 HTML 格式和 Unicode 字符偏移。
func TestRichRoundTrip(t *testing.T) {
	d, e := docFromHTML(`<p>你好<strong>🙂</strong>世界</p><ul><li>事项</li></ul>`)
	if e != nil {
		t.Fatal(e)
	}
	if d.Text != "你好🙂世界\n事项" {
		t.Fatalf("text %q", d.Text)
	}
	if !d.styleAt(2).Bold {
		t.Fatal("emoji bold missing")
	}
	d.replace("你好🙂新世界\n事项", plainStyle())
	if !d.styleAt(2).Bold || d.styleAt(3).Bold {
		t.Fatal("edit style not preserved")
	}
}

// TestAccountAddStartsSync 验证新账户验证与首次同步属于同一个可等待的业务闭环。
func TestAccountAddStartsSync(t *testing.T) {
	s := testSession(t)
	executeTest(t, s, "account.add", Params{"email": "new@example.com", "password": "demo-only", "verify": true})
	s.mu.Lock()
	defer s.mu.Unlock()
	if len(s.Store.Accounts) != 2 || s.status.LastSync == 0 || len(s.status.Errors) != 0 {
		t.Fatalf("account/sync state: accounts=%d status=%+v", len(s.Store.Accounts), s.status)
	}
}

// TestDemoDatesSurviveSync 保证刷新不会把所有演示邮件日期改成构建时刻。
func TestDemoDatesSurviveSync(t *testing.T) {
	s := testSession(t)
	s.mu.Lock()
	index, err := s.Store.Index("demo-mailfabrica-dev", "INBOX")
	if err != nil {
		s.mu.Unlock()
		t.Fatal(err)
	}
	dates := map[uint32]int64{}
	for _, m := range index.Messages {
		dates[m.Key.UID] = m.Date
	}
	s.mu.Unlock()
	executeTest(t, s, "mail.sync", Params{})
	s.mu.Lock()
	defer s.mu.Unlock()
	for _, m := range index.Messages {
		if m.Date != dates[m.Key.UID] {
			t.Fatalf("UID %d date changed", m.Key.UID)
		}
	}
}

// TestReplyQuotePreview 验证 HTML/CID 预览不改写发送引用，纯文本引用保留换行并转义内容。
func TestReplyQuotePreview(t *testing.T) {
	s := testSession(t)
	key := "demo-mailfabrica-dev/1/INBOX"
	cid := "quote-image"
	original := `<h2 style="color:#245584">项目周会</h2><table><tr><td>会议主题</td><td>计划</td></tr></table><img src="cid:quote-image">`
	s.mu.Lock()
	s.bodies[key] = mail.Body{Subject: "项目周会", From: []mail.Address{{Name: "张三", Email: "sender@example.com"}}, HTML: &original, Text: "会议主题：计划", Attachments: []mail.Attachment{{MIME: "image/png", Inline: true, ContentID: &cid, Data: []byte("image-data")}}}
	s.mu.Unlock()
	var created struct{ ID uint64 }
	if err := json.Unmarshal(executeTest(t, s, "compose.reply", Params{"key": key}), &created); err != nil {
		t.Fatal(err)
	}
	var state struct{ Composes []Compose }
	if err := json.Unmarshal(s.Snapshot(), &state); err != nil {
		t.Fatal(err)
	}
	if len(state.Composes) != 1 || state.Composes[0].Quote == nil {
		t.Fatal("missing quote")
	}
	preview := state.Composes[0].Quote.HTML
	if !strings.Contains(preview, "<table>") || !strings.Contains(preview, "data:image/png;base64,") {
		t.Fatal("preview lost HTML or inline image")
	}
	s.mu.Lock()
	stored := s.composes[created.ID].Quote.HTML
	s.mu.Unlock()
	if !strings.Contains(stored, "cid:quote-image") || strings.Contains(stored, "data:image") {
		t.Fatal("preview mutated outgoing quote")
	}
	executeTest(t, s, "compose.removeQuote", Params{"id": float64(created.ID)})
	if err := json.Unmarshal(s.Snapshot(), &state); err != nil {
		t.Fatal(err)
	}
	if state.Composes[0].Quote != nil {
		t.Fatal("quote was not removed")
	}
	executeTest(t, s, "compose.discard", Params{"id": float64(created.ID)})
	s.mu.Lock()
	s.bodies[key] = mail.Body{Subject: "纯文本", From: []mail.Address{{Email: "sender@example.com"}}, Text: "第一行\n<script>第二行</script>"}
	s.mu.Unlock()
	if err := json.Unmarshal(executeTest(t, s, "compose.reply", Params{"key": key}), &created); err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(s.Snapshot(), &state); err != nil {
		t.Fatal(err)
	}
	quote := state.Composes[0].Quote.HTML
	if !strings.Contains(quote, "white-space:pre-wrap") || !strings.Contains(quote, "第一行\n&lt;script&gt;第二行&lt;/script&gt;") {
		t.Fatal("plain quote lost whitespace or escaping")
	}
}
