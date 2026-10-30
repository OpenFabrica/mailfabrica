package store

import (
	"github.com/openfabrica/mailfabrica/internal/mail"
	"os"
	"path/filepath"
	"testing"
)

// TestLegacyRoundTrip 验证旧索引字段、原文持久化和重启后的读取。
func TestLegacyRoundTrip(t *testing.T) {
	root := t.TempDir()
	s, e := Open(root)
	if e != nil {
		t.Fatal(e)
	}
	s.Accounts = append(s.Accounts, mail.Account{ID: "demo", Email: "a@example.com"})
	if e = s.SaveAccounts(); e != nil {
		t.Fatal(e)
	}
	if e = s.SetFolders("demo", []mail.Folder{{Account: "demo", Path: "工作/收件", Role: "inbox", Selectable: true}}); e != nil {
		t.Fatal(e)
	}
	k := mail.Key{Account: "demo", Folder: "工作/收件", UID: 12}
	i, e := s.Index(k.Account, k.Folder)
	if e != nil {
		t.Fatal(e)
	}
	i.UIDValidity = 88
	i.Messages = append(i.Messages, mail.Message{Key: k, Subject: "测试"})
	if e = s.SaveIndex(k.Account, k.Folder); e != nil {
		t.Fatal(e)
	}
	if e = s.SaveRaw(k, []byte("Subject: test\r\n\r\nhello")); e != nil {
		t.Fatal(e)
	}
	s, e = Open(root)
	if e != nil {
		t.Fatal(e)
	}
	i, e = s.Index(k.Account, k.Folder)
	if e != nil || i.UIDValidity != 88 || len(i.Messages) != 1 {
		t.Fatalf("index %v %v", i, e)
	}
	b, e := s.Raw(k)
	if e != nil || string(b) != "Subject: test\r\n\r\nhello" {
		t.Fatalf("raw %q %v", b, e)
	}
	if _, e = s.FolderDir("../escape", "INBOX"); e == nil {
		t.Fatal("accepted traversal")
	}
}

// TestCorruptData 保证损坏数据不会被当成空账户覆盖。
func TestCorruptData(t *testing.T) {
	root := t.TempDir()
	if e := os.WriteFile(filepath.Join(root, "accounts.json"), []byte("{"), 0600); e != nil {
		t.Fatal(e)
	}
	if _, e := Open(root); e == nil {
		t.Fatal("expected malformed JSON error")
	}
}

// TestFNVFixture 通过公开 FNV 测试向量确认字节哈希。
func TestFNVFixture(t *testing.T) {
	if got := FolderDirName("hello"); got != "f-a430d84680aabd0b" {
		t.Fatal(got)
	}
}
