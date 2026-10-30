package engine

import (
	"context"
	"fmt"
	"github.com/openfabrica/mailfabrica/internal/mail"
	"github.com/openfabrica/mailfabrica/internal/protocol"
	"time"
)

// startSync 为各账户启动独立任务，后台不接触 Store。
func (s *Session) startSync(account string) (any, error) {
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
		e = s.launch(a.ID, func(ctx context.Context) (any, error) { return s.remote.Sync(ctx, a, password, limit, prefetch, known) }, func(v any) error {
			results, ok := v.([]protocol.FolderResult)
			if !ok {
				return fmt.Errorf("同步结果无效")
			}
			folders := []mail.Folder{}
			for _, fr := range results {
				folders = append(folders, fr.Folder)
				if !fr.Folder.Selectable {
					continue
				}
				idx, err := s.Store.Index(a.ID, fr.Folder.Path)
				if err != nil {
					return err
				}
				old := map[uint32]mail.Message{}
				if idx.UIDValidity != 0 && idx.UIDValidity != fr.Folder.UIDValidity {
					for _, m := range idx.Messages {
						delete(s.bodies, m.Key.String())
					}
					if err = s.Store.ResetFolder(a.ID, fr.Folder.Path); err != nil {
						return err
					}
					idx, err = s.Store.Index(a.ID, fr.Folder.Path)
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
						delete(s.bodies, (mail.Key{Account: a.ID, Folder: fr.Folder.Path, UID: uid}).String())
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
				if err = s.Store.SaveIndex(a.ID, fr.Folder.Path); err != nil {
					return err
				}
			}
			if e := s.Store.SetFolders(a.ID, folders); e != nil {
				return e
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
