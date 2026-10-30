package engine

import (
	"context"
	"fmt"
	assist "github.com/openfabrica/mailfabrica/internal/ai"
	"strings"
)

// aiConfig 计算实际接口配置，密钥仅返回是否存在。
func (s *Session) aiConfig() (Params, error) {
	provider := str(s.prefs, "aiProvider")
	base := strings.TrimRight(str(s.prefs, "aiBaseUrl"), "/")
	model := str(s.prefs, "aiModel")
	if provider == "anthropic" {
		if base == "" {
			base = "https://api.anthropic.com"
		}
		if model == "" {
			model = "claude-opus-5-5"
		}
	}
	key, e := s.secrets.Get("ai:api-key")
	if e != nil {
		return nil, e
	}
	valid := assist.ValidateURL(base) == nil && model != ""
	return Params{"provider": provider, "baseUrl": str(s.prefs, "aiBaseUrl"), "model": str(s.prefs, "aiModel"), "effectiveUrl": base, "effectiveModel": model, "hasKey": key != "", "ready": valid && (key != "" || provider == "openai")}, nil
}

// ownBodyEnd 将签名和原文引用排除在 AI 改写范围之外。
func ownBodyEnd(body string) int {
	pos := 0
	for _, line := range strings.SplitAfter(body, "\n") {
		l := strings.TrimRight(line, "\r\n")
		if strings.TrimSpace(l) == "--" || strings.HasPrefix(l, "---------- 转发的邮件") || strings.HasPrefix(l, "---------- Forwarded message") || strings.HasSuffix(l, "写道：") || strings.HasSuffix(l, " wrote:") {
			return pos
		}
		pos += len(line)
	}
	return len(body)
}

// aiCommand 保留建议采用/放弃流程，生成结果不直接覆盖用户正文。
func (s *Session) aiCommand(id string, p Params) (any, error) {
	switch id {
	case "ai.summarizeGroup":
		return s.summarizeGroup(p)
	case "ai.config":
		if provider, ok := p["provider"]; ok {
			if provider != "anthropic" && provider != "openai" {
				return nil, fmt.Errorf("AI 协议无效")
			}
			s.prefs["aiProvider"] = provider
		}
		if base, ok := p["baseUrl"]; ok {
			v, ok := base.(string)
			if !ok {
				return nil, fmt.Errorf("baseUrl 必须是字符串")
			}
			if v != "" {
				if e := assist.ValidateURL(v); e != nil {
					return nil, e
				}
			}
			s.prefs["aiBaseUrl"] = strings.TrimSpace(v)
		}
		if model, ok := p["model"]; ok {
			v, ok := model.(string)
			if !ok {
				return nil, fmt.Errorf("model 必须是字符串")
			}
			s.prefs["aiModel"] = strings.TrimSpace(v)
		}
		if value, ok := p["apiKey"]; ok {
			key, ok := value.(string)
			if !ok {
				return nil, fmt.Errorf("apiKey 必须是字符串")
			}
			if e := s.secrets.Set("ai:api-key", strings.TrimSpace(key)); e != nil {
				return nil, e
			}
		}
		if len(p) > 0 {
			if e := s.savePrefs(); e != nil {
				return nil, e
			}
		}
		return s.aiConfig()
	case "ai.status":
		composes := []Params{}
		for _, c := range s.composes {
			composes = append(composes, Params{"id": c.ID, "busy": c.AIBusy, "suggestion": c.AISuggestion, "error": c.AIError})
		}
		config, e := s.aiConfig()
		if e != nil {
			return nil, e
		}
		return Params{"config": config, "reply": s.aiReply, "group": s.aiGroupSummary, "composes": composes}, nil
	case "ai.clearReply":
		s.aiReply = nil
		return nil, nil
	case "ai.accept", "ai.dismiss":
		c, e := s.composeByID(p)
		if e != nil {
			return nil, e
		}
		if id == "ai.dismiss" {
			c.AISuggestion = nil
			return nil, nil
		}
		if c.AISuggestion == nil {
			return nil, fmt.Errorf("没有可采用的建议")
		}
		mode := str(p, "mode")
		if mode != "" && mode != "replace" && mode != "append" {
			return nil, fmt.Errorf("采用方式无效")
		}
		end := ownBodyEnd(c.Body)
		body := *c.AISuggestion + c.Body[end:]
		if mode == "append" {
			body = c.Body[:end] + "\n\n" + *c.AISuggestion + c.Body[end:]
		}
		c.Doc.replace(body, plainStyle())
		c.Body = body
		c.HTML = c.Doc.HTML()
		c.AISuggestion = nil
		c.Dirty = true
		return nil, nil
	}
	config, e := s.aiConfig()
	if e != nil {
		return nil, e
	}
	if config["ready"] != true {
		return nil, fmt.Errorf("请先配置 AI 服务")
	}
	key, e := s.secrets.Get("ai:api-key")
	if e != nil {
		return nil, e
	}
	prompt := ""
	jobID := "ai:reply"
	var composeID uint64
	var messageKey string
	var originalBody string
	if id == "ai.compose" {
		c, e := s.composeByID(p)
		if e != nil {
			return nil, e
		}
		if c.AIBusy {
			return nil, fmt.Errorf("AI 正在生成")
		}
		action := str(p, "action")
		if action == "" {
			action = "write"
		}
		switch action {
		case "write", "polish", "formal", "friendly", "shorter", "longer", "translate":
		default:
			return nil, fmt.Errorf("未知 AI 操作")
		}
		composeID = c.ID
		originalBody = c.Body
		prompt = fmt.Sprintf("操作: %s\n要求: %s\n收件人: %s\n主题: %s\n用户正文:\n%s", action, str(p, "instruction"), c.To, c.Subject, c.Body[:ownBodyEnd(c.Body)])
		if c.Source != nil {
			if b, e := s.loadBody(*c.Source); e == nil {
				prompt += "\n原邮件上下文:\n" + b.Text
			}
		}
		jobID = fmt.Sprintf("ai:compose:%d", c.ID)
		c.AIBusy = true
		c.AIError = nil
	} else {
		k, e := s.focusedKey(p)
		if e != nil {
			return nil, e
		}
		b, e := s.loadBody(k)
		if e != nil {
			return nil, e
		}
		messageKey = k.String()
		prompt = "为以下邮件写一条简短回复。\n要求: " + str(p, "instruction") + "\n原邮件:\n" + b.Text
		s.aiReply = Params{"key": messageKey, "seq": s.revision + 1, "busy": true, "text": nil, "error": nil}
	}
	return nil, s.launch(jobID, func(ctx context.Context) (any, error) {
		text, err := assist.Complete(ctx, assist.Config{Provider: str(config, "provider"), BaseURL: str(config, "effectiveUrl"), Model: str(config, "effectiveModel")}, key, prompt)
		if err != nil {
			s.mu.Lock()
			if c, ok := s.composes[composeID]; ok {
				c.AIBusy = false
				msg := err.Error()
				c.AIError = &msg
			}
			if composeID == 0 {
				s.aiReply = Params{"key": messageKey, "seq": s.revision + 1, "busy": false, "error": err.Error()}
			}
			s.mu.Unlock()
		}
		return text, err
	}, func(v any) error {
		text, ok := v.(string)
		if !ok {
			return fmt.Errorf("AI 结果无效")
		}
		if composeID != 0 {
			if c, ok := s.composes[composeID]; ok {
				c.AIBusy = false
				if c.Body != originalBody {
					msg := "正文在生成期间已修改，请重新生成"
					c.AIError = &msg
				} else {
					c.AISuggestion = &text
				}
			}
		} else {
			s.aiReply = Params{"key": messageKey, "seq": s.revision + 1, "busy": false, "text": text, "error": nil}
		}
		return nil
	})
}
