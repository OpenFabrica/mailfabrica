package protocol

import (
	"context"
	"github.com/openfabrica/mailfabrica/internal/mail"
)

// SyncOptions 指定快速收件或完整同步，Folders 为已发现的文件夹，Known 为只读索引快照。
type SyncOptions struct {
	Limit, Prefetch uint32
	InboxOnly       bool
	FolderPaths     []string
	Folders         []mail.Folder
	Known           map[string]KnownFolder
}

// SyncUpdate 的各批头部共享完整 UID 集合，用于合并增量及删除核对；BodyOnly 只补充原文，不覆盖当前标记或删除其它邮件。
type SyncUpdate struct {
	FolderResult
	BodyOnly bool
}

// StreamingRemote 在网络操作之间发布进展；回调失败立即停止，已发布的文件夹保留。
type StreamingRemote interface {
	SyncStream(context.Context, mail.Account, string, SyncOptions, func(SyncUpdate) error) ([]mail.Folder, error)
}
