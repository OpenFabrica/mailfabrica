// Package desktop 装配 Wails 服务，不实现邮件业务规则。
package desktop

import (
	"encoding/json"
	"fmt"
	"github.com/openfabrica/mailfabrica/internal/engine"
	"github.com/openfabrica/mailfabrica/internal/update"
	"github.com/wailsapp/wails/v3/pkg/application"
	"log/slog"
	"net/url"
	"os"
	"sync"
)

// Service 是前端调用的稳定桥接边界。
type Service struct {
	startup sync.Once
	mu      sync.Mutex
	pending map[string]chan uiResult
	Session *engine.Session
	App     *application.App
}

// Execute 将 JSON 字符串参数交给唯一命令引擎。
func (s *Service) Execute(command, params string) (string, error) {
	result, e := s.Session.Execute(command, json.RawMessage(params))
	if e != nil {
		return "", e
	}
	return string(result), nil
}

// Snapshot 返回多窗口共用的不可变状态快照。
func (s *Service) Snapshot() (string, error) {
	if s.App != nil {
		s.startup.Do(func() {
			executable, e := os.Executable()
			if e == nil {
				if err := update.AcknowledgeStartup(executable); err != nil {
					slog.Warn("更新备份清理未完成", "error", err)
				}
			}
		})
	}
	return string(s.Session.Snapshot()), nil
}

// PickFile 只执行宿主文件对话框，具体保存与导出仍由命令完成。
func (s *Service) PickFile(mode, name string) (string, error) {
	if s.App == nil {
		return "", fmt.Errorf("当前入口没有原生对话框")
	}
	switch mode {
	case "open":
		path, e := s.App.Dialog.OpenFile().PromptForSingleSelection()
		if e != nil {
			return "", e
		}
		b, _ := json.Marshal(path)
		return string(b), nil
	case "directory":
		path, e := s.App.Dialog.OpenFile().CanChooseFiles(false).CanChooseDirectories(true).PromptForSingleSelection()
		if e != nil {
			return "", e
		}
		b, _ := json.Marshal(path)
		return string(b), nil
	case "save":
		path, e := s.App.Dialog.SaveFile().SetFilename(name).PromptForSingleSelection()
		if e != nil {
			return "", e
		}
		b, _ := json.Marshal(path)
		return string(b), nil
	}
	return "", fmt.Errorf("文件对话框类型无效")
}

// OpenExternal 校验明确的用户外链操作，禁止本地文件和活动协议。
func (s *Service) OpenExternal(raw string) (string, error) {
	u, e := url.Parse(raw)
	if e != nil || (u.Scheme != "http" && u.Scheme != "https" && u.Scheme != "mailto") {
		return "", fmt.Errorf("链接协议不支持")
	}
	if (u.Scheme == "http" || u.Scheme == "https") && u.Hostname() == "" {
		return "", fmt.Errorf("链接地址无效")
	}
	if s.App == nil {
		return "", fmt.Errorf("当前入口没有系统浏览器")
	}
	if e = s.App.Browser.OpenURL(raw); e != nil {
		return "", e
	}
	return "null", nil
}
