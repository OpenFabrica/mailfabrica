// Package engine 为桌面、CLI、MCP 提供唯一邮件命令入口。
package engine

import (
	"context"
	_ "embed"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/openfabrica/mailfabrica/internal/mail"
	"github.com/openfabrica/mailfabrica/internal/platform"
	"github.com/openfabrica/mailfabrica/internal/protocol"
	"github.com/openfabrica/mailfabrica/internal/store"
	"github.com/openfabrica/mailfabrica/internal/update"
)

//go:embed commands.json
var commandJSON []byte

// Params 是经过 JSON 解码的命令参数。
type Params map[string]any

// Command 保留原命令发现元数据和当前可用性。
type Command struct {
	ID       string   `json:"id"`
	Label    string   `json:"label"`
	Menu     []string `json:"menu"`
	Shortcut *string  `json:"shortcut"`
	Params   string   `json:"params"`
	Query    bool     `json:"query"`
	Gate     string   `json:"gate"`
	Enabled  bool     `json:"enabled"`
	Reason   *string  `json:"reason"`
}

// Scope 对应旧版带 kind 的视图范围。
type Scope struct {
	Kind    string `json:"kind"`
	Account string `json:"account,omitempty"`
	Folder  string `json:"folder,omitempty"`
}

// Row 将同主题邮件合成一行。
type Row struct {
	Group   *string      `json:"group"`
	Count   int          `json:"count"`
	Unread  int          `json:"unread"`
	Flagged bool         `json:"flagged"`
	Keys    []string     `json:"keys"`
	Message mail.Message `json:"message"`
	Senders []string     `json:"senders"`
}

// View 是前端可读取的邮件列表状态。
type View struct {
	Scope    Scope          `json:"scope"`
	Query    string         `json:"query"`
	Messages []mail.Message `json:"messages"`
	Rows     []Row          `json:"rows"`
	Selected []string       `json:"selected"`
	Focused  *string        `json:"focused"`
	Group    *string        `json:"group"`
}

// Status 记录后台工作和可见失败，避免静默丢失任务错误。
type Status struct {
	Composes      int          `json:"composes"`
	Update        *UpdateState `json:"update"`
	ErrorRevision uint64       `json:"errorRevision"`
	Busy          bool         `json:"busy"`
	Text          string       `json:"text"`
	Errors        [][2]string  `json:"errors"`
	LastSync      int64        `json:"lastSync"`
	Revision      uint64       `json:"revision"`
	BusyAccounts  []string     `json:"busyAccounts"`
}

// Quote 将原文与用户编辑内容分离。
type Quote struct {
	HTML   string            `json:"html"`
	Text   string            `json:"text"`
	Inline []mail.Attachment `json:"inline"`
}

// Compose 是独立写信窗口的后端状态。
type Compose struct {
	Background   bool              `json:"background"`
	ID           uint64            `json:"id"`
	Mode         string            `json:"mode"`
	Account      string            `json:"account"`
	To           string            `json:"to"`
	CC           string            `json:"cc"`
	BCC          string            `json:"bcc"`
	Subject      string            `json:"subject"`
	Body         string            `json:"body"`
	HTML         string            `json:"html"`
	Rich         bool              `json:"rich"`
	Quote        *Quote            `json:"quote"`
	Attachments  []mail.Attachment `json:"attachments"`
	Source       *mail.Key         `json:"source"`
	Draft        *mail.Key         `json:"draft"`
	InReplyTo    *string           `json:"inReplyTo"`
	References   []string          `json:"references"`
	Sending      bool              `json:"sending"`
	Error        *string           `json:"error"`
	Dirty        bool              `json:"dirty"`
	AIBusy       bool              `json:"aiBusy"`
	AISuggestion *string           `json:"aiSuggestion"`
	AIError      *string           `json:"aiError"`
	Doc          RichDoc           `json:"doc"`
}

// UIRequest 是界面执行的宿主操作，业务状态仍经命令修改。
type UIRequest struct {
	Kind   string `json:"kind"`
	ID     uint64 `json:"id,omitempty"`
	Name   string `json:"name,omitempty"`
	Params any    `json:"params,omitempty"`
}

// Session 串行处理状态，网络 goroutine 只提交有锁结果。
type Session struct {
	updateTarget     string
	updateExecutable string
	updateHandoff    *update.Handoff
	accountLocks     map[string]*sync.Mutex
	accountBusy      map[string]int
	mu               sync.Mutex
	ctx              context.Context
	cancel           context.CancelFunc
	wg               sync.WaitGroup
	Store            *store.Store
	secrets          *platform.Secrets
	remote           protocol.Remote
	demo             bool
	prefs            Params
	view             View
	status           Status
	commands         []Command
	composes         map[uint64]*Compose
	nextID           uint64
	bodies           map[string]mail.Body
	bodyErrors       map[string]string
	loading          map[string]bool
	busy             map[string]bool
	journal          []any
	requests         []UIRequest
	revision         uint64
	contactIndex     *contactIndex
	aiGroupSummary   *GroupSummary
	aiReply          any
	calendar         CalendarState
	invoice          InvoiceState
	update           UpdateState
}

// defaultPrefs 与 Rust 版持久化默认值保持一致。
func defaultPrefs() Params {
	return Params{"closeToTray": false, "markReadOnOpen": true, "remoteImages": true, "syncIntervalMinutes": float64(10), "syncLimit": float64(500), "prefetch": float64(50), "quoteOnReply": true, "saveSent": true, "chineseQuotes": true, "language": "zh-CN", "composeRich": true, "groupBySubject": true, "mailListDensity": "compact", "theme": "system", "collapsed": []any{}, "calendarHiddenAccounts": []any{}, "calendarHiddenCalendars": []any{}, "sidebarWidth": float64(220), "listWidth": float64(350), "aiProvider": "anthropic", "aiBaseUrl": "", "aiModel": ""}
}

// New 创建独立会话；demo 只使用调用方提供的演示数据目录。
func New(root string, demo bool) (*Session, error) {
	st, e := store.Open(root)
	if e != nil {
		return nil, e
	}
	ctx, cancel := context.WithCancel(context.Background())
	s := &Session{ctx: ctx, cancel: cancel, Store: st, secrets: platform.NewSecrets(root, demo), remote: protocol.Network{}, demo: demo, accountLocks: map[string]*sync.Mutex{}, accountBusy: map[string]int{}, prefs: defaultPrefs(), view: View{Scope: Scope{Kind: "allInboxes"}, Messages: []mail.Message{}, Rows: []Row{}, Selected: []string{}}, status: Status{Text: "就绪", Errors: [][2]string{}, BusyAccounts: []string{}}, composes: map[uint64]*Compose{}, bodies: map[string]mail.Body{}, bodyErrors: map[string]string{}, loading: map[string]bool{}, busy: map[string]bool{}, journal: []any{}, requests: []UIRequest{}}
	if e = json.Unmarshal(commandJSON, &s.commands); e != nil {
		cancel()
		return nil, e
	}
	saved := Params{}
	if e = store.ReadJSON(filepath.Join(root, "prefs.json"), &saved); e != nil {
		cancel()
		return nil, e
	}
	for k, v := range saved {
		s.prefs[k] = v
	}
	legacy := Params{}
	if e = store.ReadJSON(filepath.Join(root, "ui.json"), &legacy); e != nil {
		cancel()
		return nil, e
	}
	for _, k := range []string{"sidebarWidth", "listWidth", "collapsed", "calendarHiddenAccounts", "calendarHiddenCalendars"} {
		if _, exists := saved[k]; !exists {
			if v, ok := legacy[k]; ok {
				s.prefs[k] = v
			}
		}
	}
	if _, exists := saved["theme"]; !exists {
		if dark, ok := legacy["dark"].(bool); ok {
			if dark {
				s.prefs["theme"] = "dark"
			} else {
				s.prefs["theme"] = "light"
			}
		}
	}
	if v, ok := legacy["language"]; ok {
		if _, exists := saved["language"]; !exists {
			s.prefs["language"] = v
		}
	}

	if demo {
		if e = s.seedDemo(); e != nil {
			cancel()
			return nil, e
		}
	}
	if e = s.refresh(); e != nil {
		cancel()
		return nil, e
	}
	s.wg.Add(1)
	go s.autoSync()
	return s, nil
}

// Close 取消外部任务并等待所有后台工作退出。
func (s *Session) Close() { s.cancel(); s.wg.Wait() }

// autoSync 定时检查偏好，避免前端关窗后同步停止。
func (s *Session) autoSync() {
	defer s.wg.Done()
	tick := time.NewTicker(time.Second * 10)
	defer tick.Stop()
	last := time.Now()
	for {
		select {
		case <-s.ctx.Done():
			return
		case <-tick.C:
			s.mu.Lock()
			minutes := number(s.prefs, "syncIntervalMinutes", 10)
			if minutes > 0 && time.Since(last) >= time.Duration(minutes)*time.Minute {
				_, _ = s.startSync("")
				last = time.Now()
			}
			s.mu.Unlock()
		}
	}
}

// str 读取可选字符串。
func str(p Params, k string) string { v, _ := p[k].(string); return v }

// number 读取 JSON 整数参数；参数类型在 Execute 边界校验。
func number(p Params, k string, fallback int) int {
	v, ok := p[k].(float64)
	if !ok {
		return fallback
	}
	return int(v)
}

// boolean 读取可选布尔值。
func boolean(p Params, k string, fallback bool) bool {
	v, ok := p[k].(bool)
	if !ok {
		return fallback
	}
	return v
}

// required 保证命令必需的非空字符串存在。
func required(p Params, k string) (string, error) {
	v, ok := p[k].(string)
	if !ok || strings.TrimSpace(v) == "" {
		return "", fmt.Errorf("缺少参数 %s", k)
	}
	return v, nil
}

// account 查找账户，不返回不存在账户的默认配置。
func (s *Session) account(id string) (mail.Account, error) {
	for _, a := range s.Store.Accounts {
		if a.ID == id {
			return a, nil
		}
	}
	return mail.Account{}, fmt.Errorf("账户不存在: %s", id)
}

// touch 增加快照版本，供所有界面窗口共同观察。
func (s *Session) touch() {
	s.revision++
	s.status.Revision = s.revision
	s.status.Composes = len(s.composes)
	s.status.Update = &s.update
	s.status.Busy = len(s.busy) > 0
	s.status.BusyAccounts = []string{}
	for a := range s.accountBusy {
		s.status.BusyAccounts = append(s.status.BusyAccounts, a)
	}
	sort.Strings(s.status.BusyAccounts)
}

// errStatus 将后台错误纳入可见状态。
func (s *Session) errStatus(id string, e error) {
	if e != nil {
		s.status.Errors = append(s.status.Errors, [2]string{id, e.Error()})
		s.status.ErrorRevision = s.revision + 1
		if len(s.status.Errors) > 100 {
			s.status.Errors = s.status.Errors[len(s.status.Errors)-100:]
		}
		s.status.Text = e.Error()
	}
	s.touch()
}

// launch 在后台运行任务，结束时持锁更新状态。
func (s *Session) launch(id string, work func(context.Context) (any, error), apply func(any) error, accounts ...string) error {
	if s.busy[id] {
		return fmt.Errorf("该任务正在运行")
	}
	s.busy[id] = true
	if s.accountLocks == nil {
		s.accountLocks = map[string]*sync.Mutex{}
	}
	if s.accountBusy == nil {
		s.accountBusy = map[string]int{}
	}
	sort.Strings(accounts)
	locks := []*sync.Mutex{}
	seen := map[string]bool{}
	for _, account := range accounts {
		if seen[account] {
			continue
		}
		seen[account] = true
		lock := s.accountLocks[account]
		if lock == nil {
			lock = &sync.Mutex{}
			s.accountLocks[account] = lock
		}
		locks = append(locks, lock)
		s.accountBusy[account]++
	}
	s.touch()
	s.wg.Add(1)
	go func() {
		defer s.wg.Done()
		for _, lock := range locks {
			lock.Lock()
		}
		defer func() {
			for i := len(locks) - 1; i >= 0; i-- {
				locks[i].Unlock()
			}
		}()
		var value any
		var err error
		func() {
			defer func() {
				if recover() != nil {
					err = fmt.Errorf("后台任务发生内部错误")
				}
			}()
			value, err = work(s.ctx)
		}()
		s.mu.Lock()
		defer s.mu.Unlock()
		delete(s.busy, id)
		for account := range seen {
			s.accountBusy[account]--
			if s.accountBusy[account] == 0 {
				delete(s.accountBusy, account)
			}
		}
		if err == nil && apply != nil {
			err = apply(value)
		}
		if err != nil {
			s.errStatus(id, err)
		} else {
			s.status.Text = "已完成"
			kept := [][2]string{}
			for _, entry := range s.status.Errors {
				if entry[0] != id {
					kept = append(kept, entry)
				}
			}
			s.status.Errors = kept
			s.touch()
		}
	}()
	return nil
}

// enabled 统一控制界面和自动化的可执行条件。
func (s *Session) enabled(c Command) (bool, string) {
	switch c.Gate {
	case "has_account":
		if len(s.Store.Accounts) == 0 {
			return false, "请先添加账户"
		}
	case "has_focus":
		if s.view.Focused == nil {
			return false, "请先打开一封邮件"
		}
	case "has_messages":
		if len(s.view.Messages) == 0 {
			return false, "没有邮件"
		}
	case "has_selection":
		if len(s.view.Selected) == 0 {
			return false, "请选择邮件"
		}
	case "has_compose":
		if len(s.composes) == 0 {
			return false, "没有写信窗口"
		}
	case "has_group":
		if _, err := s.groupRow(""); err != nil {
			return false, err.Error()
		}
	case "has_group_next", "has_group_prev":
		row, err := s.groupRow("")
		if err != nil || s.view.Focused == nil {
			return false, "没有打开主题组邮件"
		}
		for i, key := range row.Keys {
			if key == *s.view.Focused && (c.Gate == "has_group_next" && i+1 < len(row.Keys) || c.Gate == "has_group_prev" && i > 0) {
				return true, ""
			}
		}
		return false, "已到主题组边界"
	}
	return true, ""
}

// Commands 返回命令发现目录和当前 enabled 状态。
func (s *Session) Commands(search string) json.RawMessage {
	s.mu.Lock()
	defer s.mu.Unlock()
	out := []Command{}
	for _, c := range s.commands {
		if search != "" && !strings.Contains(strings.ToLower(c.ID+" "+c.Label), strings.ToLower(search)) {
			continue
		}
		c.Enabled, _ = s.enabled(c)
		if !c.Enabled {
			_, reason := s.enabled(c)
			c.Reason = &reason
		}
		out = append(out, c)
	}
	b, _ := json.Marshal(out)
	return b
}

// Execute 校验参数并执行命令；所有返回值在锁内序列化形成不可变快照。
func (s *Session) Execute(id string, raw json.RawMessage) (result json.RawMessage, err error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	defer func() {
		if recover() != nil {
			err = fmt.Errorf("命令 %s 发生内部错误", id)
		}
	}()
	var spec *Command
	for _, c := range s.commands {
		if c.ID == id {
			spec = &c
			break
		}
	}
	if spec == nil {
		return nil, fmt.Errorf("未知命令: %s", id)
	}

	p := Params{}
	if len(raw) > 0 && string(raw) != "null" {
		if e := json.Unmarshal(raw, &p); e != nil || p == nil {
			return nil, fmt.Errorf("命令参数必须是 JSON 对象")
		}
	}
	if e := validateParams(id, p); e != nil {
		return nil, e
	}
	if ok, reason := s.enabled(*spec); !ok {
		explicit := false
		if spec.Gate == "has_focus" {
			_, e := mail.ParseKey(str(p, "key"))
			explicit = e == nil
		}
		if spec.Gate == "has_group" && str(p, "group") != "" {
			_, e := s.groupRow(str(p, "group"))
			explicit = e == nil
		}
		if spec.Gate == "has_selection" {
			keys, e := s.selectedKeys(p)
			explicit = e == nil && len(keys) > 0
		}
		if !explicit {
			return nil, errors.New(reason)
		}
	}

	v, e := s.dispatch(id, p)
	if e != nil {
		return nil, e
	}
	if !spec.Query {
		s.journal = append(s.journal, Params{"command": id, "params": redacted(p)})
		if len(s.journal) > 10000 {
			s.journal = s.journal[5000:]
		}
		s.touch()
	}
	return json.Marshal(v)
}

// redacted 阻止命令日志泄漏凭据。
func redacted(p Params) Params {
	out := Params{}
	for k, v := range p {
		if k == "password" || k == "apiKey" {
			out[k] = "[REDACTED]"
		} else {
			out[k] = v
		}
	}
	return out
}

// validateParams 拒绝常用参数类型错误，避免依赖零值静默改变行为。
func validateParams(id string, p Params) error {
	numeric := map[string]bool{"id": strings.HasPrefix(id, "compose.") || id == "ai.compose" || id == "ai.accept" || id == "ai.dismiss", "limit": true, "offset": true, "index": true, "start": true, "end": true, "imapPort": true, "smtpPort": true}
	bools := map[string]bool{"verify": true, "read": true, "flagged": true, "extend": true, "rich": true, "group": id == "message.list", "unreadOnly": true, "force": true}
	for k, v := range p {
		if numeric[k] {
			n, ok := v.(float64)
			if !ok || n < 0 || n > float64(1<<53-1) || n != float64(int64(n)) {
				return fmt.Errorf("%s 必须是非负整数", k)
			}
		}
		if bools[k] {
			if _, ok := v.(bool); !ok {
				return fmt.Errorf("%s 必须是布尔值", k)
			}
		}
	}
	return nil
}

// Snapshot 汇总 UI 所需状态；读取不会消费其他窗口的事件。
func (s *Session) Snapshot() json.RawMessage {
	s.mu.Lock()
	defer s.mu.Unlock()
	composes := []*Compose{}
	for _, c := range s.composes {
		// 预览中的 CID 映射只写入快照副本，发送仍保留原 HTML 和内联附件。
		view := *c
		if c.Quote != nil {
			quote := *c.Quote
			body := readerBody(mail.Body{HTML: &quote.HTML, Attachments: quote.Inline})
			if body.HTML != nil {
				quote.HTML = *body.HTML
			}
			view.Quote = &quote
		}
		composes = append(composes, &view)
	}
	sort.Slice(composes, func(i, j int) bool { return composes[i].ID < composes[j].ID })
	var current any
	if s.view.Focused != nil {
		current = s.messageResult(*s.view.Focused)
	}
	v := Params{"revision": s.revision, "accounts": s.Store.Accounts, "folders": s.Store.Folders, "view": s.view, "message": current, "composes": composes, "prefs": s.prefs, "status": s.status, "requests": s.requests, "aiReply": s.aiReply, "aiGroupSummary": s.aiGroupSummary, "calendar": s.calendar, "invoice": s.invoice, "update": s.update, "demo": s.demo}
	b, _ := json.Marshal(v)
	return b
}

// Journal 返回脱敏的命令历史。
func (s *Session) Journal() json.RawMessage {
	s.mu.Lock()
	defer s.mu.Unlock()
	b, _ := json.Marshal(s.journal)
	return b
}

// acknowledge 清除已被宿主处理的界面请求。
func (s *Session) AcknowledgeRequests() []UIRequest {
	s.mu.Lock()
	defer s.mu.Unlock()
	out := s.requests
	s.requests = []UIRequest{}
	return out
}

// allMessages 从所有可选择文件夹读取缓存邮件。
func (s *Session) allMessages() ([]mail.Message, error) {
	out := []mail.Message{}
	for _, a := range s.Store.Accounts {
		for fi, f := range s.Store.Folders[a.ID] {
			if !f.Selectable {
				continue
			}
			idx, e := s.Store.Index(a.ID, f.Path)
			if e != nil {
				return nil, e
			}
			for _, message := range idx.Messages {
				out = append(out, protocol.DecodeMetadata(message))
			}
			s.Store.Folders[a.ID][fi].Total = uint32(len(idx.Messages))
			unread := uint32(0)
			for _, m := range idx.Messages {
				if !m.Flags.Seen {
					unread++
				}
			}
			s.Store.Folders[a.ID][fi].Unread = unread
		}
	}
	sort.SliceStable(out, func(i, j int) bool {
		if out[i].Date == out[j].Date {
			return out[i].Key.UID > out[j].Key.UID
		}
		return out[i].Date > out[j].Date
	})
	return out, nil
}

// refresh 根据范围、搜索、偏好重新计算列表行。
func (s *Session) refresh() error {
	all, e := s.allMessages()
	if e != nil {
		return e
	}
	s.view.Messages = []mail.Message{}
	q := strings.ToLower(s.view.Query)
	for _, m := range all {
		scope := s.view.Scope
		role := mail.FolderRole(m.Key.Folder, nil)
		for _, f := range s.Store.Folders[m.Key.Account] {
			if f.Path == m.Key.Folder {
				role = f.Role
			}
		}
		include := false
		switch scope.Kind {
		case "folder":
			include = m.Key.Account == scope.Account && (q != "" || m.Key.Folder == scope.Folder)
		case "unread":
			include = !m.Flags.Seen && role != "junk" && role != "trash"
		case "flagged":
			include = m.Flags.Flagged
		case "invoices":
			include = mail.IsInvoice(m) && role != "sent" && role != "drafts"
		default:
			include = role == "inbox"
		}
		if include && (q == "" || strings.Contains(strings.ToLower(m.Subject+" "+m.From.Name+" "+m.From.Email+" "+m.Preview), q)) {
			s.view.Messages = append(s.view.Messages, m)
		}
	}
	s.view.Rows = []Row{}
	groups := map[string]int{}
	for _, m := range s.view.Messages {
		key := mail.SubjectKey(m.Subject)
		idx, ok := groups[key]
		if !boolean(s.prefs, "groupBySubject", true) || key == "" {
			ok = false
		}
		if !ok {
			idx = len(s.view.Rows)
			s.view.Rows = append(s.view.Rows, Row{Message: m, Keys: []string{}, Senders: []string{}})
			if key != "" {
				groups[key] = idx
			}
		}
		r := &s.view.Rows[idx]
		r.Count++
		r.Keys = append(r.Keys, m.Key.String())
		name := m.From.Name
		if name == "" {
			name = m.From.Email
		}
		r.Senders = append(r.Senders, name)
		if !m.Flags.Seen {
			r.Unread++
		}
		r.Flagged = r.Flagged || m.Flags.Flagged
		if r.Count > 1 {
			r.Group = &key
		}
	}
	return nil
}

// messageResult 形成 message.get 的加载状态响应。
func (s *Session) messageResult(key string) Params {
	out := Params{"state": "missing", "body": nil}
	k, e := mail.ParseKey(key)
	if e != nil {
		return out
	}
	idx, e := s.Store.Index(k.Account, k.Folder)
	if e == nil {
		for _, m := range idx.Messages {
			if m.Key == k {
				out["meta"] = protocol.DecodeMetadata(m)
				break
			}
		}
	}
	if b, ok := s.bodies[key]; ok {
		out["body"] = readerBody(b)
		out["state"] = "ready"
		return out
	}
	if raw, e := s.Store.Raw(k); e == nil {
		if b, e := protocol.Parse(raw); e == nil {
			s.bodies[key] = b
			out["body"] = readerBody(b)
			out["state"] = "ready"
			return out
		} else {
			s.bodyErrors[key] = e.Error()
		}
	}
	if s.loading[key] {
		out["state"] = "loading"
	}
	if e, ok := s.bodyErrors[key]; ok {
		out["state"] = "failed"
		out["error"] = e
	}
	return out
}

// focusedKey 使用显式邮件键或当前聚焦邮件。
func (s *Session) focusedKey(p Params) (mail.Key, error) {
	key := str(p, "key")
	if key == "" && s.view.Focused != nil {
		key = *s.view.Focused
	}
	if key == "" {
		return mail.Key{}, fmt.Errorf("请先打开邮件")
	}
	return mail.ParseKey(key)
}

// savePrefs 原子保存偏好。
func (s *Session) savePrefs() error {
	return store.WriteJSON(filepath.Join(s.Store.Root, "prefs.json"), s.prefs)
}

// Wait 等待后台任务结束，用于无界面交付入口。
func (s *Session) Wait(ctx context.Context) error {
	tick := time.NewTicker(time.Millisecond * 20)
	defer tick.Stop()
	for {
		s.mu.Lock()
		busy := len(s.busy) > 0
		s.mu.Unlock()
		if !busy {
			return nil
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-tick.C:
		}
	}
}

// DataRoot 解析跨平台默认数据目录与显式覆盖。
func DataRoot(explicit string) (string, error) {
	if explicit != "" {
		return filepath.Abs(explicit)
	}
	if root := os.Getenv("MAILFABRICA_HOME"); root != "" {
		return filepath.Abs(root)
	}
	if runtime.GOOS != "darwin" && runtime.GOOS != "windows" {
		if root := os.Getenv("XDG_DATA_HOME"); root != "" {
			return filepath.Join(root, "mailfabrica"), nil
		}
		home, e := os.UserHomeDir()
		if e != nil {
			return "", e
		}
		return filepath.Join(home, ".local", "share", "mailfabrica"), nil
	}
	root, e := os.UserConfigDir()
	if e != nil {
		return "", e
	}
	return filepath.Join(root, "MailFabrica"), nil
}

// readerBody 将 CID 图片映射到隔离正文可读取的数据 URL，不把附件字节暴露给应用脚本。
func readerBody(body mail.Body) mail.Body {
	if body.HTML == nil {
		return body
	}
	source := *body.HTML
	for _, a := range body.Attachments {
		if a.ContentID == nil || len(a.Data) > 20<<20 {
			continue
		}
		kind := a.MIME
		if !strings.HasPrefix(kind, "image/") {
			continue
		}
		source = strings.ReplaceAll(source, "cid:"+*a.ContentID, "data:"+kind+";base64,"+base64.StdEncoding.EncodeToString(a.Data))
	}
	body.HTML = &source
	return body
}
