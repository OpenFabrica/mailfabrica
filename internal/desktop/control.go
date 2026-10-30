package desktop

import (
	"crypto/rand"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"github.com/openfabrica/mailfabrica/internal/store"
	"strconv"
	"strings"
	"time"
)

// uiResult 是固定 UI 操作的异步结果，不接受任意脚本。
type uiResult struct {
	Value json.RawMessage
	Error string
}

// UI 将有限控制请求交给当前主窗口的前端适配层。
func (s *Service) UI(method string, params json.RawMessage) (any, error) {
	allowed := map[string]bool{"ui.inspect": true, "ui.screenshot": true, "ui.click": true, "ui.drag": true, "ui.key": true, "ui.text": true, "ui.calendar": true, "ui.readerReload": true, "ui.readerScroll": true, "ui.readerSelect": true, "ui.readerCopy": true, "ui.readerHitTest": true, "ui.readerMode": true, "ui.readerActivateLink": true}
	if !allowed[method] {
		return nil, fmt.Errorf("未知 UI 控制方法")
	}
	if s.App == nil {
		return nil, fmt.Errorf("当前没有原生窗口")
	}
	idBytes := make([]byte, 16)
	if _, e := rand.Read(idBytes); e != nil {
		return nil, e
	}
	id := hex.EncodeToString(idBytes)
	ch := make(chan uiResult, 1)
	s.mu.Lock()
	if s.pending == nil {
		s.pending = map[string]chan uiResult{}
	}
	s.pending[id] = ch
	s.mu.Unlock()
	defer func() { s.mu.Lock(); delete(s.pending, id); s.mu.Unlock() }()
	if len(params) == 0 {
		params = json.RawMessage(`{}`)
	}
	var target struct {
		Window string `json:"window"`
	}
	if err := json.Unmarshal(params, &target); err != nil {
		return nil, err
	}
	if target.Window == "" {
		target.Window = "main"
	}
	if target.Window != "main" {
		if !strings.HasPrefix(target.Window, "compose-") {
			return nil, fmt.Errorf("窗口名称无效")
		}
		if _, err := strconv.ParseUint(strings.TrimPrefix(target.Window, "compose-"), 10, 64); err != nil {
			return nil, fmt.Errorf("窗口名称无效")
		}
	}
	s.App.Event.Emit("mailfabrica:control", map[string]any{"id": id, "method": method, "params": params, "window": target.Window})
	select {
	case r := <-ch:
		if r.Error != "" {
			return nil, fmt.Errorf("%s", r.Error)
		}
		if method == "ui.screenshot" {
			var result struct {
				Image   string `json:"base64"`
				Backend string `json:"backend"`
			}
			if e := json.Unmarshal(r.Value, &result); e != nil {
				return nil, e
			}
			var p struct {
				Path string `json:"path"`
			}
			if e := json.Unmarshal(params, &p); e != nil {
				return nil, e
			}
			if p.Path != "" {
				data, e := base64.StdEncoding.DecodeString(result.Image)
				if e != nil {
					return nil, e
				}
				if len(data) > 40<<20 {
					return nil, fmt.Errorf("截图过大")
				}
				if e = store.WriteAtomic(p.Path, data); e != nil {
					return nil, e
				}
				return map[string]any{"path": p.Path, "backend": result.Backend, "includesEmailBody": false}, nil
			}
		}
		return r.Value, nil
	case <-time.After(20 * time.Second):
		return nil, fmt.Errorf("窗口未响应 UI 操作，请确认窗口已加载")
	}
}

// UIResult 仅接收当前宿主发起且未超时的请求结果。
func (s *Service) UIResult(id, result, errorMessage string) (string, error) {
	if len(result) > 60<<20 || !json.Valid([]byte(result)) {
		return "", fmt.Errorf("UI 结果无效")
	}
	s.mu.Lock()
	ch, ok := s.pending[id]
	s.mu.Unlock()
	if !ok {
		return "", fmt.Errorf("UI 请求已过期")
	}
	select {
	case ch <- uiResult{Value: json.RawMessage(result), Error: errorMessage}:
	default:
		return "", fmt.Errorf("UI 请求已完成")
	}
	return "null", nil
}

// CopyText 为固定阅读复制操作写入系统剪贴板。
func (s *Service) CopyText(text string) (string, error) {
	if s.App == nil || len(text) > 8<<20 {
		return "", fmt.Errorf("剪贴板操作无效")
	}
	s.App.Clipboard.SetText(text)
	return "null", nil
}
