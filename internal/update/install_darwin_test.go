//go:build darwin

package update

import (
	"github.com/openfabrica/mailfabrica/internal/store"
	"os"
	"path/filepath"
	"testing"
)

// TestReplaceRollback 验证同卷替换与缺失载荷时的旧包恢复，只使用临时目录。
func TestReplaceRollback(t *testing.T) {
	for _, fail := range []bool{false, true} {
		root := t.TempDir()
		target := filepath.Join(root, "MailFabrica.app")
		payload := filepath.Join(root, "new.app")
		backup := filepath.Join(root, "old.app")
		if e := os.Mkdir(target, 0700); e != nil {
			t.Fatal(e)
		}
		if e := os.WriteFile(filepath.Join(target, "version"), []byte("old"), 0600); e != nil {
			t.Fatal(e)
		}
		if !fail {
			if e := os.Mkdir(payload, 0700); e != nil {
				t.Fatal(e)
			}
			if e := os.WriteFile(filepath.Join(payload, "version"), []byte("new"), 0600); e != nil {
				t.Fatal(e)
			}
		}
		e := replaceApplication(payload, target, backup)
		if fail && e == nil {
			t.Fatal("expected missing payload failure")
		}
		if !fail && e != nil {
			t.Fatal(e)
		}
		b, e := os.ReadFile(filepath.Join(target, "version"))
		if e != nil {
			t.Fatal(e)
		}
		want := "new"
		if fail {
			want = "old"
		}
		if string(b) != want {
			t.Fatalf("got %q want %q", b, want)
		}
	}
}

// TestHelperRequiresNormalExitCommit 保证没有交接标记时绝不改动现有应用。
func TestHelperRequiresNormalExitCommit(t *testing.T) {
	root := t.TempDir()
	stage, e := os.MkdirTemp(root, ".mailfabrica-update-")
	if e != nil {
		t.Fatal(e)
	}
	target := filepath.Join(root, "MailFabrica.app")
	if e = os.Mkdir(target, 0700); e != nil {
		t.Fatal(e)
	}
	h := Handoff{Target: target, Stage: stage, Payload: filepath.Join(stage, "MailFabrica.app"), Backup: target + ".previous-" + filepath.Base(stage), Parent: 99999999, Nonce: "012345678901234567890123456789012345678901234567"}
	file := filepath.Join(stage, "handoff.json")
	if e = store.WriteJSON(file, h); e != nil {
		t.Fatal(e)
	}
	if RunHelper(file) == nil {
		t.Fatal("expected missing commit")
	}
	if _, e = os.Stat(target); e != nil {
		t.Fatal("original application changed")
	}
}
