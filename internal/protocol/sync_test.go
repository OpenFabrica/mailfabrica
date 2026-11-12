package protocol

import (
	"context"
	"errors"
	"github.com/openfabrica/mailfabrica/internal/mail"
	"testing"
	"time"
)

// TestNetworkStreamingOrder 验证真正 TCP IMAP 同步优先发布收件箱头部，再发送正文补充。
func TestNetworkStreamingOrder(t *testing.T) {
	a, r := testIMAP(t)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if err := r.Create(ctx, a, "password", "A-other"); err != nil {
		t.Fatal(err)
	}
	headers := map[string]bool{}
	first := ""
	bodies := 0
	_, err := r.SyncStream(ctx, a, "password", SyncOptions{Limit: 500, Prefetch: 1}, func(u SyncUpdate) error {
		if u.BodyOnly {
			if !headers[u.Folder.Path] {
				t.Error("body before headers")
			}
			bodies++
			return nil
		}
		if first == "" {
			first = u.Folder.Path
		}
		if len(u.Raw) != 0 {
			t.Error("headers waited for body")
		}
		headers[u.Folder.Path] = true
		return nil
	})
	if err != nil || first != "INBOX" || bodies != 1 || !headers["A-other"] {
		t.Fatalf("first=%s bodies=%d error=%v", first, bodies, err)
	}
}

// TestNetworkQuickSyncAndCallbackFailure 验证快速同步保留范围，发布失败停止后续正文下载。
func TestNetworkQuickSyncAndCallbackFailure(t *testing.T) {
	a, r := testIMAP(t)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if err := r.Create(ctx, a, "password", "Archive"); err != nil {
		t.Fatal(err)
	}
	folders := []mail.Folder{{Account: a.ID, Path: "INBOX", Role: "inbox", Selectable: true}, {Account: a.ID, Path: "Archive", Role: "archive", Selectable: true}}
	seen := map[string]bool{}
	_, err := r.SyncStream(ctx, a, "password", SyncOptions{Limit: 500, InboxOnly: true, Folders: folders}, func(u SyncUpdate) error { seen[u.Folder.Path] = true; return nil })
	if err != nil || len(seen) != 1 || !seen["INBOX"] {
		t.Fatalf("quick=%v %v", seen, err)
	}
	sentinel := errors.New("publish failed")
	_, err = r.SyncStream(ctx, a, "password", SyncOptions{Limit: 500, Prefetch: 50}, func(u SyncUpdate) error {
		if u.BodyOnly {
			t.Error("body downloaded after failed publish")
		}
		return sentinel
	})
	if !errors.Is(err, sentinel) {
		t.Fatalf("callback failure lost: %v", err)
	}
}

// TestScopedSyncDiscoversUncachedTarget 验证只核对变更目录时，也能发现尚未缓存的目标目录。
func TestScopedSyncDiscoversUncachedTarget(t *testing.T) {
	a, remote := testIMAP(t)
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	if err := remote.Create(ctx, a, "password", "Archive"); err != nil {
		t.Fatal(err)
	}
	seen := map[string]bool{}
	_, err := remote.SyncStream(ctx, a, "password", SyncOptions{Limit: 500, Folders: []mail.Folder{{Account: a.ID, Path: "INBOX", Role: "inbox", Selectable: true}}, FolderPaths: []string{"INBOX", "Archive"}}, func(u SyncUpdate) error { seen[u.Folder.Path] = true; return nil })
	if err != nil || !seen["Archive"] || !seen["INBOX"] || len(seen) != 2 {
		t.Fatalf("局部核对目录错误: %v %v", seen, err)
	}
}
