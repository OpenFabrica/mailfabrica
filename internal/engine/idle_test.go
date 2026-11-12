package engine

import (
	"context"
	"github.com/openfabrica/mailfabrica/internal/mail"
	"github.com/openfabrica/mailfabrica/internal/platform"
	"github.com/openfabrica/mailfabrica/internal/protocol"
	"testing"
	"time"
)

// watchRemote 记录监听建立和取消，不访问网络或真实钥匙串。
type watchRemote struct {
	protocol.Remote
	started chan struct{}
	stopped chan struct{}
}

// WatchInbox 发送一次变化后等待取消，覆盖后台监听与同步队列的边界。
func (r *watchRemote) WatchInbox(ctx context.Context, _ mail.Account, _, _ string, notify func()) error {
	r.started <- struct{}{}
	notify()
	<-ctx.Done()
	r.stopped <- struct{}{}
	return ctx.Err()
}

// TestInboxWatcherLifecycle 验证重复调度不重复连接、变化入队、关闭自动收件会取消连接。
func TestInboxWatcherLifecycle(t *testing.T) {
	s, err := New(t.TempDir(), false)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(s.Close)
	s.mu.Lock()
	s.secrets = platform.NewSecrets(s.Store.Root, true)
	err = s.seedDemo()
	if err != nil {
		s.mu.Unlock()
		t.Fatal(err)
	}
	remote := &watchRemote{Remote: s.remote, started: make(chan struct{}, 4), stopped: make(chan struct{}, 4)}
	s.remote = remote
	watches := map[string]inboxWatch{}
	changes := make(chan string, 4)
	s.reconcileInboxWatches(watches, changes)
	s.reconcileInboxWatches(watches, changes)
	if len(watches) != 1 {
		s.mu.Unlock()
		t.Fatal("duplicate watchers")
	}
	s.mu.Unlock()
	select {
	case <-remote.started:
	case <-time.After(time.Second):
		t.Fatal("watcher not started")
	}
	select {
	case account := <-changes:
		if account != "demo-mailfabrica-dev" {
			t.Fatal(account)
		}
	case <-time.After(time.Second):
		t.Fatal("change not queued")
	}
	s.mu.Lock()
	s.prefs["syncIntervalMinutes"] = float64(0)
	s.reconcileInboxWatches(watches, changes)
	count := len(watches)
	s.mu.Unlock()
	if count != 0 {
		t.Fatal("disabled watcher retained")
	}
	select {
	case <-remote.stopped:
	case <-time.After(time.Second):
		t.Fatal("watcher not canceled")
	}
}
