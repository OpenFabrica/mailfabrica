// Package update 仅从固定发布源下载并验证本机安装包。
package update

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"github.com/openfabrica/mailfabrica/internal/store"
	"golang.org/x/mod/semver"
	"io"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"time"
)

const Repository = "https://github.com/OpenFabrica/mailfabrica"
const MaxInstaller = 128 << 20

// Release 的 URL、名称、大小和哈希均来自已校验的固定仓库元数据。
type Release struct {
	Version string `json:"version"`
	Notes   string `json:"notes"`
	Page    string `json:"page"`
	Asset   string `json:"asset"`
	URL     string `json:"url"`
	Size    int64  `json:"size"`
	SHA256  string `json:"sha256"`
}

// limitedRead 拒绝超大更新响应。
func limitedRead(r io.Reader, n int64) ([]byte, error) {
	b, e := io.ReadAll(io.LimitReader(r, n+1))
	if e != nil {
		return nil, e
	}
	if int64(len(b)) > n {
		return nil, fmt.Errorf("更新文件超出大小限制")
	}
	return b, nil
}

// Check 查找比当前版本新的正式版本，不接受自定义来源。
func Check(ctx context.Context, current string) (*Release, error) {
	ctx, cancel := context.WithTimeout(ctx, 25*time.Second)
	defer cancel()
	req, e := http.NewRequestWithContext(ctx, "GET", "https://api.github.com/repos/OpenFabrica/mailfabrica/releases/latest", nil)
	if e != nil {
		return nil, e
	}
	req.Header.Set("Accept", "application/vnd.github+json")
	response, e := updateClient().Do(req)
	if e != nil {
		return nil, fmt.Errorf("无法检查更新")
	}
	defer response.Body.Close()
	if response.StatusCode == 404 {
		return nil, nil
	}
	if response.StatusCode != 200 {
		return nil, fmt.Errorf("更新检查返回 HTTP %d", response.StatusCode)
	}
	b, e := limitedRead(response.Body, 2<<20)
	if e != nil {
		return nil, e
	}
	var metadata struct {
		Tag        string `json:"tag_name"`
		Draft      bool   `json:"draft"`
		Prerelease bool   `json:"prerelease"`
		Page       string `json:"html_url"`
		Body       string `json:"body"`
		Assets     []struct {
			Name   string `json:"name"`
			URL    string `json:"browser_download_url"`
			Size   int64  `json:"size"`
			Digest string `json:"digest"`
		} `json:"assets"`
	}
	if e = json.Unmarshal(b, &metadata); e != nil {
		return nil, fmt.Errorf("更新元数据无效")
	}
	if metadata.Draft || metadata.Prerelease {
		return nil, nil
	}
	tag := metadata.Tag
	if !semver.IsValid(tag) || semver.Prerelease(tag) != "" {
		return nil, fmt.Errorf("更新版本无效")
	}
	if semver.Compare(tag, "v"+strings.TrimPrefix(current, "v")) <= 0 {
		return nil, nil
	}
	version := strings.TrimPrefix(tag, "v")
	page := Repository + "/releases/tag/" + tag
	if metadata.Page != page {
		return nil, fmt.Errorf("更新来源无效")
	}
	arch := runtime.GOARCH
	name := ""
	switch runtime.GOOS {
	case "darwin":
		if arch == "amd64" {
			arch = "x86_64"
		}
		name = "MailFabrica-" + version + "-" + arch + ".dmg"
	case "windows":
		name = "MailFabrica-" + version + "-windows-" + arch + ".zip"
	case "linux":
		name = "MailFabrica-" + version + "-linux-" + arch + ".tar.gz"
	default:
		return nil, fmt.Errorf("当前平台没有更新包")
	}
	var result *Release
	for _, a := range metadata.Assets {
		if a.Name != name {
			continue
		}
		if result != nil {
			return nil, fmt.Errorf("更新资产重复")
		}
		url := Repository + "/releases/download/" + tag + "/" + name
		if a.URL != url || a.Size < 1 || a.Size > MaxInstaller {
			return nil, fmt.Errorf("更新资产无效")
		}
		digest := strings.TrimPrefix(a.Digest, "sha256:")
		if !strings.HasPrefix(a.Digest, "sha256:") || len(digest) != 64 {
			return nil, fmt.Errorf("更新缺少 SHA-256")
		}
		if _, e := hex.DecodeString(digest); e != nil {
			return nil, fmt.Errorf("更新哈希无效")
		}
		notes := []rune(metadata.Body)
		if len(notes) > 20000 {
			notes = notes[:20000]
		}
		result = &Release{Version: version, Page: page, Notes: string(notes), Asset: name, URL: url, Size: a.Size, SHA256: strings.ToLower(digest)}
	}
	if result == nil {
		return nil, fmt.Errorf("没有适合本机的更新安装包")
	}
	return result, nil
}

// VerifyBytes 同时检查实际大小和内容哈希。
func VerifyBytes(r Release, b []byte) error {
	if r.Size < 1 || r.Size > MaxInstaller || int64(len(b)) != r.Size {
		return fmt.Errorf("安装包大小不匹配")
	}
	hash := sha256.Sum256(b)
	if hex.EncodeToString(hash[:]) != r.SHA256 {
		return fmt.Errorf("安装包 SHA-256 校验失败")
	}
	return nil
}

// Download 下载、校验、持久化，再验证系统发布者。
func Download(ctx context.Context, r Release, root string, progress func(int64)) (string, error) {
	if r.URL != Repository+"/releases/download/v"+r.Version+"/"+r.Asset || filepath.Base(r.Asset) != r.Asset {
		return "", fmt.Errorf("安装包来源无效")
	}
	ctx, cancel := context.WithTimeout(ctx, 5*time.Minute)
	defer cancel()
	req, e := http.NewRequestWithContext(ctx, "GET", r.URL, nil)
	if e != nil {
		return "", e
	}
	response, e := updateClient().Do(req)
	if e != nil {
		return "", fmt.Errorf("下载更新失败")
	}
	defer response.Body.Close()
	if response.StatusCode != 200 {
		return "", fmt.Errorf("更新下载返回 HTTP %d", response.StatusCode)
	}
	b := make([]byte, 0, r.Size)
	buf := make([]byte, 64<<10)
	for {
		n, err := response.Body.Read(buf)
		if n > 0 {
			if int64(len(b)+n) > r.Size {
				return "", fmt.Errorf("安装包超出声明大小")
			}
			b = append(b, buf[:n]...)
			if progress != nil {
				progress(int64(len(b)))
			}
		}
		if err == io.EOF {
			break
		}
		if err != nil {
			return "", err
		}
	}
	if e = VerifyBytes(r, b); e != nil {
		return "", e
	}
	path := filepath.Join(root, "updates", r.Asset)
	if e = store.WriteAtomic(path, b); e != nil {
		return "", e
	}
	if e = VerifyPublisher(ctx, path); e != nil {
		_ = os.Remove(path)
		return "", e
	}
	return path, nil
}

// VerifyPublisher 验证 Apple Team 与 Gatekeeper；其他平台保留固定来源和摘要验证。
func VerifyPublisher(ctx context.Context, path string) error {
	if runtime.GOOS != "darwin" {
		return nil
	}
	ctx, cancel := context.WithTimeout(ctx, 60*time.Second)
	defer cancel()
	for i, args := range [][]string{{"/usr/bin/codesign", "--verify", "--strict", path}, {"/usr/bin/codesign", "-d", "--verbose=4", path}, {"/usr/sbin/spctl", "--assess", "--type", "open", "--context", "context:primary-signature", path}} {
		b, e := exec.CommandContext(ctx, args[0], args[1:]...).CombinedOutput()
		if e != nil {
			return fmt.Errorf("Apple 签名或公证验证失败")
		}
		if i == 1 {
			found := false
			for _, line := range strings.Split(string(b), "\n") {
				found = found || strings.TrimSpace(line) == "TeamIdentifier=HQ537XMLJY"
			}
			if !found {
				return fmt.Errorf("安装包不是 MailFabrica 发布者签名")
			}
		}
	}
	return nil
}

// updateClient 保留更新通道 HTTPS 和有限跳转约束，不携带用户邮箱认证信息。
func updateClient() *http.Client {
	return &http.Client{CheckRedirect: func(request *http.Request, via []*http.Request) error {
		if request.URL.Scheme != "https" || len(via) > 5 {
			return fmt.Errorf("更新来源跳转无效")
		}
		return nil
	}}
}
