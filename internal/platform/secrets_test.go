package platform

import (
	"os"
	"path/filepath"
	"runtime"
	"testing"
)

// TestFileSecretsRoundTrip 验证显式文件模式兼容，不接触系统凭据。
func TestFileSecretsRoundTrip(t *testing.T) {
	t.Setenv("MAILFABRICA_SECRETS", "file")
	root := t.TempDir()
	s := NewSecrets(root, false)
	if e := s.Set("account", "test-only-secret"); e != nil {
		t.Fatal(e)
	}
	other := NewSecrets(root, false)
	v, e := other.Get("account")
	if e != nil || v != "test-only-secret" {
		t.Fatal("credential round trip failed")
	}
	info, e := os.Stat(filepath.Join(root, "secrets.json"))
	if e != nil {
		t.Fatal(e)
	}
	if runtime.GOOS != "windows" && info.Mode().Perm()&0077 != 0 {
		t.Fatal("credential file readable by other users")
	}
	if e = other.Set("account", ""); e != nil {
		t.Fatal(e)
	}
	v, e = s.Get("account")
	if e != nil || v != "" {
		t.Fatal("credential deletion failed")
	}
}
