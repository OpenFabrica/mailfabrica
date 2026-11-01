package engine

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/openfabrica/mailfabrica/internal/mail"
	"github.com/openfabrica/mailfabrica/internal/protocol"
)

// TestAccountMarkAllRead 验证按账户一键已读覆盖全部文件夹，且不依赖当前列表范围。
func TestAccountMarkAllRead(t *testing.T) {
	s := testSession(t)
	executeTest(t, s, "view.scope", Params{"account": "demo-mailfabrica-dev", "folder": "Archive"})
	s.mu.Lock()
	before := s.counts.Unread
	s.mu.Unlock()
	if before == 0 {
		t.Fatal("demo must start with unread mail")
	}
	if _, err := s.Execute("account.markAllRead", marshalParams(Params{"id": "missing"})); err == nil {
		t.Fatal("unknown account accepted")
	}
	executeTest(t, s, "account.markAllRead", Params{"id": "demo-mailfabrica-dev"})
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.counts.Unread != 0 {
		t.Fatalf("unread %d", s.counts.Unread)
	}
	idx, err := s.Store.Index("demo-mailfabrica-dev", "INBOX")
	if err != nil {
		t.Fatal(err)
	}
	for _, m := range idx.Messages {
		if !m.Flags.Seen {
			t.Fatalf("%s still unread", m.Key)
		}
	}
}

// TestAIJunkScanAndClean 用本地 AI 接口验证分批只发送元数据、校验编号、部分失败保留结果，并在确认后移到垃圾邮件。
func TestAIJunkScanAndClean(t *testing.T) {
	s := testSession(t)
	prompts := make(chan string, 2)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		var payload struct {
			Messages []struct{ Role, Content string }
		}
		if err := json.NewDecoder(r.Body).Decode(&payload); err != nil {
			t.Error(err)
		}
		prompt := ""
		for _, m := range payload.Messages {
			if m.Role == "user" {
				prompt = m.Content
				prompts <- prompt
			}
		}
		// 第二批只有 1 封，模拟这一批 AI 调用失败
		if !strings.Contains(prompt, "[2]") {
			w.WriteHeader(http.StatusInternalServerError)
			return
		}
		// 代码块包裹、越界与重复编号都应被容忍或忽略
		answer, _ := json.Marshal("```json\n[{\"id\":1,\"reason\":\"营销广告\"},{\"id\":99,\"reason\":\"越界\"},{\"id\":1,\"reason\":\"重复\"}]\n```")
		_, _ = fmt.Fprintf(w, `{"choices":[{"message":{"content":%s}}]}`, answer)
	}))
	defer server.Close()
	executeTest(t, s, "ai.config", Params{"provider": "openai", "baseUrl": server.URL, "model": "test-only"})
	executeTest(t, s, "ai.scanJunk", Params{"account": "demo-mailfabrica-dev"})
	first, second := <-prompts, <-prompts
	if strings.Contains(first, "[2]") {
		first, second = second, first
	}
	// 11 封分为 10 + 1 两批，编号只在批内有效
	if !strings.Contains(second, "[10]") || strings.Contains(second, "[11]") || !strings.Contains(first, "[1]") || strings.Contains(first, "[2]") || strings.Contains(first+second, "下一阶段规划") {
		t.Fatal("prompt must list inbox metadata in batches")
	}
	s.mu.Lock()
	scan := s.aiJunk
	if scan == nil || scan.Busy || scan.Error == nil || !strings.HasPrefix(*scan.Error, "1 封邮件未能识别") || scan.Scanned != 11 || scan.Done != 11 || scan.Current != "" || len(scan.Candidates) != 1 {
		s.mu.Unlock()
		t.Fatalf("scan %+v", scan)
	}
	// 最新邮件编号为 1，对应演示收件箱 UID 1
	key := scan.Candidates[0].Key
	s.mu.Unlock()
	if key != "demo-mailfabrica-dev/1/INBOX" || scan.Candidates[0].Reason != "营销广告" {
		t.Fatalf("candidate %+v", scan.Candidates[0])
	}
	if _, err := s.Execute("ai.cleanJunk", marshalParams(Params{"keys": []any{"demo-mailfabrica-dev/2/INBOX"}})); err == nil {
		t.Fatal("non-candidate moved")
	}
	remote := &flagRemote{Remote: s.remote, gate: make(chan struct{})}
	close(remote.gate)
	s.mu.Lock()
	s.remote = remote
	s.mu.Unlock()
	executeTest(t, s, "ai.cleanJunk", Params{})
	s.mu.Lock()
	defer s.mu.Unlock()
	if remote.batches != 1 || remote.singles != 0 {
		t.Fatalf("move batches %d singles %d", remote.batches, remote.singles)
	}
	if len(s.aiJunk.Candidates) != 0 {
		t.Fatal("moved candidate kept")
	}
	if _, err := s.findMeta(mail.Key{Account: "demo-mailfabrica-dev", Folder: "INBOX", UID: 1}); err == nil {
		t.Fatal("message still in inbox")
	}
	junk, err := s.Store.Index("demo-mailfabrica-dev", "Junk")
	if err != nil {
		t.Fatal(err)
	}
	if len(junk.Messages) != 1 {
		t.Fatalf("junk %d", len(junk.Messages))
	}
}

// flagRemote 统计批量与逐封调用，可阻塞或注入失败以观察本地即时状态。
type flagRemote struct {
	protocol.Remote
	mu       sync.Mutex
	batches  int
	singles  int
	gate     chan struct{}
	failWith error
}

// SetFlags 记录一次批量调用，等待放行后返回注入错误或委托内存远端。
func (r *flagRemote) SetFlags(ctx context.Context, a mail.Account, p, folder string, uids []uint32, op string, value bool) error {
	r.mu.Lock()
	r.batches++
	r.mu.Unlock()
	<-r.gate
	if r.failWith != nil {
		return r.failWith
	}
	return r.Remote.SetFlags(ctx, a, p, folder, uids, op, value)
}

// MoveMany 记录一次批量移动并委托内存远端。
func (r *flagRemote) MoveMany(ctx context.Context, a mail.Account, p, folder string, uids []uint32, dest string) error {
	r.mu.Lock()
	r.batches++
	r.mu.Unlock()
	return r.Remote.MoveMany(ctx, a, p, folder, uids, dest)
}

// Change 记录逐封调用；批量标记和移动不应走到这里。
func (r *flagRemote) Change(ctx context.Context, a mail.Account, p string, k mail.Key, op string, value any) error {
	r.mu.Lock()
	r.singles++
	r.mu.Unlock()
	return r.Remote.Change(ctx, a, p, k, op, value)
}

// TestMarkAllReadBatchedAndImmediate 验证一键已读立即更新本地、每文件夹只登录一次，失败时回滚且错误不重复堆叠。
func TestMarkAllReadBatchedAndImmediate(t *testing.T) {
	for _, fail := range []bool{false, true} {
		s := testSession(t)
		remote := &flagRemote{Remote: s.remote, gate: make(chan struct{})}
		if fail {
			remote.failWith = errors.New("IMAP 登录失败: Too many login attempts")
		}
		s.mu.Lock()
		s.remote = remote
		before := s.counts.Unread
		s.mu.Unlock()
		if _, err := s.Execute("account.markAllRead", marshalParams(Params{"id": "demo-mailfabrica-dev"})); err != nil {
			t.Fatal(err)
		}
		// 服务器尚未返回时，本地已显示为已读，界面不必等待网络
		s.mu.Lock()
		immediate := s.counts.Unread
		s.mu.Unlock()
		close(remote.gate)
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		if err := s.Wait(ctx); err != nil {
			t.Fatal(err)
		}
		cancel()
		if immediate != 0 {
			t.Fatalf("local unread before server reply %d", immediate)
		}
		s.mu.Lock()
		after, errs := s.counts.Unread, s.status.Errors
		s.mu.Unlock()
		if remote.batches != 1 || remote.singles != 0 {
			t.Fatalf("batches %d singles %d", remote.batches, remote.singles)
		}
		if !fail {
			if after != 0 {
				t.Fatalf("unread %d", after)
			}
			continue
		}
		if after != before {
			t.Fatalf("failed change not rolled back: %d != %d", after, before)
		}
		last := errs[len(errs)-1][1]
		if !strings.HasPrefix(last, fmt.Sprintf("%d 封邮件操作失败", before)) || strings.Count(last, "Too many") != 1 {
			t.Fatalf("error %q", last)
		}
	}
}
