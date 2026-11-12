package protocol

import (
	"context"
	"errors"
	"github.com/emersion/go-imap/client"
	"github.com/openfabrica/mailfabrica/internal/mail"
	"time"
)

// ErrIdleUnsupported 表示服务器未声明 IDLE，应继续使用定时收件。
var ErrIdleUnsupported = errors.New("服务器不支持 IMAP IDLE")

// InboxWatcher 使用独立只读连接监听收件箱变化，不占用账户的邮件操作队列。
type InboxWatcher interface {
	WatchInbox(context.Context, mail.Account, string, string, func()) error
}

// WatchInbox 在通知后交给引擎增量同步；断开和取消均关闭连接，IDLE 每 25 分钟续期。
func (Network) WatchInbox(ctx context.Context, a mail.Account, password, folder string, notify func()) error {
	c, cleanup, err := dialIMAP(ctx, a, password)
	if err != nil {
		return err
	}
	defer cleanup()
	ok, err := c.Support("IDLE")
	if err != nil {
		return err
	}
	if !ok {
		return ErrIdleUnsupported
	}
	updates := make(chan client.Update, 64)
	c.Updates = updates
	if _, err = c.Select(folder, true); err != nil {
		return err
	}
	// IDLE 是长连接，普通命令的 45 秒超时不能用于等待通知；取消由 dialIMAP 的上下文关闭连接。
	c.Timeout = 0
	done := make(chan error, 1)
	go func() {
		done <- c.Idle(ctx.Done(), &client.IdleOptions{LogoutTimeout: 25 * time.Minute, PollInterval: -1})
	}()
	notify()
	for {
		select {
		case <-ctx.Done():
			cleanup()
			// 保持消费更新，避免协议读取线程因缓冲区满而无法完成退出。
			for {
				select {
				case <-updates:
				case <-done:
					return ctx.Err()
				}
			}
		case err := <-done:
			return err
		case update := <-updates:
			switch update.(type) {
			case *client.MailboxUpdate, *client.ExpungeUpdate:
				notify()
			}
		}
	}
}
