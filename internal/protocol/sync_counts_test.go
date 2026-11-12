package protocol

import (
	"context"
	"errors"
	"fmt"
	"github.com/openfabrica/mailfabrica/internal/mail"
	"strings"
	"testing"
	"time"
)

// TestSyncPublishesHeaderBatches 验证未读数可以随首批邮件头更新，不等待整个文件夹 FETCH 完成。
func TestSyncPublishesHeaderBatches(t *testing.T) {
	a, remote := testIMAP(t)
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	c, closeConn, err := dialIMAP(ctx, a, "password")
	if err != nil {
		t.Fatal(err)
	}
	for i := 0; i < 205; i++ {
		raw := fmt.Sprintf("From: test@example.com\r\nSubject: incremental %d\r\n\r\nbody", i)
		if err = c.Append("INBOX", nil, time.Now(), strings.NewReader(raw)); err != nil {
			closeConn()
			t.Fatal(err)
		}
	}
	closeConn()
	stop := errors.New("first batch published")
	first := 0
	cached := KnownFolder{Messages: map[uint32]mail.Message{}}
	_, err = remote.SyncStream(ctx, a, "password", SyncOptions{Limit: 500}, func(u SyncUpdate) error {
		if len(u.Messages) == 0 {
			return nil
		}
		first = len(u.Messages)
		cached.UIDValidity = u.Folder.UIDValidity
		for _, message := range u.Messages {
			cached.Messages[message.Key.UID] = message
		}
		if first > 100 {
			t.Errorf("首批发布了 %d 封，角标仍在等待整文件夹；应最多 100 封", first)
		}
		return stop
	})
	if !errors.Is(err, stop) || first == 0 {
		t.Fatalf("未发布首批邮件头: count=%d err=%v", first, err)
	}
	total := map[uint32]bool{}
	for uid := range cached.Messages {
		total[uid] = true
	}
	results, err := remote.Sync(ctx, a, "password", 500, 0, map[string]KnownFolder{"INBOX": cached})
	if err != nil {
		t.Fatal(err)
	}
	if len(results) != 1 {
		t.Fatalf("分批结果被错误当作多个目录: %d", len(results))
	}
	for _, message := range results[0].Messages {
		total[message.Key.UID] = true
	}
	if len(total) != 206 {
		t.Fatalf("断线后跳过了未完成的旧批次: %d", len(total))
	}

}
