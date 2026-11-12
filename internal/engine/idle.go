package engine

import (
	"context"
	"errors"
	"github.com/openfabrica/mailfabrica/internal/mail"
	"github.com/openfabrica/mailfabrica/internal/protocol"
	"time"
)

// inboxWatch 绑定连接配置与收件箱路径，配置改变或关闭自动收件时取消旧监听。
type inboxWatch struct {
	account mail.Account
	folder  string
	cancel  context.CancelFunc
}

// reconcileInboxWatches 在会话锁内管理监听，密码只在建立或重连时读取，避免轮询钥匙串。
func (s *Session) reconcileInboxWatches(watches map[string]inboxWatch, changes chan<- string) {
	watcher, supported := s.remote.(protocol.InboxWatcher)
	desired := map[string]inboxWatch{}
	if supported && !s.demo && number(s.prefs, "syncIntervalMinutes", 1) > 0 {
		for _, a := range s.Store.Accounts {
			for _, f := range s.Store.Folders[a.ID] {
				if f.Selectable && f.Role == "inbox" {
					desired[a.ID] = inboxWatch{account: a, folder: f.Path}
					break
				}
			}
		}
	}
	for id, old := range watches {
		next, ok := desired[id]
		if !ok || old.account != next.account || old.folder != next.folder {
			old.cancel()
			delete(watches, id)
		}
	}
	for id, next := range desired {
		if _, ok := watches[id]; ok {
			continue
		}
		ctx, cancel := context.WithCancel(s.ctx)
		next.cancel = cancel
		watches[id] = next
		s.wg.Add(1)
		go s.watchInbox(ctx, watcher, next, changes)
	}
}

// watchInbox 断开后每分钟重连；不支持 IDLE 时等待配置变更或退出，定时同步继续工作。
func (s *Session) watchInbox(ctx context.Context, watcher protocol.InboxWatcher, watch inboxWatch, changes chan<- string) {
	defer s.wg.Done()
	notify := func() {
		select {
		case changes <- watch.account.ID:
		default:
		}
	}
	for {
		if ctx.Err() != nil {
			return
		}
		s.mu.Lock()
		password, err := s.secrets.Get(watch.account.ID)
		s.mu.Unlock()
		if err == nil {
			err = watcher.WatchInbox(ctx, watch.account, password, watch.folder, notify)
		}
		if errors.Is(err, protocol.ErrIdleUnsupported) {
			<-ctx.Done()
			return
		}
		timer := time.NewTimer(time.Minute)
		select {
		case <-ctx.Done():
			timer.Stop()
			return
		case <-timer.C:
		}
	}
}
