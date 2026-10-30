package engine

import (
	"context"
	"fmt"
	"sort"
	"strings"
	"sync"
	"time"

	assist "github.com/openfabrica/mailfabrica/internal/ai"
	"github.com/openfabrica/mailfabrica/internal/mail"
	"github.com/openfabrica/mailfabrica/internal/protocol"
)

// GroupSummary 仅属于一次特定主题组和邮件集合，不写入邮件或写信状态。
type GroupSummary struct {
	Group string   `json:"group"`
	Keys  []string `json:"keys"`
	Busy  bool     `json:"busy"`
	Text  *string  `json:"text"`
	Error *string  `json:"error"`
}

// groupRow 查找明确的聚合组；非聚合邮件不能运行组内操作。
func (s *Session) groupRow(group string) (Row, error) {
	if group == "" && s.view.Group != nil {
		group = *s.view.Group
	}
	for _, row := range s.view.Rows {
		if row.Group != nil && *row.Group == group && len(row.Keys) > 1 {
			return row, nil
		}
	}
	return Row{}, fmt.Errorf("没有打开聚合邮件")
}

// summaryMail 固定本次汇总的元数据与正文；后台不访问 Store。
type summaryMail struct {
	Meta     mail.Message
	Text     *string
	Account  mail.Account
	Password string
	Lock     *sync.Mutex
}

// summarizeGroup 读取完整组内正文后汇总，不将摘要冒充原文，也不自动发送。
func (s *Session) summarizeGroup(p Params) (any, error) {
	if v, ok := p["group"]; ok {
		if _, ok = v.(string); !ok {
			return nil, fmt.Errorf("group 必须是字符串")
		}
	}
	row, err := s.groupRow(str(p, "group"))
	if err != nil {
		return nil, err
	}
	if s.aiGroupSummary != nil && s.aiGroupSummary.Busy {
		return nil, fmt.Errorf("AI 正在汇总，请稍后再试")
	}
	if len(row.Keys) > 200 {
		return nil, fmt.Errorf("单次最多汇总 200 封邮件")
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
	items := []summaryMail{}
	for _, value := range row.Keys {
		parsed, err := mail.ParseKey(value)
		if err != nil {
			return nil, err
		}
		meta, err := s.findMeta(parsed)
		if err != nil {
			return nil, err
		}
		item := summaryMail{Meta: meta}
		if body, err := s.loadBody(parsed); err == nil {
			text := body.Text
			item.Text = &text
		} else {
			item.Account, err = s.account(parsed.Account)
			if err != nil {
				return nil, err
			}
			item.Password, err = s.secrets.Get(parsed.Account)
			if err != nil {
				return nil, err
			}
			item.Lock = s.accountLocks[parsed.Account]
			if item.Lock == nil {
				item.Lock = &sync.Mutex{}
				s.accountLocks[parsed.Account] = item.Lock
			}
		}
		items = append(items, item)
	}
	sort.SliceStable(items, func(i, j int) bool { return items[i].Meta.Date < items[j].Meta.Date })
	result := &GroupSummary{Group: *row.Group, Keys: append([]string{}, row.Keys...), Busy: true}
	s.aiGroupSummary = result
	language := "请使用简体中文。"
	if str(s.prefs, "language") == "en" {
		language = "Respond in English."
	}
	err = s.launch("ai:group", func(ctx context.Context) (any, error) {
		text, err := func() (string, error) {
			var prompt strings.Builder
			prompt.WriteString(language + "\n以下是同一主题组的全部邮件，按时间从早到晚排列。请汇总并在相关结论后标明邮件编号。\n")
			for i, item := range items {
				body := item.Text
				if body == nil {
					raw, err := func() ([]byte, error) {
						item.Lock.Lock()
						defer item.Lock.Unlock()
						return s.remote.Fetch(ctx, item.Account, item.Password, item.Meta.Key)
					}()
					if err != nil {
						return "", fmt.Errorf("第 %d 封邮件正文读取失败：%w", i+1, err)
					}
					parsed, err := protocol.Parse(raw)
					if err != nil {
						return "", err
					}
					body = &parsed.Text
				}
				if prompt.Len()+len(*body)+len(item.Meta.Subject)+len(item.Meta.From.Name)+len(item.Meta.From.Email)+200 > 512*1024 {
					return "", fmt.Errorf("邮件正文总量超过 512 KiB，无法一次汇总全部内容")
				}
				fmt.Fprintf(&prompt, "\n--- 邮件 %d ---\n发件人：%s\n时间：%s\n主题：%s\n正文：\n%s\n", i+1, item.Meta.From.Display(), time.Unix(item.Meta.Date, 0).Format(time.RFC3339), item.Meta.Subject, *body)
			}
			return assist.Summarize(ctx, assist.Config{Provider: str(config, "provider"), BaseURL: str(config, "effectiveUrl"), Model: str(config, "effectiveModel")}, key, prompt.String())
		}()
		if err != nil {
			s.mu.Lock()
			result.Busy = false
			message := err.Error()
			result.Error = &message
			s.mu.Unlock()
		}
		return text, err
	}, func(value any) error {
		result.Busy = false
		text, ok := value.(string)
		if !ok {
			return fmt.Errorf("AI 汇总结果无效")
		}
		result.Text = &text
		return nil
	})
	if err != nil {
		result.Busy = false
		message := err.Error()
		result.Error = &message
	}
	return result, err
}
