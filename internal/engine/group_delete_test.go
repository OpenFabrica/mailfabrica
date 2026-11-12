package engine

import (
	"context"
	"encoding/json"
	"fmt"
	"github.com/openfabrica/mailfabrica/internal/mail"
	"github.com/openfabrica/mailfabrica/internal/protocol"
	"testing"
)

// partialGroupRemote 模拟大组第一批 500 封已移动、后续批次连接失败。
type partialGroupRemote struct{ protocol.Remote }

// MoveMany 复现服务器已部分成功而整个批量调用返回错误的真实边界。
func (r partialGroupRemote) MoveMany(ctx context.Context, a mail.Account, p, folder string, uids []uint32, dest string) error {
	if err := r.Remote.MoveMany(ctx, a, p, folder, uids[:min(500, len(uids))], dest); err != nil {
		return err
	}
	return fmt.Errorf("后续批次连接中断")
}

// TestGroupDeleteReconcilesPartialBatchFailure 验证大组部分删除后立即核对服务器，组数不能停留在旧的 501 封。
func TestGroupDeleteReconcilesPartialBatchFailure(t *testing.T) {
	s := testSession(t)
	remote := s.remote.(*protocol.Memory)
	keys := []any{}
	for uid := uint32(100); uid < 601; uid++ {
		remote.Put("demo-mailfabrica-dev", "INBOX", uid, []byte("Subject: large group\r\n\r\nbody"), mail.Flags{})
		keys = append(keys, mail.Key{Account: "demo-mailfabrica-dev", Folder: "INBOX", UID: uid}.String())
	}
	executeTest(t, s, "mail.sync", Params{})
	s.remote = partialGroupRemote{s.remote}
	result := executeTest(t, s, "message.delete", Params{"keys": keys})
	var accepted struct {
		Task     string
		Revision uint64
	}
	if err := json.Unmarshal(result, &accepted); err != nil || accepted.Task == "" || accepted.Revision == 0 {
		t.Fatalf("missing task receipt: %s", result)
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	remaining := 0
	for _, row := range s.view.Rows {
		if row.Message.Subject == "large group" {
			remaining += row.Count
		}
	}
	if remaining != 1 {
		t.Fatalf("分组仍显示 %d 封，服务器实际只剩 1 封", remaining)
	}
	trash, err := s.Store.Index("demo-mailfabrica-dev", "Trash")
	if err != nil || len(trash.Messages) != 500 {
		t.Fatalf("已成功删除部分未同步: %v", err)
	}
	if len(s.status.Errors) == 0 {
		t.Fatal("partial failure swallowed")
	}
}

// arrivingGroupRemote 在旧分组被删除后模拟同主题新邮件到达，验证删除范围固定在确认时。
type arrivingGroupRemote struct {
	protocol.Remote
	memory *protocol.Memory
}

// MoveMany 只删除传入的 UID，然后添加一封不属于本次操作的新邮件。
func (r arrivingGroupRemote) MoveMany(ctx context.Context, a mail.Account, p, folder string, uids []uint32, dest string) error {
	if err := r.Remote.MoveMany(ctx, a, p, folder, uids, dest); err != nil {
		return err
	}
	r.memory.Put(a.ID, folder, 900, []byte("Subject: 下周产品评审安排\r\n\r\nnew arrival"), mail.Flags{})
	return nil
}

// TestGroupDeleteKeepsLaterArrival 验证删除整组不会连同稍后到达的同主题邮件一起删除。
func TestGroupDeleteKeepsLaterArrival(t *testing.T) {
	s := testSession(t)
	memory := s.remote.(*protocol.Memory)
	s.remote = arrivingGroupRemote{Remote: s.remote, memory: memory}
	executeTest(t, s, "message.delete", Params{"keys": []any{"demo-mailfabrica-dev/1/INBOX", "demo-mailfabrica-dev/8/INBOX"}})
	s.mu.Lock()
	defer s.mu.Unlock()
	idx, err := s.Store.Index("demo-mailfabrica-dev", "INBOX")
	if err != nil {
		t.Fatal(err)
	}
	found := false
	for _, m := range idx.Messages {
		if m.Key.UID == 900 {
			found = true
		}
		if m.Key.UID == 1 || m.Key.UID == 8 {
			t.Fatal("old group member remained")
		}
	}
	if !found {
		t.Fatal("new arrival was removed with old group")
	}
	if len(s.status.Errors) > 0 {
		t.Fatal(s.status.Errors)
	}
}
