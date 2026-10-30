package update

import (
	"crypto/sha256"
	"encoding/hex"
	"testing"
)

// TestVerifyBytes 验证截断、额外内容、错误哈希与正确安装包的分支。
func TestVerifyBytes(t *testing.T) {
	payload := []byte("signed installer fixture")
	sum := sha256.Sum256(payload)
	release := Release{Size: int64(len(payload)), SHA256: hex.EncodeToString(sum[:])}
	if e := VerifyBytes(release, payload); e != nil {
		t.Fatal(e)
	}
	for _, bad := range [][]byte{payload[:len(payload)-1], append(append([]byte{}, payload...), 0), []byte("bad installer fixture!!!")} {
		if VerifyBytes(release, bad) == nil {
			t.Fatal("accepted modified installer")
		}
	}
}
