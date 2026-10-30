package engine

import (
	"fmt"
	"strings"
)

// suggestContacts 从地址簿和本地邮件往来中按名称或邮箱匹配，不把历史地址写入联系人。
func (s *Session) suggestContacts(p Params) (any, error) {
	if value, ok := p["query"]; ok {
		if _, ok = value.(string); !ok {
			return nil, fmt.Errorf("query 必须是字符串")
		}
	}
	query := strings.ToLower(strings.TrimSpace(str(p, "query")))
	if len(query) > 1000 {
		return nil, fmt.Errorf("联系人搜索内容过长")
	}
	limit := number(p, "limit", 12)
	if limit < 1 || limit > 100 {
		return nil, fmt.Errorf("联系人候选数量必须为 1–100")
	}
	if s.contactIndex == nil || s.contactIndex.revision != s.Store.Revision() {
		messages, err := s.allMessages()
		if err != nil {
			return nil, err
		}
		s.contactIndex = buildContactIndex(s.Store.Revision(), s.Store.Contacts, s.Store.Accounts, messages)
	}
	return s.contactIndex.search(query, limit), nil
}
