// Package control 提供兼容 JSON 行控制通道与浏览器验收接口。
package control

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"github.com/openfabrica/mailfabrica/internal/engine"
	"io"
	"net"
	"net/http"
	"strings"
	"time"
)

// Request 与旧版 JSON 行协议保持一致。
type Request struct {
	ID     any             `json:"id"`
	Method string          `json:"method"`
	Params json.RawMessage `json:"params"`
}

// Response 不向调用方返回内部凭据。
type Response struct {
	ID     any    `json:"id"`
	OK     bool   `json:"ok"`
	Result any    `json:"result,omitempty"`
	Error  string `json:"error,omitempty"`
}

// UIHandler 将有限的 UI 操作交给桌面宿主，不暴露任意脚本执行。
type UIHandler func(string, json.RawMessage) (any, error)

// Handle 分发既有引擎、控制方法或直接命令 id。
func Handle(s *engine.Session, r Request, ui UIHandler) Response {
	out := Response{ID: r.ID, OK: true}
	var v any
	var err error
	switch r.Method {
	case "engine.commands":
		var p struct {
			Search string `json:"search"`
		}
		err = json.Unmarshal(defaultObject(r.Params), &p)
		if err == nil {
			v = s.Commands(p.Search)
		}
	case "engine.journal":
		v = s.Journal()
	case "engine.execute":
		var p struct {
			Command string          `json:"command"`
			Params  json.RawMessage `json:"params"`
		}
		if err = json.Unmarshal(defaultObject(r.Params), &p); err == nil {
			v, err = s.Execute(p.Command, p.Params)
		}
	case "ui.inspect":
		if ui != nil {
			v, err = ui(r.Method, r.Params)
		} else {
			v = s.Snapshot()
		}
	default:
		if strings.HasPrefix(r.Method, "ui.") || r.Method == "app.quit" {
			if ui == nil {
				err = fmt.Errorf("该入口没有桌面界面")
			} else {
				v, err = ui(r.Method, r.Params)
			}
		} else {
			v, err = s.Execute(r.Method, r.Params)
		}
	}
	if err != nil {
		out.OK = false
		out.Error = err.Error()
	} else {
		out.Result = v
	}
	return out
}

// defaultObject 为空参数提供对象，拒绝其他格式由下游负责。
func defaultObject(raw json.RawMessage) json.RawMessage {
	if len(raw) == 0 || string(raw) == "null" {
		return json.RawMessage(`{}`)
	}
	return raw
}

// Serve 启动仅监听回环地址的有界 JSON 行服务，取消时关闭全部连接。
func Serve(ctx context.Context, s *engine.Session, port int, ui UIHandler) (net.Listener, error) {
	if port < 1 || port > 65535 {
		return nil, fmt.Errorf("控制端口无效")
	}
	listener, e := net.Listen("tcp", fmt.Sprintf("127.0.0.1:%d", port))
	if e != nil {
		return nil, e
	}
	go func() { <-ctx.Done(); _ = listener.Close() }()
	go func() {
		for {
			conn, e := listener.Accept()
			if e != nil {
				return
			}
			go func() {
				defer conn.Close()
				stop := context.AfterFunc(ctx, func() { _ = conn.Close() })
				defer stop()
				scanner := bufio.NewScanner(conn)
				scanner.Buffer(make([]byte, 4096), 4<<20)
				encoder := json.NewEncoder(conn)
				for {
					_ = conn.SetReadDeadline(time.Now().Add(5 * time.Minute))
					if !scanner.Scan() {
						return
					}
					var r Request
					if e := json.Unmarshal(scanner.Bytes(), &r); e != nil {
						if encoder.Encode(Response{OK: false, Error: "无效 JSON 请求"}) != nil {
							return
						}
						continue
					}
					_ = conn.SetWriteDeadline(time.Now().Add(20 * time.Second))
					if e := encoder.Encode(Handle(s, r, ui)); e != nil {
						return
					}
				}
			}()
		}
	}()
	return listener, nil
}

// HTTPService 是浏览器模式与 Wails 服务共用的最小接口。
type HTTPService interface {
	Execute(string, string) (string, error)
	Snapshot() (string, error)
}

// HTTPHandler 为同源浏览器提供真实会话 API；拒绝跨站请求和任意方法调用。
func HTTPHandler(service HTTPService) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		if r.Method != "POST" {
			http.Error(w, `{"error":"POST required"}`, 405)
			return
		}
		if site := r.Header.Get("Sec-Fetch-Site"); site == "cross-site" {
			http.Error(w, `{"error":"cross-site request rejected"}`, 403)
			return
		}
		if origin := r.Header.Get("Origin"); origin != "" && origin != "http://"+r.Host {
			http.Error(w, `{"error":"origin mismatch"}`, 403)
			return
		}
		r.Body = http.MaxBytesReader(w, r.Body, 75<<20)
		var args []json.RawMessage
		var result string
		var err error
		if err = json.NewDecoder(r.Body).Decode(&args); err == nil {
			switch strings.TrimPrefix(r.URL.Path, "/api/") {
			case "Snapshot":
				result, err = service.Snapshot()
			case "Execute":
				if len(args) != 2 {
					err = fmt.Errorf("参数数量无效")
					break
				}
				var command, params string
				if err = json.Unmarshal(args[0], &command); err == nil {
					err = json.Unmarshal(args[1], &params)
				}
				if err == nil {
					result, err = service.Execute(command, params)
				}
			case "PickFile":
				err = fmt.Errorf("浏览器验收模式无法打开原生路径选择器，请输入保存路径或使用桌面程序")
			default:
				err = fmt.Errorf("未知接口")
			}
		}
		if err != nil {
			w.WriteHeader(400)
			_ = json.NewEncoder(w).Encode(map[string]string{"error": err.Error()})
			return
		}
		if !json.Valid([]byte(result)) {
			result = "null"
		}
		_, _ = io.WriteString(w, `{"result":`+result+`}`)
	})
}
