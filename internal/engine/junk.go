package engine

import (
	"context"
	"encoding/json"
	"fmt"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"

	assist "github.com/openfabrica/mailfabrica/internal/ai"
	"github.com/openfabrica/mailfabrica/internal/mail"
	"github.com/openfabrica/mailfabrica/internal/protocol"
)

// JunkCandidate 是 AI 判定的疑似垃圾邮件，仅作建议，移动需用户确认。
type JunkCandidate struct {
	Key     string       `json:"key"`
	Subject string       `json:"subject"`
	From    mail.Address `json:"from"`
	Date    int64        `json:"date"`
	Reason  string       `json:"reason"`
}

// JunkScan 记录一次 AI 垃圾邮件扫描的进度与候选，不写入邮件或索引。
type JunkScan struct {
	Busy       bool            `json:"busy"`
	Scanned    int             `json:"scanned"`
	Done       int             `json:"done"`
	Current    string          `json:"current"`
	Candidates []JunkCandidate `json:"candidates"`
	Error      *string         `json:"error"`
}

// junkBatch 是每次 AI 调用的邮件数；分批让进度和结果能边识别边显示。
const junkBatch = 10

// junkWorkers 限制同时进行的 AI 请求，避免触发服务端限流。
const junkWorkers = 3

// clip 按字符截断并压平换行，限制发送给 AI 的单字段长度。
func clip(text string, n int) string {
	text = strings.Join(strings.Fields(text), " ")
	runes := []rune(text)
	if len(runes) > n {
		return string(runes[:n]) + "…"
	}
	return text
}

// folderRoleOf 返回文件夹角色；索引中缺失时按名称推断。
func (s *Session) folderRoleOf(account, path string) string {
	for _, f := range s.Store.Folders[account] {
		if f.Path == path {
			return f.Role
		}
	}
	return mail.FolderRole(path, nil)
}

// junkSource 选取待检查邮件：指定账户时取其文件夹（默认收件箱），否则取当前列表。
func (s *Session) junkSource(p Params) ([]mail.Message, error) {
	account := str(p, "account")
	if account == "" {
		return append([]mail.Message{}, s.view.Messages...), nil
	}
	if _, e := s.account(account); e != nil {
		return nil, e
	}
	folder := str(p, "folder")
	if folder == "" {
		inbox, e := s.roleFolder(account, "inbox")
		if e != nil {
			return nil, e
		}
		folder = inbox
	}
	idx, e := s.Store.Index(account, folder)
	if e != nil {
		return nil, e
	}
	out := []mail.Message{}
	for _, m := range idx.Messages {
		out = append(out, protocol.DecodeMetadata(m))
	}
	return out, nil
}

// scanJunk 只把发件人、主题和预览交给 AI，结果作为候选等待用户确认移动。
func (s *Session) scanJunk(p Params) (any, error) {
	if s.aiJunk != nil && s.aiJunk.Busy {
		return nil, fmt.Errorf("AI 正在识别垃圾邮件，请稍后再试")
	}
	limit := number(p, "limit", 100)
	if limit < 1 || limit > 200 {
		return nil, fmt.Errorf("limit 必须在 1–200 之间")
	}
	config, err := s.aiConfig()
	if err != nil {
		return nil, err
	}
	if config["ready"] != true {
		return nil, fmt.Errorf("请先配置 AI 服务")
	}
	key, err := s.secrets.Get("ai:api-key")
	if err != nil {
		return nil, err
	}
	source, err := s.junkSource(p)
	if err != nil {
		return nil, err
	}
	// 已在垃圾箱、废纸篓或自己发出的邮件不参与识别
	items := []mail.Message{}
	for _, m := range source {
		switch s.folderRoleOf(m.Key.Account, m.Key.Folder) {
		case "junk", "trash", "sent", "drafts":
			continue
		}
		if !m.Flags.Draft {
			items = append(items, m)
		}
	}
	if len(items) == 0 {
		return nil, fmt.Errorf("没有可检查的邮件")
	}
	sort.SliceStable(items, func(i, j int) bool { return items[i].Date > items[j].Date })
	if len(items) > limit {
		items = items[:limit]
	}
	language := "reason 使用简体中文。"
	if str(s.prefs, "language") == "en" {
		language = "Write reason in English."
	}
	assistConfig := assist.Config{Provider: str(config, "provider"), BaseURL: str(config, "effectiveUrl"), Model: str(config, "effectiveModel")}
	result := &JunkScan{Busy: true, Scanned: len(items), Candidates: []JunkCandidate{}}
	s.aiJunk = result
	err = s.launch("ai:junk", func(ctx context.Context) (any, error) {
		jobs := make(chan int)
		var wg sync.WaitGroup
		failed := 0
		var firstErr error
		for w := 0; w < junkWorkers; w++ {
			wg.Add(1)
			go func() {
				defer wg.Done()
				for start := range jobs {
					batch := items[start:min(start+junkBatch, len(items))]
					s.mu.Lock()
					result.Current = batch[0].Subject
					s.touch()
					s.mu.Unlock()
					text, err := assist.ClassifyJunk(ctx, assistConfig, key, junkPrompt(language, batch))
					var found []JunkCandidate
					if err == nil {
						found, err = parseJunk(text, batch)
					}
					// 每批完成立即写回进度与候选，界面轮询即可看到
					s.mu.Lock()
					result.Done += len(batch)
					result.Candidates = append(result.Candidates, found...)
					if err != nil {
						failed += len(batch)
						if firstErr == nil {
							firstErr = err
						}
					}
					s.touch()
					s.mu.Unlock()
				}
			}()
		}
		for start := 0; start < len(items) && ctx.Err() == nil; start += junkBatch {
			jobs <- start
		}
		close(jobs)
		wg.Wait()
		s.mu.Lock()
		defer s.mu.Unlock()
		result.Current = ""
		// 全部失败才算任务失败；部分失败保留已识别结果并说明未识别封数
		if failed == len(items) {
			result.Busy = false
			message := firstErr.Error()
			result.Error = &message
			return nil, firstErr
		}
		if failed > 0 {
			message := fmt.Sprintf("%d 封邮件未能识别：%s", failed, firstErr.Error())
			result.Error = &message
		}
		return nil, nil
	}, func(any) error {
		result.Busy = false
		// 并发批次完成顺序不定，最终按时间从新到旧排列
		sort.SliceStable(result.Candidates, func(i, j int) bool { return result.Candidates[i].Date > result.Candidates[j].Date })
		return nil
	})
	if err != nil {
		result.Busy = false
		message := err.Error()
		result.Error = &message
	}
	return result, err
}

// junkPrompt 生成单批邮件的元数据提示，编号从 1 开始，仅在本批内有效。
func junkPrompt(language string, batch []mail.Message) string {
	var prompt strings.Builder
	fmt.Fprintf(&prompt, "%s\n以下是 %d 封邮件的元数据，按编号逐封判断是否为垃圾邮件。\n", language, len(batch))
	for i, m := range batch {
		fmt.Fprintf(&prompt, "\n[%d]\n发件人：%s\n时间：%s\n主题：%s\n摘要：%s\n", i+1, clip(m.From.Display(), 200), time.Unix(m.Date, 0).Format(time.RFC3339), clip(m.Subject, 200), clip(m.Preview, 300))
	}
	return prompt.String()
}

// parseJunk 从模型回复中取出编号数组；越界或重复编号被忽略，不信任模型输出的邮件键。
func parseJunk(text string, items []mail.Message) ([]JunkCandidate, error) {
	entries, ok := junkEntries(text)
	if !ok {
		return nil, fmt.Errorf("AI 未返回有效的识别结果：%s", clip(text, 80))
	}
	seen := map[int]bool{}
	candidates := []JunkCandidate{}
	for _, v := range entries {
		id := junkID(v["id"])
		if id < 1 || id > len(items) || seen[id] {
			continue
		}
		seen[id] = true
		reason, _ := v["reason"].(string)
		m := items[id-1]
		candidates = append(candidates, JunkCandidate{Key: m.Key.String(), Subject: m.Subject, From: m.From, Date: m.Date, Reason: clip(reason, 200)})
	}
	return candidates, nil
}

// junkEntries 在回复中逐段尝试 JSON，返回最后一段符合格式的非空结果。
// 推理模型会输出思考段，邮件主题也常带 [高] 这类方括号，不能简单截取首尾括号。
func junkEntries(text string) ([]map[string]any, bool) {
	if end := strings.LastIndex(text, "</think>"); end >= 0 {
		text = text[end+len("</think>"):]
	}
	var found []map[string]any
	ok := false
	for i := 0; i < len(text); i++ {
		if text[i] != '[' && text[i] != '{' {
			continue
		}
		decoder := json.NewDecoder(strings.NewReader(text[i:]))
		var value any
		if decoder.Decode(&value) != nil {
			continue
		}
		// 优先采用最后一段非空结果，避免被说明文字里的 [] 覆盖
		if entries, match := junkShape(value); match && (len(entries) > 0 || len(found) == 0) {
			found, ok = entries, true
		}
		// 跳过已解析的整段，避免把数组里的单个对象当成完整结果
		i += int(decoder.InputOffset()) - 1
	}
	return found, ok
}

// junkShape 接受对象数组、空数组，或把数组包在任意字段中的对象。
func junkShape(value any) ([]map[string]any, bool) {
	switch v := value.(type) {
	case []any:
		out := []map[string]any{}
		for _, item := range v {
			entry, ok := item.(map[string]any)
			if !ok || junkID(entry["id"]) == 0 {
				return nil, false
			}
			out = append(out, entry)
		}
		return out, true
	case map[string]any:
		if junkID(v["id"]) != 0 {
			return []map[string]any{v}, true
		}
		for _, field := range v {
			if entries, ok := junkShape(field); ok {
				if _, isList := field.([]any); isList {
					return entries, true
				}
			}
		}
	}
	return nil, false
}

// junkID 兼容数字和数字字符串形式的编号，无效时返回 0。
func junkID(value any) int {
	switch v := value.(type) {
	case float64:
		if v == float64(int(v)) {
			return int(v)
		}
	case string:
		if n, err := strconv.Atoi(strings.TrimSpace(v)); err == nil {
			return n
		}
	}
	return 0
}

// cleanJunk 将用户确认的候选移到各自账户的垃圾邮件文件夹。
func (s *Session) cleanJunk(p Params) (any, error) {
	if s.aiJunk == nil || s.aiJunk.Busy {
		return nil, fmt.Errorf("没有可清理的识别结果")
	}
	wanted := map[string]bool{}
	if v, exists := p["keys"]; exists {
		a, ok := v.([]any)
		if !ok {
			return nil, fmt.Errorf("keys 必须是字符串数组")
		}
		for _, item := range a {
			k, ok := item.(string)
			if !ok {
				return nil, fmt.Errorf("邮件键必须是字符串")
			}
			wanted[k] = true
		}
	}
	keys := []any{}
	remaining := []JunkCandidate{}
	for _, c := range s.aiJunk.Candidates {
		if len(wanted) > 0 && !wanted[c.Key] {
			remaining = append(remaining, c)
			continue
		}
		delete(wanted, c.Key)
		// 扫描后已被移动或删除的邮件直接从候选中去掉
		k, err := mail.ParseKey(c.Key)
		if err != nil {
			return nil, err
		}
		if _, err = s.findMeta(k); err == nil {
			keys = append(keys, c.Key)
		}
	}
	if len(wanted) > 0 {
		return nil, fmt.Errorf("只能清理 AI 识别出的邮件")
	}
	if len(keys) > 0 {
		if _, err := s.messageCommand("message.junk", Params{"keys": keys}); err != nil {
			return nil, err
		}
	}
	s.aiJunk.Candidates = remaining
	return Params{"moved": len(keys)}, nil
}
