package engine

import (
	"context"
	"fmt"
	"github.com/openfabrica/mailfabrica/internal/mail"
	"github.com/openfabrica/mailfabrica/internal/protocol"
	"time"
)

// startSync 为各账户启动独立任务，后台不接触 Store。
func (s *Session) startSync(account string) (any, error) { return s.startSyncMode(account, false) }

// startSyncMode 快速同步只检查收件箱，完整同步核对全部文件夹；同账户共用串行队列。
func (s *Session) startSyncMode(account string, inboxOnly bool) (any, error) {
	return s.startSyncFolders(account, inboxOnly, nil)
}

// startSyncFolders 变更后只核对源目录和目标目录，避免角标等待无关文件夹或正文下载。
func (s *Session) startSyncFolders(account string, inboxOnly bool, paths []string) (any, error) {
	wanted := func(folder mail.Folder) bool {
		if inboxOnly {
			return folder.Role == "inbox"
		}
		if len(paths) == 0 {
			return true
		}
		for _, path := range paths {
			if path == folder.Path {
				return true
			}
		}
		return false
	}
	started := 0
	for _, a := range s.Store.Accounts {
		if account != "" && a.ID != account {
			continue
		}
		if s.busy[a.ID] {
			continue
		}
		password, e := s.secrets.Get(a.ID)
		if e != nil {
			return nil, e
		}
		limit := uint32(number(s.prefs, "syncLimit", 500))
		if limit < 1 {
			limit = 500
		}
		prefetch := uint32(number(s.prefs, "prefetch", 50))
		known := map[string]protocol.KnownFolder{}
		for _, folder := range s.Store.Folders[a.ID] {
			if !wanted(folder) {
				continue
			}
			idx, err := s.Store.Index(a.ID, folder.Path)
			if err != nil {
				return nil, err
			}
			messages := map[uint32]mail.Message{}
			for _, m := range idx.Messages {
				messages[m.Key.UID] = m
			}
			known[folder.Path] = protocol.KnownFolder{UIDValidity: idx.UIDValidity, Messages: messages}
		}

		options := protocol.SyncOptions{Limit: limit, Prefetch: prefetch, Known: known, Folders: append([]mail.Folder{}, s.Store.Folders[a.ID]...), InboxOnly: inboxOnly, FolderPaths: append([]string{}, paths...)}
		if len(paths) > 0 {
			options.Prefetch = 0
		}
		e = s.launch(a.ID, func(ctx context.Context) (any, error) {
			emit := func(update protocol.SyncUpdate) error {
				s.mu.Lock()
				defer s.mu.Unlock()
				if err := s.applySyncFolder(a.ID, update); err != nil {
					return err
				}
				if err := s.refresh(); err != nil {
					return err
				}
				s.touch()
				return nil
			}
			if remote, ok := s.remote.(protocol.StreamingRemote); ok {
				return remote.SyncStream(ctx, a, password, options, emit)
			}
			results, err := s.remote.Sync(ctx, a, password, limit, prefetch, known)
			if err != nil {
				return nil, err
			}
			folders := []mail.Folder{}
			for _, fr := range results {
				if !wanted(fr.Folder) {
					continue
				}
				if err = emit(protocol.SyncUpdate{FolderResult: fr}); err != nil {
					return nil, err
				}
				folders = append(folders, fr.Folder)
			}
			return folders, nil
		}, func(v any) error {
			folders, ok := v.([]mail.Folder)
			if !ok {
				return fmt.Errorf("同步结果无效")
			}
			if !inboxOnly && len(paths) == 0 {
				if err := s.Store.SetFolders(a.ID, folders); err != nil {
					return err
				}
			}
			s.status.LastSync = time.Now().Unix()
			return s.refresh()
		}, a.ID)
		if e != nil {
			return nil, e
		}
		started++
	}
	if started == 0 && account != "" {
		if _, e := s.account(account); e != nil {
			return nil, e
		}
	}
	return Params{"started": started}, nil
}

// openBody 优先读缓存，仅缺少原文时后台下载。
func (s *Session) openBody(k mail.Key) error {
	key := k.String()
	result := s.messageResult(key)
	if result["state"] == "ready" {
		return nil
	}
	if s.loading[key] {
		return nil
	}
	a, e := s.account(k.Account)
	if e != nil {
		return e
	}
	password, e := s.secrets.Get(a.ID)
	if e != nil {
		return e
	}
	s.loading[key] = true
	delete(s.bodyErrors, key)
	return s.launch("body:"+key, func(ctx context.Context) (any, error) {
		raw, err := s.remote.Fetch(ctx, a, password, k)
		if err != nil {
			s.mu.Lock()
			delete(s.loading, key)
			s.bodyErrors[key] = err.Error()
			s.mu.Unlock()
		}
		return raw, err
	}, func(v any) error {
		delete(s.loading, key)
		raw, ok := v.([]byte)
		if !ok {
			return fmt.Errorf("原文结果无效")
		}
		b, e := protocol.Parse(raw)
		if e != nil {
			s.bodyErrors[key] = e.Error()
			return e
		}
		if e = s.Store.SaveRaw(k, raw); e != nil {
			return e
		}
		s.bodies[key] = b
		idx, e := s.Store.Index(k.Account, k.Folder)
		if e != nil {
			return e
		}
		for i, m := range idx.Messages {
			if m.Key == k {
				idx.Messages[i] = protocol.Metadata(k, b, len(raw), m.Flags)
			}
		}
		if e = s.Store.SaveIndex(k.Account, k.Folder); e != nil {
			return e
		}
		return s.refresh()
	}, k.Account)
}

// applySyncFolder 在会话锁内逐文件夹发布邮件头；正文补充不覆盖期间产生的标记。
func (s *Session) applySyncFolder(account string, update protocol.SyncUpdate) error {
	fr := update.FolderResult
	if update.BodyOnly {
		idx, err := s.Store.Index(account, fr.Folder.Path)
		if err != nil {
			return err
		}
		if idx.UIDValidity != fr.Folder.UIDValidity {
			return nil
		}
		for i, m := range idx.Messages {
			raw, ok := fr.Raw[m.Key.UID]
			if !ok {
				continue
			}
			b, err := protocol.Parse(raw)
			if err != nil {
				return err
			}
			if err = s.Store.SaveRaw(m.Key, raw); err != nil {
				return err
			}
			idx.Messages[i] = protocol.Metadata(m.Key, b, len(raw), m.Flags)
			s.bodies[m.Key.String()] = b
		}
		return s.Store.SaveIndex(account, fr.Folder.Path)
	}
	folders := append([]mail.Folder{}, s.Store.Folders[account]...)
	found := false
	for i, f := range folders {
		if f.Path == fr.Folder.Path {
			folders[i] = fr.Folder
			found = true
			break
		}
	}
	if !found {
		folders = append(folders, fr.Folder)
	}
	if err := s.Store.SetFolders(account, folders); err != nil {
		return err
	}
	if !fr.Folder.Selectable {
		return nil
	}
	idx, err := s.Store.Index(account, fr.Folder.Path)
	if err != nil {
		return err
	}
	old := map[uint32]mail.Message{}
	if idx.UIDValidity != 0 && idx.UIDValidity != fr.Folder.UIDValidity {
		for _, m := range idx.Messages {
			delete(s.bodies, m.Key.String())
		}
		if err = s.Store.ResetFolder(account, fr.Folder.Path); err != nil {
			return err
		}
		idx, err = s.Store.Index(account, fr.Folder.Path)
		if err != nil {
			return err
		}
	}
	if idx.UIDValidity == fr.Folder.UIDValidity {
		for _, m := range idx.Messages {
			old[m.Key.UID] = m
		}
	}
	remoteUIDs := map[uint32]bool{}
	for _, uid := range fr.UIDs {
		remoteUIDs[uid] = true
	}
	for uid := range old {
		if !remoteUIDs[uid] {
			delete(old, uid)
			delete(s.bodies, (mail.Key{Account: account, Folder: fr.Folder.Path, UID: uid}).String())
		}
	}
	for _, m := range fr.Messages {
		m = protocol.DecodeMetadata(m)
		if prior, ok := old[m.Key.UID]; ok {
			m.Preview = prior.Preview
			m.HasAttachments = prior.HasAttachments
		}
		if raw, ok := fr.Raw[m.Key.UID]; ok {
			b, err := protocol.Parse(raw)
			if err != nil {
				return err
			}
			m = protocol.Metadata(m.Key, b, len(raw), m.Flags)
			if err = s.Store.SaveRaw(m.Key, raw); err != nil {
				return err
			}
			s.bodies[m.Key.String()] = b
		}
		old[m.Key.UID] = m
	}
	idx.Messages = []mail.Message{}
	idx.UIDValidity = fr.Folder.UIDValidity
	for _, m := range old {
		idx.Messages = append(idx.Messages, m)
	}
	if err = s.Store.SaveIndex(account, fr.Folder.Path); err != nil {
		return err
	}
	return nil
}
