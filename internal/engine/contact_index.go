package engine

import (
	"container/heap"
	"sort"
	"strings"
	"unicode"

	"github.com/openfabrica/mailfabrica/internal/mail"
	"github.com/openfabrica/mailfabrica/internal/protocol"
)

// indexedContact 将同一邮箱的多个检索词关联到一份联系人和实际往来权重。
type indexedContact struct {
	value mail.Contact
	terms []string
}

// betterContact 按最近往来、次数、手工联系人和邮箱稳定排序，不使用任意混合权重。
func betterContact(a, b *indexedContact) bool {
	if b == nil {
		return true
	}
	if a.value.Last != b.value.Last {
		return a.value.Last > b.value.Last
	}
	if a.value.Count != b.value.Count {
		return a.value.Count > b.value.Count
	}
	if a.value.Manual != b.value.Manual {
		return a.value.Manual
	}
	return strings.ToLower(a.value.Email) < strings.ToLower(b.value.Email)
}

// contactTrieNode 缓存子树最高权重；终端列表只在首次查询该节点时排序。
type contactTrieNode struct {
	children map[rune]*contactTrieNode
	best     *indexedContact
	values   []*indexedContact
	sorted   bool
}

// contactIndex 保留一次数据版本的加权前缀树，查询本身不扫描全部邮件。
type contactIndex struct {
	revision uint64
	root     contactTrieNode
}

// contactTerms 为完整邮箱、姓名词和邮箱各段建立前缀入口；中文姓名支持给定名入口。
func contactTerms(name, email string) []string {
	terms := map[string]bool{}
	add := func(value string) {
		value = strings.ToLower(strings.TrimSpace(value))
		if value != "" {
			terms[value] = true
		}
	}
	add(email)
	if at := strings.LastIndex(email, "@"); at >= 0 {
		add(email[:at])
		add(email[at+1:])
		add(email[at:])
	}
	add(name)
	for _, part := range strings.FieldsFunc(name, func(r rune) bool { return !unicode.IsLetter(r) && !unicode.IsNumber(r) }) {
		add(part)
		runes := []rune(part)
		if len(runes) <= 64 {
			for i, r := range runes {
				if unicode.Is(unicode.Han, r) {
					add(string(runes[i:]))
				}
			}
		}
	}
	for _, part := range strings.FieldsFunc(email, func(r rune) bool { return r == '@' || r == '.' || r == '_' || r == '-' || r == '+' }) {
		add(part)
	}
	result := []string{}
	for term := range terms {
		result = append(result, term)
	}
	sort.Strings(result)
	return result
}

// add 在每级节点保存子树的最佳联系人，供优先队列跳过低权重分支。
func (n *contactTrieNode) add(term string, value *indexedContact) {
	node := n
	for _, r := range term {
		if betterContact(value, node.best) {
			node.best = value
		}
		if node.children == nil {
			node.children = map[rune]*contactTrieNode{}
		}
		if node.children[r] == nil {
			node.children[r] = &contactTrieNode{}
		}
		node = node.children[r]
	}
	if betterContact(value, node.best) {
		node.best = value
	}
	node.values = append(node.values, value)
	node.sorted = false
}

// contactCandidate 代表一个未展开子树或终端列表当前的最高权重候选。
type contactCandidate struct {
	node   *contactTrieNode
	values []*indexedContact
	offset int
	best   *indexedContact
}

// contactQueue 按候选的最高权重执行 best-first 查询。
type contactQueue []contactCandidate

// Len 返回待访问候选数。
func (q contactQueue) Len() int { return len(q) }

// Less 保证最近往来的联系人优先弹出。
func (q contactQueue) Less(i, j int) bool { return betterContact(q[i].best, q[j].best) }

// Swap 交换堆节点。
func (q contactQueue) Swap(i, j int) { q[i], q[j] = q[j], q[i] }

// Push 接收内部类型的候选。
func (q *contactQueue) Push(value any) {
	candidate, ok := value.(contactCandidate)
	if ok {
		*q = append(*q, candidate)
	}
}

// Pop 删除并返回堆末尾节点。
func (q *contactQueue) Pop() any {
	n := len(*q)
	if n == 0 {
		return nil
	}
	v := (*q)[n-1]
	*q = (*q)[:n-1]
	return v
}

// search 从前缀节点按最高权重展开，达到唯一邮箱数量后停止；多词须全部匹配。
func (index *contactIndex) search(query string, limit int) []mail.Contact {
	words := strings.Fields(strings.ToLower(strings.TrimSpace(query)))
	prefix := ""
	for _, word := range words {
		if len(word) > len(prefix) {
			prefix = word
		}
	}
	node := &index.root
	for _, r := range prefix {
		node = node.children[r]
		if node == nil {
			return []mail.Contact{}
		}
	}
	result := []mail.Contact{}
	seen := map[string]bool{}
	if node.best == nil {
		return result
	}
	queue := &contactQueue{}
	heap.Push(queue, contactCandidate{node: node, best: node.best})
	for queue.Len() > 0 && len(result) < limit {
		candidate, ok := heap.Pop(queue).(contactCandidate)
		if !ok {
			continue
		}
		if n := candidate.node; n != nil {
			for _, child := range n.children {
				if child.best != nil {
					heap.Push(queue, contactCandidate{node: child, best: child.best})
				}
			}
			if !n.sorted {
				sort.SliceStable(n.values, func(i, j int) bool { return betterContact(n.values[i], n.values[j]) })
				n.sorted = true
			}
			if len(n.values) > 0 {
				heap.Push(queue, contactCandidate{values: n.values, best: n.values[0]})
			}
			continue
		}
		value := candidate.values[candidate.offset]
		candidate.offset++
		if candidate.offset < len(candidate.values) {
			candidate.best = candidate.values[candidate.offset]
			heap.Push(queue, candidate)
		}
		email := strings.ToLower(value.value.Email)
		if seen[email] {
			continue
		}
		seen[email] = true
		match := true
		for _, word := range words {
			found := false
			for _, term := range value.terms {
				if strings.HasPrefix(term, word) {
					found = true
					break
				}
			}
			if !found {
				match = false
				break
			}
		}
		if match {
			result = append(result, value.value)
		}
	}
	return result
}

// buildContactIndex 合并地址簿与邮件往来；保留人工名称，避免对同一封邮件重复计数。
func buildContactIndex(revision uint64, saved []mail.Contact, accounts []mail.Account, messages []mail.Message) *contactIndex {
	entries := map[string]*indexedContact{}
	observed := map[string]uint32{}
	add := func(name, email string) *indexedContact {
		parsed, err := mail.ParseAddresses(email)
		if err != nil || len(parsed) != 1 {
			return nil
		}
		email = parsed[0].Email
		key := strings.ToLower(email)
		name = protocol.DecodeHeader(name)
		entry := entries[key]
		if entry == nil {
			entry = &indexedContact{value: mail.Contact{Name: name, Email: email}}
			entries[key] = entry
		} else if entry.value.Name == "" && name != "" {
			entry.value.Name = name
		}
		return entry
	}
	for _, contact := range saved {
		if entry := add(contact.Name, contact.Email); entry != nil {
			entry.value.Last = contact.Last
			entry.value.Count = contact.Count
			entry.value.Manual = contact.Manual
		}
	}
	for _, account := range accounts {
		add(account.Name, account.Email)
	}
	for _, message := range messages {
		seen := map[string]bool{}
		values := append([]mail.Address{message.From}, message.To...)
		values = append(values, message.CC...)
		for _, address := range values {
			entry := add(address.Name, address.Email)
			if entry == nil {
				continue
			}
			key := strings.ToLower(entry.value.Email)
			if seen[key] {
				continue
			}
			seen[key] = true
			observed[key]++
			if message.Date > entry.value.Last {
				entry.value.Last = message.Date
			}
		}
	}
	index := &contactIndex{revision: revision}
	for email, entry := range entries {
		if observed[email] > entry.value.Count {
			entry.value.Count = observed[email]
		}
		entry.terms = contactTerms(entry.value.Name, entry.value.Email)
		for _, term := range entry.terms {
			index.root.add(term, entry)
		}
	}
	return index
}
