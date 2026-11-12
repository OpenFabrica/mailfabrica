package protocol

import (
	"context"
	"fmt"
	"github.com/emersion/go-imap"
	"github.com/openfabrica/mailfabrica/internal/mail"
	"sort"
	"strings"
)

// Sync 保留一次性同步接口，供命令行、测试和非流式调用使用。
func (n Network) Sync(ctx context.Context, a mail.Account, p string, limit, prefetch uint32, known ...map[string]KnownFolder) ([]FolderResult, error) {
	options := SyncOptions{Limit: limit, Prefetch: prefetch}
	if len(known) > 0 {
		options.Known = known[0]
	}
	results := []FolderResult{}
	positions := map[string]int{}
	_, err := n.SyncStream(ctx, a, p, options, func(update SyncUpdate) error {
		if !update.BodyOnly {
			i, ok := positions[update.Folder.Path]
			if !ok {
				i = len(results)
				positions[update.Folder.Path] = i
				results = append(results, FolderResult{Messages: []mail.Message{}, Raw: map[uint32][]byte{}})
			}
			result := &results[i]
			result.Folder, result.UIDs = update.Folder, update.UIDs
			byUID := map[uint32]int{}
			for i, message := range result.Messages {
				byUID[message.Key.UID] = i
			}
			for _, message := range update.Messages {
				if i, ok := byUID[message.Key.UID]; ok {
					result.Messages[i] = message
				} else {
					byUID[message.Key.UID] = len(result.Messages)
					result.Messages = append(result.Messages, message)
				}
			}
			return nil
		}
		i, ok := positions[update.Folder.Path]
		if !ok {
			return fmt.Errorf("正文缺少邮件头")
		}
		for uid, raw := range update.Raw {
			results[i].Raw[uid] = raw
		}
		return nil
	})
	return results, err
}

// SyncStream 优先收件箱，逐文件夹发布头部快照，再补充正文；快速模式不刷新历史标记。
func (Network) SyncStream(ctx context.Context, a mail.Account, password string, options SyncOptions, emit func(SyncUpdate) error) ([]mail.Folder, error) {
	limit, prefetch := options.Limit, options.Prefetch
	c, close, e := dialIMAP(ctx, a, password)
	if e != nil {
		return nil, e
	}
	defer close()

	wanted := func(f mail.Folder) bool {
		if options.InboxOnly {
			return f.Role == "inbox"
		}
		if len(options.FolderPaths) == 0 {
			return true
		}
		for _, path := range options.FolderPaths {
			if f.Path == path {
				return true
			}
		}
		return false
	}
	list := []*imap.MailboxInfo{}
	if options.InboxOnly || len(options.FolderPaths) > 0 {
		for _, f := range options.Folders {
			if f.Selectable && wanted(f) {
				list = append(list, &imap.MailboxInfo{Name: f.Path})
			}
		}
	}
	missingPath := false
	for _, path := range options.FolderPaths {
		if path == "" {
			continue
		}
		found := false
		for _, f := range list {
			if f.Name == path {
				found = true
				break
			}
		}
		if !found {
			missingPath = true
			break
		}
	}
	if (!options.InboxOnly && len(options.FolderPaths) == 0) || len(list) == 0 || missingPath {
		list = []*imap.MailboxInfo{}
		ch := make(chan *imap.MailboxInfo, 32)
		done := make(chan error, 1)
		go func() { done <- c.List("", "*", ch) }()
		listed := 0
		for f := range ch {
			listed++
			if listed > 2000 {
				c.Terminate()
				return nil, fmt.Errorf("文件夹超过 2000 个")
			}
			if wanted(mail.Folder{Path: f.Name, Role: mail.FolderRole(f.Name, f.Attributes)}) {
				list = append(list, f)
			}
		}
		if e = <-done; e != nil {
			return nil, e
		}
	}
	sort.SliceStable(list, func(i, j int) bool {
		return mail.FolderRole(list[i].Name, list[i].Attributes) == "inbox" && mail.FolderRole(list[j].Name, list[j].Attributes) != "inbox"
	})
	done := make(chan error, 1)
	out := []mail.Folder{}
	for _, f := range list {
		if e = ctx.Err(); e != nil {
			return nil, e
		}
		selectable := true
		for _, attr := range f.Attributes {
			if strings.EqualFold(attr, imap.NoSelectAttr) {
				selectable = false
			}
		}
		delim := f.Delimiter
		fr := FolderResult{Folder: mail.Folder{Account: a.ID, Path: f.Name, Delimiter: &delim, Role: mail.FolderRole(f.Name, f.Attributes), Selectable: selectable}, Messages: []mail.Message{}, UIDs: []uint32{}, Raw: map[uint32][]byte{}}

		if options.InboxOnly || len(options.FolderPaths) > 0 {
			for _, known := range options.Folders {
				if known.Path == f.Name {
					fr.Folder = known
					break
				}
			}
		}
		if !selectable {
			if err := emit(SyncUpdate{FolderResult: fr}); err != nil {
				return nil, err
			}
			out = append(out, fr.Folder)
			continue
		}
		status, err := c.Select(f.Name, false)
		if err != nil {
			return nil, err
		}
		fr.Folder.UIDValidity = status.UidValidity
		fr.Folder.Total = status.Messages
		fr.Folder.Unread = 0
		ids, err := c.UidSearch(imap.NewSearchCriteria())
		if err != nil {
			return nil, err
		}
		fr.UIDs = ids
		if len(ids) > 2000000 {
			return nil, fmt.Errorf("文件夹邮件过多")
		}
		start := len(ids) - int(limit)
		if start < 0 {
			start = 0
		}
		if len(ids) == 0 {
			if err := emit(SyncUpdate{FolderResult: fr}); err != nil {
				return nil, err
			}
			out = append(out, fr.Folder)
			continue
		}
		newIDs := append([]uint32{}, ids[start:]...)
		prior := options.Known[f.Name]
		fr.Folder.Unread = 0
		if prior.UIDValidity == status.UidValidity {
			newIDs = []uint32{}
			// 按最近窗口内缺失的 UID 补取，首批成功后断线也不会跳过较早的未完成批次。
			for i, uid := range ids {
				meta, known := prior.Messages[uid]
				if known && !meta.Flags.Seen {
					fr.Folder.Unread++
				}
				if !known && i >= start {
					newIDs = append(newIDs, uid)
				}
			}
		}

		if len(newIDs) > int(limit) && limit > 0 {
			newIDs = newIDs[len(newIDs)-int(limit):]
		}
		// 先核对删除与 UIDVALIDITY，保留已有索引；新邮件与历史标记随后按批合并。
		if err := emit(SyncUpdate{FolderResult: fr}); err != nil {
			return nil, err
		}
		for end := len(newIDs); end > 0; {
			start := max(0, end-100)

			set := new(imap.SeqSet)
			set.AddNum(newIDs[start:end]...)
			messages := make(chan *imap.Message, 32)
			go func() {
				done <- c.UidFetch(set, []imap.FetchItem{imap.FetchUid, imap.FetchEnvelope, imap.FetchFlags, imap.FetchRFC822Size}, messages)
			}()
			for m := range messages {
				if m.Envelope == nil {
					continue
				}
				env := m.Envelope
				convert := func(a []*imap.Address) []mail.Address {
					out := []mail.Address{}
					for _, v := range a {
						out = append(out, mail.Address{Name: DecodeHeader(v.PersonalName), Email: v.Address()})
					}
					return out
				}
				from := mail.Address{}
				if v := convert(env.From); len(v) > 0 {
					from = v[0]
				}
				meta := mail.Message{Key: mail.Key{Account: a.ID, Folder: f.Name, UID: m.Uid}, Subject: DecodeHeader(env.Subject), MessageID: env.MessageId, From: from, To: convert(env.To), CC: convert(env.Cc), Date: env.Date.Unix(), Size: m.Size, References: []string{}}
				for _, v := range m.Flags {
					switch strings.ToLower(v) {
					case "\\seen":
						meta.Flags.Seen = true
					case "\\flagged":
						meta.Flags.Flagged = true
					case "\\answered":
						meta.Flags.Answered = true
					case "\\draft":
						meta.Flags.Draft = true
					case "\\deleted":
						meta.Flags.Deleted = true
					}
				}
				if !meta.Flags.Seen {
					fr.Folder.Unread++
				}
				fr.Messages = append(fr.Messages, meta)
			}
			if e = <-done; e != nil {
				return nil, e
			}

			// 每批最多 100 封，较新的邮件先显示；不等整文件夹或正文下载。
			if err := emit(SyncUpdate{FolderResult: fr}); err != nil {
				return nil, err
			}
			fr.Messages = []mail.Message{}
			end = start
		}

		if !options.InboxOnly && prior.UIDValidity == status.UidValidity {
			// 已知邮件仅刷新标记；新邮件才获取 Envelope 和正文，避免反复下载。
			knownIDs := []uint32{}
			for _, uid := range ids {
				if _, ok := prior.Messages[uid]; ok {
					knownIDs = append(knownIDs, uid)
				}
			}
			for offset := 0; offset < len(knownIDs); offset += 100 {
				end := offset + 100
				if end > len(knownIDs) {
					end = len(knownIDs)
				}
				flagsSet := new(imap.SeqSet)
				flagsSet.AddNum(knownIDs[offset:end]...)
				flagMessages := make(chan *imap.Message, 32)
				go func() { done <- c.UidFetch(flagsSet, []imap.FetchItem{imap.FetchUid, imap.FetchFlags}, flagMessages) }()
				for m := range flagMessages {
					meta, ok := prior.Messages[m.Uid]
					if ok {
						oldUnread := !meta.Flags.Seen
						meta.Flags = flagsFromIMAP(m.Flags)
						if oldUnread && meta.Flags.Seen {
							fr.Folder.Unread--
						}
						if !oldUnread && !meta.Flags.Seen {
							fr.Folder.Unread++
						}
						fr.Messages = append(fr.Messages, meta)
					}
				}
				if e = <-done; e != nil {
					return nil, e
				}
				if err := emit(SyncUpdate{FolderResult: fr}); err != nil {
					return nil, err
				}
				fr.Messages = []mail.Message{}
			}
		}
		n := int(prefetch)
		if n > len(newIDs) {
			n = len(newIDs)
		}
		total := 0
		for _, uid := range newIDs[len(newIDs)-n:] {
			raw, err := fetch(c, uid)
			if err != nil {
				return nil, err
			}
			total += len(raw)
			if total > 200<<20 {
				return nil, fmt.Errorf("单文件夹预取超过 200 MiB")
			}
			body := FolderResult{Folder: fr.Folder, Raw: map[uint32][]byte{uid: raw}}
			if err := emit(SyncUpdate{FolderResult: body, BodyOnly: true}); err != nil {
				return nil, err
			}
		}
		out = append(out, fr.Folder)
	}
	return out, nil
}
