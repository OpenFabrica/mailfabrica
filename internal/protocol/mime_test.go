package protocol

import (
	"github.com/openfabrica/mailfabrica/internal/mail"
	"strings"
	"testing"
)

// TestMIMERoundTrip 验证中文、多格式、附件和 BCC 信封隔离。
func TestMIMERoundTrip(t *testing.T) {
	raw, e := Build(mail.Outgoing{From: mail.Address{Name: "张三", Email: "a@example.com"}, To: []mail.Address{{Email: "b@example.com"}}, BCC: []mail.Address{{Email: "secret@example.com"}}, Subject: "评审🙂", Text: "中文正文", HTML: "<p>中文<strong>正文</strong></p>", Attachments: []mail.Attachment{{Name: "清单.txt", MIME: "text/plain", Data: []byte("123")}}})
	if e != nil {
		t.Fatal(e)
	}
	if strings.Contains(string(raw), "secret@example.com") {
		t.Fatal("BCC leaked")
	}
	b, e := Parse(raw)
	if e != nil {
		t.Fatal(e)
	}
	if b.Subject != "评审🙂" || b.Text != "中文正文" || b.HTML == nil || len(b.Attachments) != 1 || string(b.Attachments[0].Data) != "123" || b.Attachments[0].Name != "清单.txt" {
		t.Fatalf("round trip: %+v", b)
	}
}
