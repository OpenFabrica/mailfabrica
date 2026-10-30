//go:build !darwin

package update

import (
	"context"
	"fmt"
)

// Handoff 在 portable 平台不启用应用包替换。
type Handoff struct{}

// ApplicationTarget 表明 portable 平台使用手动安装。
func ApplicationTarget(string) string { return "" }

// Prepare 在不支持的系统明确返回手动安装要求。
func Prepare(context.Context, string, string) (*Handoff, error) {
	return nil, fmt.Errorf("此平台请手动安装更新")
}

// Start 在 portable 平台不启动替换进程。
func (*Handoff) Start(string) error { return fmt.Errorf("此平台请手动安装更新") }

// Commit 在 portable 平台没有自动交接。
func (*Handoff) Commit() error { return nil }

// Cancel 在 portable 平台没有自动交接。
func (*Handoff) Cancel() error { return nil }

// RunHelper 拒绝非 macOS 的应用包更新交接。
func RunHelper(string) error { return fmt.Errorf("此平台不支持应用包替换") }

// AcknowledgeStartup 在 portable 平台不清理任何目录。
func AcknowledgeStartup(string) error { return nil }
