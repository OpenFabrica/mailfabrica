package engine

import (
	"fmt"
	"sort"
	"strings"

	"github.com/openfabrica/mailfabrica/internal/mail"
)

// ContactTimelineItem 是一封往来邮件及其本地副本位置，不下载正文或改变阅读状态。
type ContactTimelineItem struct {
	Message   mail.Message `json:"message"`
	Direction string       `json:"direction"`
	Locations []mail.Key   `json:"locations"`
}

// ContactTimeline 返回去重后的往来记录与按同一账户范围计算的总数。
type ContactTimeline struct {
	Email    string                `json:"email"`
	Account  string                `json:"account"`
	Total    int                   `json:"total"`
	Received int                   `json:"received"`
	Sent     int                   `json:"sent"`
	Related  int                   `json:"related"`
	Offset   int                   `json:"offset"`
	HasMore  bool                  `json:"hasMore"`
	Items    []ContactTimelineItem `json:"items"`
}

// contactTimeline 查询已缓存的收发与共同收件记录，同一 Message-ID 的跨文件夹副本只显示一次。
func (s *Session) contactTimeline(p Params) (any, error) {
	email, err := required(p, "email")
	if err != nil {
		return nil, err
	}
	addresses, err := mail.ParseAddresses(email)
	if err != nil || len(addresses) != 1 {
		return nil, fmt.Errorf("请输入一个有效的联系人邮箱")
	}
	email = strings.ToLower(addresses[0].Email)
	account := ""
	if v, ok := p["account"]; ok {
		var valid bool
		account, valid = v.(string)
		if !valid {
			return nil, fmt.Errorf("account 必须是字符串")
		}
	}
	if account != "" {
		if _, err = s.account(account); err != nil {
			return nil, err
		}
	}
	limit, offset := 50, 0
	for key, target := range map[string]*int{"limit": &limit, "offset": &offset} {
		if v, ok := p[key]; ok {
			n, valid := v.(float64)
			if !valid || n < 0 || n > 1000000 || n != float64(int(n)) {
				return nil, fmt.Errorf("%s 必须是有效整数", key)
			}
			*target = int(n)
		}
	}
	if limit < 1 || limit > 100 {
		return nil, fmt.Errorf("每页数量必须为 1–100")
	}
	messages, err := s.allMessages()
	if err != nil {
		return nil, err
	}
	own := map[string]bool{}
	roles := map[string]string{}
	for _, a := range s.Store.Accounts {
		own[strings.ToLower(a.Email)] = true
		for _, f := range s.Store.Folders[a.ID] {
			roles[a.ID+"\x00"+f.Path] = f.Role
		}
	}
	entries := map[string]*ContactTimelineItem{}
	for _, m := range messages {
		if account != "" && m.Key.Account != account {
			continue
		}
		role := roles[m.Key.Account+"\x00"+m.Key.Folder]
		if m.Flags.Draft || m.Flags.Deleted || role == "drafts" || role == "trash" || role == "junk" {
			continue
		}
		from := strings.EqualFold(m.From.Email, email)
		recipient := false
		for _, list := range [][]mail.Address{m.To, m.CC} {
			for _, a := range list {
				if strings.EqualFold(a.Email, email) {
					recipient = true
				}
			}
		}
		if !from && !recipient {
			continue
		}
		direction := "related"
		if own[strings.ToLower(m.From.Email)] && recipient {
			direction = "outgoing"
		} else if from {
			direction = "incoming"
		}
		id := strings.TrimSpace(m.MessageID)
		if id == "" {
			id = m.Key.String()
		}
		entry, ok := entries[id]
		if !ok {
			entries[id] = &ContactTimelineItem{Message: m, Direction: direction, Locations: []mail.Key{m.Key}}
			continue
		}
		entry.Locations = append(entry.Locations, m.Key)
		// 发件优先定位已发送副本，其余优先收件箱；相同角色以邮件键稳定选择。
		rank := func(message mail.Message) int {
			r := roles[message.Key.Account+"\x00"+message.Key.Folder]
			if direction == "outgoing" && r == "sent" {
				return 0
			}
			if r == "inbox" {
				return 1
			}
			return 2
		}
		if rank(m) < rank(entry.Message) || (rank(m) == rank(entry.Message) && m.Key.String() < entry.Message.Key.String()) {
			entry.Message = m
			entry.Direction = direction
		}
	}
	result := ContactTimeline{Email: email, Account: account, Offset: offset, Items: []ContactTimelineItem{}}
	rows := make([]ContactTimelineItem, 0, len(entries))
	for _, entry := range entries {
		sort.Slice(entry.Locations, func(i, j int) bool { return entry.Locations[i].String() < entry.Locations[j].String() })
		rows = append(rows, *entry)
		switch entry.Direction {
		case "incoming":
			result.Received++
		case "outgoing":
			result.Sent++
		default:
			result.Related++
		}
	}
	sort.Slice(rows, func(i, j int) bool {
		if rows[i].Message.Date == rows[j].Message.Date {
			return rows[i].Message.Key.String() < rows[j].Message.Key.String()
		}
		return rows[i].Message.Date > rows[j].Message.Date
	})
	result.Total = len(rows)
	if offset < len(rows) {
		end := offset + limit
		if end > len(rows) {
			end = len(rows)
		}
		result.Items = rows[offset:end]
		result.HasMore = end < len(rows)
	}
	return result, nil
}
