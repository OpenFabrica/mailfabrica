package control

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"github.com/openfabrica/mailfabrica/internal/engine"
	"io"
	"net"
	"strconv"
	"strings"
	"time"
)

// Backend 使 MCP 可选择无窗口会话或现有桌面控制通道。
type Backend struct {
	Session *engine.Session
	Port    int
}

// Call 使用同一稳定 JSON 行协议，并在无窗口入口等待后台任务。
func (b Backend) Call(method string, p any) (json.RawMessage, error) {
	raw, e := json.Marshal(p)
	if e != nil {
		return nil, e
	}
	request := Request{ID: 1, Method: method, Params: raw}
	if b.Port != 0 {
		conn, e := net.DialTimeout("tcp", net.JoinHostPort("127.0.0.1", strconv.Itoa(b.Port)), 5*time.Second)
		if e != nil {
			return nil, e
		}
		defer conn.Close()
		_ = conn.SetDeadline(time.Now().Add(150 * time.Second))
		if e = json.NewEncoder(conn).Encode(request); e != nil {
			return nil, e
		}
		var response struct {
			OK     bool            `json:"ok"`
			Result json.RawMessage `json:"result"`
			Error  string          `json:"error"`
		}
		if e = json.NewDecoder(io.LimitReader(conn, 100<<20)).Decode(&response); e != nil {
			return nil, e
		}
		if !response.OK {
			return nil, fmt.Errorf("%s", response.Error)
		}
		return response.Result, nil
	}
	if b.Session == nil {
		return nil, fmt.Errorf("引擎会话不存在")
	}
	var before struct {
		Revision uint64 `json:"revision"`
	}
	_ = json.Unmarshal(b.Session.Snapshot(), &before)
	response := Handle(b.Session, request, nil)
	if !response.OK {
		return nil, fmt.Errorf("%s", response.Error)
	}
	ctx, cancel := context.WithTimeout(context.Background(), 150*time.Second)
	defer cancel()
	if e = b.Session.Wait(ctx); e != nil {
		return nil, e
	}
	var status struct {
		Status struct {
			Errors        [][2]string `json:"errors"`
			ErrorRevision uint64      `json:"errorRevision"`
		} `json:"status"`
	}
	if e = json.Unmarshal(b.Session.Snapshot(), &status); e != nil {
		return nil, e
	}
	if method != "engine.commands" && method != "engine.journal" && method != "app.status" && len(status.Status.Errors) > 0 && status.Status.ErrorRevision > before.Revision {
		return nil, fmt.Errorf("%s", status.Status.Errors[len(status.Status.Errors)-1][1])
	}
	if r, ok := response.Result.(json.RawMessage); ok {
		return r, nil
	}
	return json.Marshal(response.Result)
}

// Execute 封装 engine.execute 参数。
func (b Backend) Execute(id string, p any) (json.RawMessage, error) {
	if b.Port == 0 {
		return b.Call("engine.execute", map[string]any{"command": id, "params": p})
	}
	var before struct {
		Revision uint64 `json:"revision"`
	}
	raw, _ := b.Call("app.status", map[string]any{})
	_ = json.Unmarshal(raw, &before)
	result, e := b.Call("engine.execute", map[string]any{"command": id, "params": p})
	if e != nil {
		return nil, e
	}
	wait := map[string]bool{"account.add": true, "mail.sync": true, "message.open": true, "message.markRead": true, "message.toggleRead": true, "message.flag": true, "message.move": true, "message.delete": true, "message.junk": true, "folder.create": true, "folder.markAllRead": true, "account.markAllRead": true, "ai.scanJunk": true, "ai.cleanJunk": true, "compose.send": true, "compose.saveDraft": true, "ai.compose": true, "ai.quickReply": true, "invoice.export": true, "calendar.load": true, "update.check": true, "update.download": true, "update.open": true}
	if !wait[id] {
		return result, nil
	}
	deadline := time.Now().Add(150 * time.Second)
	for time.Now().Before(deadline) {
		statusRaw, err := b.Call("app.status", map[string]any{})
		if err != nil {
			return nil, err
		}
		var state struct {
			Busy          bool        `json:"busy"`
			ErrorRevision uint64      `json:"errorRevision"`
			Errors        [][2]string `json:"errors"`
		}
		if err = json.Unmarshal(statusRaw, &state); err != nil {
			return nil, err
		}
		if !state.Busy {
			if state.ErrorRevision > before.Revision && len(state.Errors) > 0 {
				return nil, fmt.Errorf("%s", state.Errors[len(state.Errors)-1][1])
			}
			return result, nil
		}
		time.Sleep(50 * time.Millisecond)
	}
	return nil, fmt.Errorf("等待后台任务超时")
}

// toolSchema 构造不限制命令参数字段的 MCP 工具定义。
func toolSchema(name, description string, properties map[string]any, required []string) map[string]any {
	return map[string]any{"name": name, "description": description, "inputSchema": map[string]any{"type": "object", "properties": properties, "required": required}}
}

// tools 保留旧版公开的邮件工具名称与通用命令执行能力。
func (b Backend) tools() []map[string]any {
	stringProp := map[string]any{"type": "string"}
	props := map[string]any{"account": stringProp, "folder": stringProp, "key": stringProp, "query": stringProp, "subject": stringProp, "body": stringProp, "to": map[string]any{"type": []string{"string", "array"}, "items": stringProp}, "cc": map[string]any{"type": []string{"string", "array"}, "items": stringProp}, "bcc": stringProp, "limit": map[string]any{"type": "integer"}, "unreadOnly": map[string]any{"type": "boolean"}, "group": map[string]any{"type": "boolean"}, "attachments": map[string]any{"type": "array", "items": stringProp}}
	out := []map[string]any{toolSchema("mail_execute", "执行任意 MailFabrica 命令；使用 mail_commands 查看完整参数。", map[string]any{"command": stringProp, "params": map[string]any{"type": "object"}}, []string{"command"}), toolSchema("mail_commands", "查询命令目录及可用性。", map[string]any{"search": stringProp}, []string{})}
	for name, description := range map[string]string{"mail_accounts": "列出账户，不包含凭据。", "mail_folders": "列出文件夹。", "mail_list": "列出邮件。", "mail_read": "下载并阅读邮件正文。", "mail_search": "搜索本地邮件。", "mail_send": "发送邮件，收件人和正文必须由调用方提供。", "mail_status": "查询后台状态。"} {
		out = append(out, toolSchema(name, description, props, []string{}))
	}
	if b.Port != 0 {
		out = append(out, toolSchema("ui_inspect", "读取桌面状态。", map[string]any{}, []string{}), toolSchema("ui_screenshot", "截取桌面窗口。", map[string]any{"path": stringProp}, []string{}))
	}
	return out
}

// callTool 将友好工具参数归一化后转发已有命令。
func (b Backend) callTool(name string, p map[string]any) (json.RawMessage, error) {
	switch name {
	case "mail_execute":
		id, ok := p["command"].(string)
		if !ok {
			return nil, fmt.Errorf("command 必须是字符串")
		}
		params := p["params"]
		if params == nil {
			params = map[string]any{}
		}
		return b.Execute(id, params)
	case "mail_commands":
		return b.Call("engine.commands", p)
	case "mail_accounts":
		return b.Execute("account.list", p)
	case "mail_folders":
		return b.Execute("folder.list", p)
	case "mail_list":
		if _, ok := p["limit"]; !ok {
			p["limit"] = 50
		}
		raw, e := b.Execute("message.list", p)
		return normalizeKeys(raw), e
	case "mail_status":
		return b.Execute("app.status", p)
	case "mail_read":
		if _, e := b.Execute("view.select", p); e != nil {
			return nil, e
		}
		raw, e := b.Execute("message.get", p)
		return normalizeKeys(raw), e
	case "mail_search":
		if _, e := b.Execute("view.search", p); e != nil {
			return nil, e
		}
		raw, e := b.Execute("message.list", p)
		return normalizeKeys(raw), e
	case "mail_send":
		for _, key := range []string{"to", "cc", "bcc"} {
			if list, ok := p[key].([]any); ok {
				parts := []string{}
				for _, v := range list {
					part, ok := v.(string)
					if !ok {
						return nil, fmt.Errorf("%s 必须是字符串或字符串数组", key)
					}
					parts = append(parts, part)
				}
				p[key] = strings.Join(parts, ", ")
			}
		}
		for _, key := range []string{"to", "subject", "body"} {
			if _, ok := p[key].(string); !ok {
				return nil, fmt.Errorf("缺少 %s 字符串参数", key)
			}
		}
		raw, e := b.Execute("compose.new", p)
		if e != nil {
			return nil, e
		}
		var c struct {
			ID uint64 `json:"id"`
		}
		if e = json.Unmarshal(raw, &c); e != nil {
			return nil, e
		}
		if paths, ok := p["attachments"].([]any); ok {
			for _, path := range paths {
				if _, e = b.Execute("compose.attach", map[string]any{"id": c.ID, "path": path}); e != nil {
					return nil, e
				}
			}
		}
		if _, e = b.Execute("compose.setFormat", map[string]any{"id": c.ID, "rich": false}); e != nil {
			return nil, e
		}
		if _, e = b.Execute("compose.set", map[string]any{"id": c.ID, "body": p["body"]}); e != nil {
			return nil, e
		}
		if _, e = b.Execute("compose.send", map[string]any{"id": c.ID}); e != nil {
			_, _ = b.Execute("compose.discard", map[string]any{"id": c.ID})
			return nil, e
		}
		status, e := b.Execute("app.status", map[string]any{})
		if e != nil {
			return nil, e
		}
		return json.Marshal(map[string]any{"sent": true, "to": p["to"], "subject": p["subject"], "status": status})
	case "ui_inspect":
		return b.Call("ui.inspect", p)
	case "ui_screenshot":
		return b.Call("ui.screenshot", p)
	}
	return nil, fmt.Errorf("未知 MCP 工具")
}

// ServeMCP 按 JSON 行处理 stdio，协议日志不写 stdout。
func ServeMCP(in io.Reader, out io.Writer, b Backend) error {
	scanner := bufio.NewScanner(in)
	scanner.Buffer(make([]byte, 4096), 75<<20)
	encoder := json.NewEncoder(out)
	for scanner.Scan() {
		var request struct {
			JSONRPC string          `json:"jsonrpc"`
			ID      json.RawMessage `json:"id"`
			Method  string          `json:"method"`
			Params  json.RawMessage `json:"params"`
		}
		if e := json.Unmarshal(scanner.Bytes(), &request); e != nil {
			if e = encoder.Encode(map[string]any{"jsonrpc": "2.0", "id": nil, "error": map[string]any{"code": -32700, "message": "Parse error"}}); e != nil {
				return e
			}
			continue
		}
		if len(request.ID) == 0 {
			continue
		}
		var result any
		var err error
		switch request.Method {
		case "initialize":
			result = map[string]any{"protocolVersion": "2024-11-05", "capabilities": map[string]any{"tools": map[string]any{}}, "serverInfo": map[string]any{"name": "mailfabrica", "version": engine.Version}}
		case "ping":
			result = map[string]any{}
		case "tools/list":
			result = map[string]any{"tools": b.tools()}
		case "tools/call":
			var p struct {
				Name      string         `json:"name"`
				Arguments map[string]any `json:"arguments"`
			}
			err = json.Unmarshal(request.Params, &p)
			if err == nil {
				raw, e := b.callTool(p.Name, p.Arguments)
				text := strings.TrimSpace(string(raw))
				if e != nil {
					text = e.Error()
				}
				result = map[string]any{"content": []any{map[string]any{"type": "text", "text": text}}, "isError": e != nil}
			}
		default:
			err = fmt.Errorf("Method not found")
		}
		response := map[string]any{"jsonrpc": "2.0", "id": request.ID}
		if err != nil {
			response["error"] = map[string]any{"code": -32601, "message": err.Error()}
		} else {
			response["result"] = result
		}
		if e := encoder.Encode(response); e != nil {
			return e
		}
	}
	return scanner.Err()
}

// normalizeKeys 保留友好 MCP 邮件工具中 key 为字符串的既有契约。
func normalizeKeys(raw json.RawMessage) json.RawMessage {
	var value any
	if json.Unmarshal(raw, &value) != nil {
		return raw
	}
	var walk func(any)
	walk = func(v any) {
		switch node := v.(type) {
		case map[string]any:
			if key, ok := node["key"].(map[string]any); ok {
				a, aok := key["account"].(string)
				f, fok := key["folder"].(string)
				uid, uok := key["uid"].(float64)
				if aok && fok && uok {
					node["key"] = a + "/" + strconv.FormatUint(uint64(uid), 10) + "/" + f
				}
			}
			for _, child := range node {
				walk(child)
			}
		case []any:
			for _, child := range node {
				walk(child)
			}
		}
	}
	walk(value)
	b, e := json.Marshal(value)
	if e != nil {
		return raw
	}
	return b
}
