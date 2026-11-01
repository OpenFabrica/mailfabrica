package engine

import (
	"context"
	"encoding/json"
	"fmt"
	"sort"
	"strings"
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

// JunkScan 记录一次 AI 垃圾邮件扫描的状态与候选，不写入邮件或索引。
type JunkScan struct {
	Busy       bool            `json:"busy"`
	Scanned    int             `json:"scanned"`
	Candidates []JunkCandidate `json:"candidates"`
	Error      *string         `json:"error"`
}

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
	var prompt strings.Builder
	fmt.Fprintf(&prompt, "%s\n以下是 %d 封邮件的元数据，按编号逐封判断是否为垃圾邮件。\n", language, len(items))
	for i, m := range items {
		fmt.Fprintf(&prompt, "\n[%d]\n发件人：%s\n时间：%s\n主题：%s\n摘要：%s\n", i+1, clip(m.From.Display(), 200), time.Unix(m.Date, 0).Format(time.RFC3339), clip(m.Subject, 200), clip(m.Preview, 300))
	}
	result := &JunkScan{Busy: true, Scanned: len(items), Candidates: []JunkCandidate{}}
	s.aiJunk = result
	err = s.launch("ai:junk", func(ctx context.Context) (any, error) {
		text, err := assist.ClassifyJunk(ctx, assist.Config{Provider: str(config, "provider"), BaseURL: str(config, "effectiveUrl"), Model: str(config, "effectiveModel")}, key, prompt.String())
		var candidates []JunkCandidate
		if err == nil {
			candidates, err = parseJunk(text, items)
		}
		if err != nil {
			s.mu.Lock()
			result.Busy = false
			message := err.Error()
			result.Error = &message
			s.mu.Unlock()
		}
		return candidates, err
	}, func(value any) error {
		result.Busy = false
		candidates, ok := value.([]JunkCandidate)
		if !ok {
			return fmt.Errorf("AI 识别结果无效")
		}
		result.Candidates = candidates
		return nil
	})
	if err != nil {
		result.Busy = false
		message := err.Error()
		result.Error = &message
	}
	return result, err
}

// parseJunk 校验 AI 返回的编号；越界或重复编号被忽略，不信任模型输出的邮件键。
func parseJunk(text string, items []mail.Message) ([]JunkCandidate, error) {
	// 模型可能包裹代码块或说明文字，只取最外层数组
	start, end := strings.Index(text, "["), strings.LastIndex(text, "]")
	if start < 0 || end < start {
		return nil, fmt.Errorf("AI 未返回有效的识别结果")
	}
	var out []struct {
		ID     int    `json:"id"`
		Reason string `json:"reason"`
	}
	if err := json.Unmarshal([]byte(text[start:end+1]), &out); err != nil {
		return nil, fmt.Errorf("AI 未返回有效的识别结果")
	}
	seen := map[int]bool{}
	candidates := []JunkCandidate{}
	for _, v := range out {
		if v.ID < 1 || v.ID > len(items) || seen[v.ID] {
			continue
		}
		seen[v.ID] = true
		m := items[v.ID-1]
		candidates = append(candidates, JunkCandidate{Key: m.Key.String(), Subject: m.Subject, From: m.From, Date: m.Date, Reason: clip(v.Reason, 200)})
	}
	return candidates, nil
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
