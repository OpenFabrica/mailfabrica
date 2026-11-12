package engine

import (
	"context"
	"fmt"
	"github.com/openfabrica/mailfabrica/internal/mail"
	"github.com/openfabrica/mailfabrica/internal/protocol"
	"testing"
	"time"
)

// slowBodyRemote 在邮件头可用后阻塞正文，检验列表发布时间。
type slowBodyRemote struct {
	protocol.Remote
	headers chan struct{}
	release chan struct{}
}

// Sync 模拟旧的一次性同步：只有正文完成后才返回邮件头。
func (r *slowBodyRemote) Sync(ctx context.Context, a mail.Account, p string, l, n uint32, k ...map[string]protocol.KnownFolder) ([]protocol.FolderResult, error) {
	close(r.headers)
	select {
	case <-r.release:
	case <-ctx.Done():
		return nil, ctx.Err()
	}
	return r.Remote.Sync(ctx, a, p, l, n, k...)
}

// SyncStream 模拟分阶段同步：先发布邮件头，再等待正文。
func (r *slowBodyRemote) SyncStream(ctx context.Context, a mail.Account, p string, o protocol.SyncOptions, emit func(protocol.SyncUpdate) error) ([]mail.Folder, error) {
	results, err := r.Remote.Sync(ctx, a, p, o.Limit, 0, o.Known)
	if err != nil {
		return nil, err
	}
	folders := []mail.Folder{}
	for _, fr := range results {
		folders = append(folders, fr.Folder)
		fr.Raw = nil
		if err = emit(protocol.SyncUpdate{FolderResult: fr}); err != nil {
			return nil, err
		}
	}
	close(r.headers)
	select {
	case <-r.release:
	case <-ctx.Done():
		return nil, ctx.Err()
	}
	return folders, nil
}

// TestSyncPublishesHeadersBeforeBody 验证正文尚未完成时，新邮件元数据已经进入列表。
func TestSyncPublishesHeadersBeforeBody(t *testing.T) {
	s := testSession(t)
	remote := s.remote.(*protocol.Memory)
	remote.Put("demo-mailfabrica-dev", "INBOX", 100, []byte("From: new@example.com\r\nSubject: arriving now\r\n\r\nbody"), mail.Flags{})
	slow := &slowBodyRemote{Remote: remote, headers: make(chan struct{}), release: make(chan struct{})}
	defer close(slow.release)
	s.remote = slow
	started := time.Now()
	if _, err := s.Execute("mail.sync", marshalParams(Params{})); err != nil {
		t.Fatal(err)
	}
	select {
	case <-slow.headers:
	case <-time.After(time.Second):
		t.Fatal("sync did not start")
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	idx, err := s.Store.Index("demo-mailfabrica-dev", "INBOX")
	if err != nil {
		t.Fatal(err)
	}
	for _, m := range idx.Messages {
		if m.Key.UID == 100 {
			if !s.busy["demo-mailfabrica-dev"] {
				t.Fatal("正文应仍在下载")
			}
			t.Logf("新邮件头在 %s 内可见，正文仍被阻塞", time.Since(started))
			return
		}
	}
	t.Fatal(fmt.Sprintf("新邮件标题仍在等待正文下载，当前只有 %d 封", len(idx.Messages)))
}

// TestSyncSchedule 验证一分钟快速检查、十分钟完整核对及关闭自动收件。
func TestSyncSchedule(t *testing.T) {
	now := time.Now()
	for _, tc := range []struct {
		name          string
		quick, full   time.Duration
		minutes       int
		due, wantFull bool
	}{
		{"not due", 30 * time.Second, time.Minute, 1, false, false},
		{"quick", time.Minute, time.Minute, 1, true, false},
		{"full", time.Minute, 10 * time.Minute, 1, true, true},
		{"disabled", time.Hour, time.Hour, 0, false, false},
		{"custom interval", time.Minute, time.Hour, 5, false, false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			due, full := syncPlan(now, now.Add(-tc.quick), now.Add(-tc.full), tc.minutes)
			if due != tc.due || full != tc.wantFull {
				t.Fatalf("%v %v", due, full)
			}
		})
	}
}

// TestQuickSyncPreservesOtherFolders 验证快速收件不扫描或丢弃其它文件夹，完整同步才更新其邮件。
func TestQuickSyncPreservesOtherFolders(t *testing.T) {
	s := testSession(t)
	r := s.remote.(*protocol.Memory)
	const account = "demo-mailfabrica-dev"
	r.Put(account, "INBOX", 100, []byte("Subject: inbox\r\n\r\ninbox"), mail.Flags{})
	r.Put(account, "Archive", 101, []byte("Subject: archive\r\n\r\narchive"), mail.Flags{})
	s.mu.Lock()
	_, err := s.startSyncMode(account, true)
	s.mu.Unlock()
	if err != nil {
		t.Fatal(err)
	}
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	if err = s.Wait(ctx); err != nil {
		t.Fatal(err)
	}
	s.mu.Lock()
	inbox, _ := s.Store.Index(account, "INBOX")
	archive, _ := s.Store.Index(account, "Archive")
	count, archiveCount, folderCount := len(inbox.Messages), len(archive.Messages), len(s.Store.Folders[account])
	s.mu.Unlock()
	if count != 12 || archiveCount != 0 || folderCount != 7 {
		t.Fatalf("quick inbox=%d archive=%d folders=%d", count, archiveCount, folderCount)
	}
	executeTest(t, s, "mail.sync", Params{})
	s.mu.Lock()
	defer s.mu.Unlock()
	archive, err = s.Store.Index(account, "Archive")
	if err != nil || len(archive.Messages) != 1 {
		t.Fatalf("full sync: %v", err)
	}
}

// TestBodyPatchPreservesFlags 验证预取正文不会覆盖用户刚设置的已读状态，也不删除其它邮件。
func TestBodyPatchPreservesFlags(t *testing.T) {
	s := testSession(t)
	s.mu.Lock()
	defer s.mu.Unlock()
	idx, err := s.Store.Index("demo-mailfabrica-dev", "INBOX")
	if err != nil {
		t.Fatal(err)
	}
	idx.Messages[0].Flags.Seen = true
	key := idx.Messages[0].Key
	raw, err := s.Store.Raw(key)
	if err != nil {
		t.Fatal(err)
	}
	err = s.applySyncFolder(key.Account, protocol.SyncUpdate{BodyOnly: true, FolderResult: protocol.FolderResult{Folder: mail.Folder{Path: key.Folder, UIDValidity: idx.UIDValidity}, Raw: map[uint32][]byte{key.UID: raw}}})
	if err != nil || len(idx.Messages) != 11 || !idx.Messages[0].Flags.Seen {
		t.Fatalf("body patch changed state: %v", err)
	}
}

// headerFailureRemote 在发布收件箱后模拟后续文件夹断线。
type headerFailureRemote struct{ protocol.Remote }

// SyncStream 将一份已取到的头部交给引擎后返回错误。
func (r headerFailureRemote) SyncStream(ctx context.Context, a mail.Account, p string, o protocol.SyncOptions, emit func(protocol.SyncUpdate) error) ([]mail.Folder, error) {
	results, err := r.Remote.Sync(ctx, a, p, o.Limit, 0, o.Known)
	if err != nil {
		return nil, err
	}
	for _, fr := range results {
		if fr.Folder.Role == "inbox" {
			fr.Raw = nil
			if err = emit(protocol.SyncUpdate{FolderResult: fr}); err != nil {
				return nil, err
			}
			break
		}
	}
	return nil, fmt.Errorf("后续文件夹连接中断")
}

// TestSyncFailureKeepsPublishedHeaders 验证其它文件夹失败时已显示的新邮件仍然可见且错误不被吞掉。
func TestSyncFailureKeepsPublishedHeaders(t *testing.T) {
	s := testSession(t)
	s.remote.(*protocol.Memory).Put("demo-mailfabrica-dev", "INBOX", 100, []byte("Subject: new\r\n\r\nbody"), mail.Flags{})
	s.remote = headerFailureRemote{s.remote}
	executeTest(t, s, "mail.sync", Params{})
	s.mu.Lock()
	defer s.mu.Unlock()
	idx, err := s.Store.Index("demo-mailfabrica-dev", "INBOX")
	if err != nil || len(idx.Messages) != 12 || len(s.status.Errors) == 0 {
		t.Fatalf("published mail or error lost: %v", err)
	}
}

// TestSyncBatchesUpdateCounts 验证缓存计数逐批合并，重复批次不累加，删除与已读变化按当前索引计算。
func TestSyncBatchesUpdateCounts(t *testing.T) {
	s := testSession(t)
	s.mu.Lock()
	defer s.mu.Unlock()
	const account = "demo-mailfabrica-dev"
	idx, err := s.Store.Index(account, "INBOX")
	if err != nil {
		t.Fatal(err)
	}
	baseline := s.counts.Unread
	uids := []uint32{}
	for _, message := range idx.Messages {
		uids = append(uids, message.Key.UID)
	}
	folder := s.Store.Folders[account][0]
	for _, f := range s.Store.Folders[account] {
		if f.Path == "INBOX" {
			folder = f
		}
	}
	uids = append(uids, 100, 101)
	first := protocol.SyncUpdate{FolderResult: protocol.FolderResult{Folder: folder, UIDs: uids, Messages: []mail.Message{{Key: mail.Key{Account: account, Folder: "INBOX", UID: 100}, Subject: "first increment"}}}}
	for i := 0; i < 2; i++ {
		if err = s.applySyncFolder(account, first); err != nil {
			t.Fatal(err)
		}
		if err = s.refresh(); err != nil {
			t.Fatal(err)
		}
		if s.counts.Unread != baseline+1 {
			t.Fatalf("首批/重复批计数: %d", s.counts.Unread)
		}
	}
	second := first
	second.Messages = []mail.Message{{Key: mail.Key{Account: account, Folder: "INBOX", UID: 101}, Subject: "second increment"}}
	if err = s.applySyncFolder(account, second); err != nil {
		t.Fatal(err)
	}
	if err = s.refresh(); err != nil {
		t.Fatal(err)
	}
	if s.counts.Unread != baseline+2 || len(idx.Messages) != 13 {
		t.Fatalf("增量丢失旧邮件: count=%d messages=%d", s.counts.Unread, len(idx.Messages))
	}
	second.Messages[0].Flags.Seen = true
	second.UIDs = second.UIDs[:len(second.UIDs)-2]
	second.UIDs = append(second.UIDs, 101)
	if err = s.applySyncFolder(account, second); err != nil {
		t.Fatal(err)
	}
	if err = s.refresh(); err != nil {
		t.Fatal(err)
	}
	if s.counts.Unread != baseline || len(idx.Messages) != 12 {
		t.Fatalf("删除/已读计数: count=%d messages=%d", s.counts.Unread, len(idx.Messages))
	}
}

// scopedMutationRemote 要求删除后核对只包含源和目标目录，不再全量获取其它目录或正文。
type scopedMutationRemote struct {
	protocol.Remote
	checked bool
}

// SyncStream 通过同步选项验证真实变更后的工作量，并用内存服务器返回结果。
func (r *scopedMutationRemote) SyncStream(ctx context.Context, a mail.Account, p string, o protocol.SyncOptions, emit func(protocol.SyncUpdate) error) ([]mail.Folder, error) {
	if o.Prefetch != 0 || len(o.FolderPaths) != 2 {
		return nil, fmt.Errorf("删除后仍在全量同步: %+v", o.FolderPaths)
	}
	paths := map[string]bool{}
	for _, path := range o.FolderPaths {
		paths[path] = true
	}
	if !paths["INBOX"] || !paths["Trash"] {
		return nil, fmt.Errorf("核对范围错误: %v", paths)
	}
	r.checked = true
	return r.Remote.(protocol.StreamingRemote).SyncStream(ctx, a, p, o, emit)
}

// TestDeleteReconcilesOnlyAffectedFolders 验证删除不等无关文件夹的全量拉取，同时保留目录列表和计数。
func TestDeleteReconcilesOnlyAffectedFolders(t *testing.T) {
	s := testSession(t)
	remote := &scopedMutationRemote{Remote: s.remote}
	s.remote = remote
	executeTest(t, s, "message.delete", Params{"keys": []any{"demo-mailfabrica-dev/1/INBOX"}})
	s.mu.Lock()
	defer s.mu.Unlock()
	if !remote.checked || len(s.status.Errors) != 0 || len(s.Store.Folders["demo-mailfabrica-dev"]) != 7 {
		t.Fatalf("局部核对失败: checked=%v errors=%v", remote.checked, s.status.Errors)
	}
}
