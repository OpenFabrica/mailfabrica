package protocol

import (
	"encoding/base64"
	"mime"
	"strings"
	"testing"

	"github.com/openfabrica/mailfabrica/internal/mail"
	"golang.org/x/text/encoding/simplifiedchinese"
)

// TestDecodeHeaders 覆盖 UTF-8 B/Q、折行混合编码、GBK 和损坏词回退。
func TestDecodeHeaders(t *testing.T) {
	gbk, err := simplifiedchinese.GBK.NewEncoder().String("中文标题")
	if err != nil {
		t.Fatal(err)
	}
	for _, test := range []struct{ input, want string }{
		{mime.BEncoding.Encode("UTF-8", "张三"), "张三"},
		{mime.QEncoding.Encode("UTF-8", "中文标题"), "中文标题"},
		{mime.BEncoding.Encode("UTF-8", "张") + "\r\n " + mime.BEncoding.Encode("UTF-8", "三"), "张三"},
		{"=?GBK?B?" + base64.StdEncoding.EncodeToString([]byte(gbk)) + "?=", "中文标题"},
		{"=?UTF-8?B?broken!?=", "=?UTF-8?B?broken!?="},
		{"Jane Smith", "Jane Smith"},
	} {
		if got := DecodeHeader(test.input); got != test.want {
			t.Errorf("decoded %q, want %q", got, test.want)
		}
	}
}

// TestMIMEAddressNames 验证阅读和回复使用的姓名、多个收件人与抄送均已解码。
func TestMIMEAddressNames(t *testing.T) {
	encoded := mime.BEncoding.Encode("UTF-8", "张三")
	raw := []byte("From: " + encoded + " <sender@example.com>\r\nTo: " + encoded + " <a@example.com>, " + mime.QEncoding.Encode("UTF-8", "李四") + " <b@example.com>\r\nCc: " + encoded + " <c@example.com>\r\nSubject: " + mime.BEncoding.Encode("UTF-8", "项目周会纪要") + "\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n正文")
	body, err := Parse(raw)
	if err != nil {
		t.Fatal(err)
	}
	if body.Subject != "项目周会纪要" || body.From[0].Name != "张三" || len(body.To) != 2 || body.To[1].Name != "李四" || body.CC[0].Name != "张三" {
		t.Fatalf("decoded body %+v", body)
	}
	meta := DecodeMetadata(mail.Message{Subject: encoded, From: mail.Address{Name: encoded}, To: []mail.Address{{Name: encoded}}})
	if strings.Contains(meta.Subject+meta.From.Name+meta.To[0].Name, "=?") {
		t.Fatal("IMAP/cache metadata still contains encoded words")
	}
}
