// mailfabrica-cli 提供不依赖桌面框架的命令行及 MCP 入口。
package main

import (
	"bufio"
	"encoding/json"
	"fmt"
	"github.com/openfabrica/mailfabrica/internal/control"
	"github.com/openfabrica/mailfabrica/internal/engine"
	"io"
	"os"
	"strconv"
	"strings"
)

// options 保留可出现在任意位置的全局选项。
type options struct {
	demo   bool
	data   string
	port   int
	args   []string
	values map[string][]string
	flags  map[string]bool
}

// parse 将全局选项和子命令选项归一化，未知选项明确报错。
func parse(args []string) (options, error) {
	o := options{values: map[string][]string{}, flags: map[string]bool{}}
	bools := map[string]bool{"json": true, "unread": true, "flat": true, "password-stdin": true, "no-verify": true}
	values := map[string]bool{"account": true, "folder": true, "limit": true, "offset": true, "search": true, "to": true, "cc": true, "bcc": true, "subject": true, "body": true, "body-file": true, "attach": true, "password": true, "name": true, "login": true, "imap": true, "smtp": true}
	for i := 0; i < len(args); i++ {
		arg := args[i]
		if arg == "--demo" {
			o.demo = true
			continue
		}
		if !strings.HasPrefix(arg, "--") {
			o.args = append(o.args, arg)
			continue
		}
		key := strings.TrimPrefix(arg, "--")
		if bools[key] {
			o.flags[key] = true
			continue
		}
		if key != "data" && key != "control" && !values[key] {
			return o, fmt.Errorf("未知选项: %s", arg)
		}
		i++
		if i >= len(args) {
			return o, fmt.Errorf("%s 缺少值", arg)
		}
		v := args[i]
		switch key {
		case "data":
			o.data = v
		case "control":
			n, e := strconv.Atoi(v)
			if e != nil || n < 1 || n > 65535 {
				return o, fmt.Errorf("控制端口无效")
			}
			o.port = n
		default:
			o.values[key] = append(o.values[key], v)
		}
	}
	return o, nil
}

// main 让失败信息进入 stderr 并使用非零退出码。
func main() {
	if e := run(os.Args[1:]); e != nil {
		fmt.Fprintln(os.Stderr, e)
		os.Exit(1)
	}
}

// run 通过通用后端完成所有命令，不另建业务状态。
func run(args []string) error {
	o, e := parse(args)
	if e != nil {
		return e
	}
	if len(o.args) == 0 {
		fmt.Println("mailfabrica-cli [--demo] [--data DIR] accounts|account|sync|folders|list|read|search|send|commands|exec|mcp|version")
		return nil
	}
	if o.args[0] == "version" {
		fmt.Println("mailfabrica-cli " + engine.Version)
		return nil
	}
	backend := control.Backend{Port: o.port}
	if o.port == 0 {
		root, e := engine.DataRoot(o.data)
		if e != nil {
			return e
		}
		if o.demo && o.data == "" {
			root, e = os.MkdirTemp("", "mailfabrica-cli-demo-*")
			if e != nil {
				return e
			}
			defer os.RemoveAll(root)
		}
		s, e := engine.New(root, o.demo)
		if e != nil {
			return e
		}
		defer s.Close()
		backend.Session = s
	}
	if o.args[0] == "mcp" {
		return control.ServeMCP(os.Stdin, os.Stdout, backend)
	}
	value := func(key string) string {
		v := o.values[key]
		if len(v) > 0 {
			return v[len(v)-1]
		}
		return ""
	}
	p := map[string]any{}
	for k, field := range map[string]string{"account": "account", "folder": "folder", "search": "search"} {
		if v := value(k); v != "" {
			p[field] = v
		}
	}
	for _, key := range []string{"limit", "offset"} {
		if v := value(key); v != "" {
			n, e := strconv.Atoi(v)
			if e != nil || n < 0 {
				return fmt.Errorf("%s 无效", key)
			}
			p[key] = n
		}
	}
	var result json.RawMessage
	command := o.args[0]
	switch command {
	case "accounts":
		result, e = backend.Execute("account.list", p)
	case "commands":
		result, e = backend.Call("engine.commands", p)
	case "sync":
		if len(o.args) > 1 {
			p["account"] = o.args[1]
		}
		result, e = backend.Execute("mail.sync", p)
	case "folders":
		if len(o.args) < 2 {
			return fmt.Errorf("folders 需要账户 id")
		}
		p["account"] = o.args[1]
		result, e = backend.Execute("folder.list", p)
	case "list":
		p["unreadOnly"] = o.flags["unread"]
		p["group"] = !o.flags["flat"]
		result, e = backend.Execute("message.list", p)
	case "read":
		if len(o.args) < 2 {
			return fmt.Errorf("read 需要邮件键")
		}
		p["key"] = o.args[1]
		if _, e = backend.Execute("view.select", p); e == nil {
			result, e = backend.Execute("message.get", p)
		}
	case "search":
		if len(o.args) < 2 {
			return fmt.Errorf("search 需要搜索词")
		}
		if _, e = backend.Execute("view.search", map[string]any{"query": o.args[1]}); e == nil {
			result, e = backend.Execute("message.list", p)
		}
	case "exec":
		if len(o.args) < 2 {
			return fmt.Errorf("exec 需要命令 id")
		}
		if len(o.args) > 2 {
			if e = json.Unmarshal([]byte(o.args[2]), &p); e != nil {
				return e
			}
		}
		result, e = backend.Execute(o.args[1], p)
	case "send":
		for _, key := range []string{"to", "cc", "bcc"} {
			p[key] = strings.Join(o.values[key], ", ")
		}
		p["subject"] = value("subject")
		p["body"] = value("body")
		if path := value("body-file"); path != "" {
			var r io.ReadCloser
			if path == "-" {
				r = os.Stdin
			} else {
				r, e = os.Open(path)
				if e != nil {
					return e
				}
				defer r.Close()
			}
			b, err := io.ReadAll(io.LimitReader(r, 10<<20))
			if err != nil {
				return err
			}
			p["body"] = string(b)
		}
		raw, err := backend.Execute("compose.new", p)
		if err != nil {
			return err
		}
		var c struct {
			ID uint64 `json:"id"`
		}
		if e = json.Unmarshal(raw, &c); e != nil {
			return e
		}
		for _, path := range o.values["attach"] {
			if _, e = backend.Execute("compose.attach", map[string]any{"id": c.ID, "path": path}); e != nil {
				return e
			}
		}
		result, e = backend.Execute("compose.send", map[string]any{"id": c.ID})
	case "account":
		if len(o.args) < 3 {
			return fmt.Errorf("account 需要 add EMAIL 或 remove ID")
		}
		if o.args[1] == "remove" {
			result, e = backend.Execute("account.remove", map[string]any{"id": o.args[2]})
			break
		}
		if o.args[1] != "add" {
			return fmt.Errorf("未知 account 子命令")
		}
		p["email"] = o.args[2]
		p["verify"] = !o.flags["no-verify"]
		for _, key := range []string{"name", "login", "password"} {
			if v := value(key); v != "" {
				p[key] = v
			}
		}
		if o.flags["password-stdin"] {
			scanner := bufio.NewScanner(os.Stdin)
			scanner.Buffer(make([]byte, 1024), 64<<10)
			if !scanner.Scan() {
				return fmt.Errorf("无法读取密码")
			}
			p["password"] = scanner.Text()
		}
		for _, prefix := range []string{"imap", "smtp"} {
			if v := value(prefix); v != "" {
				parts := strings.Split(v, ":")
				if len(parts) < 2 || len(parts) > 3 {
					return fmt.Errorf("服务器参数应为 host:port[:security]")
				}
				n, err := strconv.Atoi(parts[1])
				if err != nil || n < 1 || n > 65535 {
					return fmt.Errorf("端口无效")
				}
				p[prefix+"Host"] = parts[0]
				p[prefix+"Port"] = n
				if len(parts) == 3 {
					p[prefix+"Security"] = parts[2]
				}
			}
		}
		result, e = backend.Execute("account.add", p)
	default:
		return fmt.Errorf("未知子命令: %s", command)
	}
	if e != nil {
		return e
	}
	if len(result) == 0 {
		result = json.RawMessage(`null`)
	}
	if !o.flags["json"] && command != "exec" {
		return writeText(os.Stdout, command, result)
	}
	var pretty strings.Builder
	var v any
	if json.Unmarshal(result, &v) == nil {
		b, e := json.MarshalIndent(v, "", "  ")
		if e != nil {
			return e
		}
		pretty.Write(b)
	} else {
		pretty.Write(result)
	}
	fmt.Println(pretty.String())
	return nil
}
