package engine

import (
	"context"
	"fmt"
	"strings"

	assist "github.com/openfabrica/mailfabrica/internal/ai"
)

// Translation 保存单封邮件的译文，邮件键和目标语言固定于发起请求时。
type Translation struct {
	Key      string  `json:"key"`
	Language string  `json:"language"`
	Busy     bool    `json:"busy"`
	Text     *string `json:"text"`
	Error    *string `json:"error"`
}

// translateMessage 只翻译已下载正文，不修改原文、已读状态或写信内容。
func (s *Session) translateMessage(p Params) (any, error) {
	if value, ok := p["key"]; ok {
		if key, valid := value.(string); !valid || key == "" {
			return nil, fmt.Errorf("key 必须是非空字符串")
		}
	}
	language := str(p, "language")
	if _, ok := p["language"]; !ok {
		language = "zh-CN"
		if str(s.prefs, "language") == "en" {
			language = "en"
		}
	}
	languages := map[string]string{"zh-CN": "简体中文", "zh-TW": "繁体中文", "en": "English", "ja": "日文", "ko": "韩文"}
	target, ok := languages[language]
	if !ok {
		return nil, fmt.Errorf("不支持的翻译目标语言")
	}
	if s.busy["ai:translate"] {
		return nil, fmt.Errorf("AI 正在翻译，请稍后再试")
	}
	k, err := s.focusedKey(p)
	if err != nil {
		return nil, err
	}
	meta, err := s.findMeta(k)
	if err != nil {
		return nil, err
	}
	body, err := s.loadBody(k)
	if err != nil {
		return nil, fmt.Errorf("请先打开邮件并下载正文：%w", err)
	}
	if strings.TrimSpace(body.Text) == "" {
		return nil, fmt.Errorf("邮件没有可翻译的文字正文")
	}
	if len(body.Text)+len(meta.Subject) > 64*1024 {
		return nil, fmt.Errorf("邮件文本超过 64 KiB，无法一次完整翻译")
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
	prompt := fmt.Sprintf("目标语言：%s\n请翻译以下邮件主题和完整正文。\n主题：%s\n正文：\n%s", target, meta.Subject, body.Text)
	result := &Translation{Key: k.String(), Language: language, Busy: true}
	s.aiTranslation = result
	err = s.launch("ai:translate", func(ctx context.Context) (any, error) {
		text, err := assist.Translate(ctx, assist.Config{Provider: str(config, "provider"), BaseURL: str(config, "effectiveUrl"), Model: str(config, "effectiveModel")}, key, prompt)
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
			return fmt.Errorf("AI 翻译结果无效")
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
