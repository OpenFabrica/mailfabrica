package mail

import "testing"

// TestKeyAndSubject 验证中文目录和连续前缀的兼容行为。
func TestKeyAndSubject(t *testing.T) {
	k, e := ParseKey("alice-qq-com/42/工作/项目")
	if e != nil || k.Folder != "工作/项目" || k.String() != "alice-qq-com/42/工作/项目" {
		t.Fatalf("key: %v %v", k, e)
	}
	for _, s := range []string{"Re: Fwd:  产品   评审", "回复：Re[2]: 产品 评审"} {
		if got := SubjectKey(s); got != "产品 评审" {
			t.Fatalf("subject %q", got)
		}
	}
	for _, s := range []string{"../a/1/INBOX", "alice/0/INBOX", "alice/x/a", "alice/1/"} {
		if _, e = ParseKey(s); e == nil {
			t.Fatalf("accepted %q", s)
		}
	}
}
