//go:build darwin

package update

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"github.com/openfabrica/mailfabrica/internal/store"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"syscall"
	"time"
)

// Handoff 保存只由宿主配置的安装交接，不允许控制命令指定替换目标。
type Handoff struct {
	Target  string   `json:"target"`
	Stage   string   `json:"stage"`
	Payload string   `json:"payload"`
	Backup  string   `json:"backup"`
	Parent  int      `json:"parent"`
	Nonce   string   `json:"nonce"`
	File    string   `json:"-"`
	Args    []string `json:"args,omitempty"`
}

// ApplicationTarget 从正在运行的可执行文件定位现有应用包。
func ApplicationTarget(executable string) string {
	path, e := filepath.Abs(executable)
	if e != nil {
		return ""
	}
	macos := filepath.Dir(path)
	contents := filepath.Dir(macos)
	target := filepath.Dir(contents)
	if filepath.Base(macos) != "MacOS" || filepath.Base(contents) != "Contents" || !strings.HasSuffix(target, ".app") {
		return ""
	}
	return target
}

// verifyBundle 校验待替换应用的发布者、签名、标识与可执行文件。
func verifyBundle(ctx context.Context, path string) error {
	for _, args := range [][]string{{"/usr/bin/codesign", "--verify", "--deep", "--strict", path}, {"/usr/bin/codesign", "-d", "--verbose=4", path}, {"/usr/sbin/spctl", "--assess", "--type", "execute", path}} {
		b, e := exec.CommandContext(ctx, args[0], args[1:]...).CombinedOutput()
		if e != nil {
			return fmt.Errorf("应用签名或公证校验失败")
		}
		if len(args) > 2 && args[1] == "-d" {
			match := false
			for _, line := range strings.Split(string(b), "\n") {
				match = match || strings.TrimSpace(line) == "TeamIdentifier=HQ537XMLJY"
			}
			if !match {
				return fmt.Errorf("应用发布者不匹配")
			}
		}
	}
	b, e := exec.CommandContext(ctx, "/usr/libexec/PlistBuddy", "-c", "Print :CFBundleIdentifier", filepath.Join(path, "Contents", "Info.plist")).Output()
	if e != nil || strings.TrimSpace(string(b)) != "dev.mailfabrica.MailFabrica" {
		return fmt.Errorf("应用标识不匹配")
	}
	info, e := os.Stat(filepath.Join(path, "Contents", "MacOS", "mailfabrica"))
	if e != nil || !info.Mode().IsRegular() || info.Mode()&0111 == 0 {
		return fmt.Errorf("安装包缺少有效可执行文件")
	}
	return nil
}

// Prepare 只读挂载已校验 DMG，在目标同一文件系统准备已验证的应用副本。
func Prepare(ctx context.Context, installer, target string) (*Handoff, error) {
	if !strings.HasSuffix(target, ".app") || ApplicationTarget(filepath.Join(target, "Contents", "MacOS", "MailFabrica")) != target {
		return nil, fmt.Errorf("安装目标无效")
	}
	if e := VerifyPublisher(ctx, installer); e != nil {
		return nil, e
	}
	mount, e := os.MkdirTemp("", "mailfabrica-update-mount-*")
	if e != nil {
		return nil, e
	}
	defer os.RemoveAll(mount)
	if e = exec.CommandContext(ctx, "/usr/bin/hdiutil", "attach", "-readonly", "-nobrowse", "-mountpoint", mount, installer).Run(); e != nil {
		return nil, fmt.Errorf("无法挂载安装镜像")
	}
	defer func() {
		detach, cancel := context.WithTimeout(context.Background(), 20*time.Second)
		defer cancel()
		_ = exec.CommandContext(detach, "/usr/bin/hdiutil", "detach", "-quiet", mount).Run()
	}()
	source := filepath.Join(mount, "MailFabrica.app")
	if e = verifyBundle(ctx, source); e != nil {
		return nil, e
	}
	stage, e := os.MkdirTemp(filepath.Dir(target), ".mailfabrica-update-*")
	if e != nil {
		return nil, fmt.Errorf("应用所在目录不可写，请手动安装")
	}
	success := false
	defer func() {
		if !success {
			_ = os.RemoveAll(stage)
		}
	}()
	payload := filepath.Join(stage, "MailFabrica.app")
	if e = exec.CommandContext(ctx, "/usr/bin/ditto", source, payload).Run(); e != nil {
		return nil, fmt.Errorf("无法准备应用副本")
	}
	if e = verifyBundle(ctx, payload); e != nil {
		return nil, e
	}
	nonce := make([]byte, 24)
	if _, e = rand.Read(nonce); e != nil {
		return nil, e
	}
	h := &Handoff{Target: target, Stage: stage, Payload: payload, Backup: target + ".previous-" + filepath.Base(stage), Parent: os.Getpid(), Nonce: hex.EncodeToString(nonce), File: filepath.Join(stage, "handoff.json"), Args: append([]string(nil), os.Args[1:]...)}
	if e = store.WriteJSON(h.File, h); e != nil {
		return nil, e
	}
	success = true
	return h, nil
}

// Start 保留完整签名应用作为辅助副本，避免丢失 Info.plist 签名上下文而被 macOS 拒绝执行。
func (h *Handoff) Start(executable string) error {
	bundle := ApplicationTarget(executable)
	if bundle == "" {
		return fmt.Errorf("更新辅助进程需要完整应用包")
	}
	helperBundle := filepath.Join(h.Stage, "helper.app")
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	if e := exec.CommandContext(ctx, "/usr/bin/ditto", bundle, helperBundle).Run(); e != nil {
		return fmt.Errorf("无法准备已签名的更新辅助应用: %w", e)
	}
	helper := filepath.Join(helperBundle, "Contents", "MacOS", filepath.Base(executable))
	log, e := os.OpenFile(filepath.Join(h.Stage, "helper.log"), os.O_CREATE|os.O_WRONLY, 0600)
	if e != nil {
		return e
	}
	defer log.Close()
	command := exec.Command(helper, "--apply-update", h.File)
	command.Stdout = log
	command.Stderr = log
	if e = command.Start(); e != nil {
		return e
	}
	return command.Process.Release()
}

// Commit 仅在宿主正常退出后写入明确交接凭据。
func (h *Handoff) Commit() error {
	return store.WriteAtomic(filepath.Join(h.Stage, "commit"), []byte(h.Nonce))
}

// Cancel 取消未提交安装，不改变当前应用。
func (h *Handoff) Cancel() error {
	if _, e := os.Stat(filepath.Join(h.Stage, "commit")); e == nil {
		return fmt.Errorf("安装已交接")
	}
	return store.WriteAtomic(filepath.Join(h.Stage, "cancel"), []byte("cancel"))
}

// RunHelper 等待父进程正常退出和提交标记，完成同卷替换；失败时恢复旧包。
func RunHelper(file string) error {
	var h Handoff
	if e := store.ReadJSON(file, &h); e != nil {
		return e
	}
	h.File = file
	if filepath.Clean(file) != filepath.Join(h.Stage, "handoff.json") || filepath.Dir(h.Stage) != filepath.Dir(h.Target) || h.Payload != filepath.Join(h.Stage, "MailFabrica.app") || h.Backup != h.Target+".previous-"+filepath.Base(h.Stage) || !strings.HasPrefix(filepath.Base(h.Stage), ".mailfabrica-update-") || len(h.Nonce) != 48 || h.Parent < 1 {
		return fmt.Errorf("更新交接无效")
	}
	deadline := time.Now().Add(2 * time.Minute)
	for time.Now().Before(deadline) {
		if _, e := os.Stat(filepath.Join(h.Stage, "cancel")); e == nil {
			return os.RemoveAll(h.Stage)
		}
		if e := syscall.Kill(h.Parent, 0); e != nil {
			break
		}
		time.Sleep(100 * time.Millisecond)
	}
	if syscall.Kill(h.Parent, 0) == nil {
		return fmt.Errorf("原应用尚未退出，未替换")
	}
	commit, e := os.ReadFile(filepath.Join(h.Stage, "commit"))
	if e != nil || string(commit) != h.Nonce {
		return fmt.Errorf("缺少正常退出交接，未替换")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
	defer cancel()
	if e = verifyBundle(ctx, h.Payload); e != nil {
		return e
	}
	if _, e = os.Lstat(h.Backup); !os.IsNotExist(e) {
		return fmt.Errorf("备份路径已存在")
	}
	if e = replaceApplication(h.Payload, h.Target, h.Backup); e != nil {
		return e
	}
	args := append([]string{"-n", h.Target, "--args"}, h.Args...)
	if e = exec.CommandContext(ctx, "/usr/bin/open", args...).Run(); e != nil {
		_ = os.Rename(h.Target, h.Payload)
		_ = os.Rename(h.Backup, h.Target)
		return fmt.Errorf("新应用启动失败，已尝试恢复旧包")
	}
	return nil
}

// AcknowledgeStartup 在新应用正常启动后清理自己的更新备份与临时目录。
func AcknowledgeStartup(executable string) error {
	target := ApplicationTarget(executable)
	if target == "" {
		return nil
	}
	entries, e := os.ReadDir(filepath.Dir(target))
	if e != nil {
		return e
	}
	for _, entry := range entries {
		if !entry.IsDir() || !strings.HasPrefix(entry.Name(), ".mailfabrica-update-") {
			continue
		}
		stage := filepath.Join(filepath.Dir(target), entry.Name())
		var h Handoff
		if store.ReadJSON(filepath.Join(stage, "handoff.json"), &h) != nil || h.Target != target || h.Stage != stage {
			continue
		}
		commit, e := os.ReadFile(filepath.Join(stage, "commit"))
		if e != nil || string(commit) != h.Nonce {
			continue
		}
		if _, e = os.Stat(h.Payload); !os.IsNotExist(e) {
			continue
		}
		if h.Backup != target+".previous-"+entry.Name() {
			continue
		}
		if _, e = os.Stat(h.Backup); os.IsNotExist(e) {
			continue
		}
		if e = os.RemoveAll(h.Backup); e != nil {
			return e
		}
		if e = os.RemoveAll(stage); e != nil {
			return e
		}
	}
	return nil
}

// replaceApplication 在相同文件系统内替换应用；第二次改名失败时恢复原应用。
func replaceApplication(payload, target, backup string) error {
	if _, e := os.Lstat(backup); !os.IsNotExist(e) {
		return fmt.Errorf("更新备份路径已存在")
	}
	if e := os.Rename(target, backup); e != nil {
		return e
	}
	if e := os.Rename(payload, target); e != nil {
		if rollback := os.Rename(backup, target); rollback != nil {
			return fmt.Errorf("替换失败且恢复旧包失败: %w", rollback)
		}
		return e
	}
	return nil
}
