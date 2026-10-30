package engine

import (
	"context"
	"fmt"
	"github.com/openfabrica/mailfabrica/internal/update"
	"os"
)

// Version 由正式构建通过 ldflags 注入，不依赖 Cargo。
var Version = "0.1.7-dev"

// UpdateState 为所有交付入口提供同一更新状态。
type UpdateState struct {
	Phase      string             `json:"phase"`
	Release    *update.Release    `json:"release"`
	Downloaded int64              `json:"downloaded"`
	Installer  *string            `json:"installer"`
	Error      *string            `json:"error"`
	Cancel     context.CancelFunc `json:"-"`
}

// updateCommand 执行检查、校验下载及显式打开安装器。
func (s *Session) updateCommand(id string, p Params) (any, error) {
	if len(p) > 0 {
		return nil, fmt.Errorf("更新命令不接受自定义来源或路径")
	}
	if s.update.Phase == "" {
		s.update.Phase = "idle"
	}
	if id == "update.status" {
		return s.update, nil
	}
	if id == "update.cancel" {
		if s.update.Cancel == nil {
			return nil, fmt.Errorf("没有可取消的更新任务")
		}
		s.update.Cancel()
		if s.updateHandoff != nil {
			if e := s.updateHandoff.Cancel(); e != nil {
				return nil, e
			}
			s.updateHandoff = nil
		}
		s.update.Phase = "cancelled"
		return s.update, nil
	}
	if s.busy["update"] {
		return nil, fmt.Errorf("更新任务正在运行")
	}
	if id == "update.open" {
		if s.update.Installer == nil || s.update.Release == nil {
			return nil, fmt.Errorf("尚无已校验安装器")
		}
		path := *s.update.Installer
		r := *s.update.Release
		return nil, s.launch("update", func(ctx context.Context) (any, error) {
			b, e := os.ReadFile(path)
			if e != nil {
				return nil, e
			}
			if e = update.VerifyBytes(r, b); e != nil {
				return nil, e
			}
			return nil, update.VerifyPublisher(ctx, path)
		}, func(any) error {
			return s.prepareInstallation(path)
		})
	}
	ctx, cancel := context.WithCancel(s.ctx)
	s.update.Cancel = cancel
	s.update.Error = nil
	if id == "update.check" {
		s.update.Phase = "checking"
		e := s.launch("update", func(context.Context) (any, error) {
			r, err := update.Check(ctx, Version)
			if err != nil {
				s.updateFailure(err)
			}
			return r, err
		}, func(v any) error {
			s.update.Cancel = nil
			cancel()
			r, _ := v.(*update.Release)
			s.update.Release = r
			s.update.Phase = "current"
			if r != nil {
				s.update.Phase = "available"
			}
			return nil
		})
		return s.update, e
	}
	if s.update.Release == nil {
		cancel()
		s.update.Cancel = nil
		return nil, fmt.Errorf("请先检查可用更新")
	}
	r := *s.update.Release
	root := s.Store.Root
	s.update.Phase = "downloading"
	e := s.launch("update", func(context.Context) (any, error) {
		path, err := update.Download(ctx, r, root, func(n int64) { s.mu.Lock(); s.update.Downloaded = n; s.touch(); s.mu.Unlock() })
		if err != nil {
			s.updateFailure(err)
		}
		return path, err
	}, func(v any) error {
		s.update.Cancel = nil
		cancel()
		path, ok := v.(string)
		if !ok {
			return fmt.Errorf("安装器路径无效")
		}
		s.update.Installer = &path
		s.update.Phase = "ready"
		return s.prepareInstallation(path)
	})
	return s.update, e
}

// updateFailure 在后台错误时恢复可重试状态。
func (s *Session) updateFailure(err error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.update.Phase = "failed"
	if s.update.Cancel != nil {
		s.update.Cancel()
		s.update.Cancel = nil
	}
	msg := err.Error()
	s.update.Error = &msg
}

// ConfigureUpdateTarget 仅允许宿主从自身可执行文件配置替换目标。
func (s *Session) ConfigureUpdateTarget(executable string) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.updateExecutable = executable
	s.updateTarget = update.ApplicationTarget(executable)
}

// prepareInstallation 在支持的应用包中准备自动替换，开发二进制保持手动打开安装器。
func (s *Session) prepareInstallation(path string) error {
	if s.updateTarget == "" {
		s.requests = append(s.requests, UIRequest{Kind: "openInstaller", Name: path})
		return nil
	}
	ctx, cancel := context.WithCancel(s.ctx)
	s.update.Cancel = cancel
	s.update.Phase = "installing"
	target, executable := s.updateTarget, s.updateExecutable
	return s.launch("update", func(context.Context) (any, error) {
		h, e := update.Prepare(ctx, path, target)
		if e == nil {
			e = ctx.Err()
		}
		if e == nil {
			e = h.Start(executable)
		}
		if e == nil {
			e = ctx.Err()
		}
		if e != nil {
			if h != nil {
				_ = h.Cancel()
			}
			s.updateFailure(e)
		}
		return h, e
	}, func(v any) error {
		h, ok := v.(*update.Handoff)
		if !ok {
			return fmt.Errorf("安装交接无效")
		}
		if s.update.Phase == "cancelled" {
			return h.Cancel()
		}
		s.updateHandoff = h
		s.requests = append(s.requests, UIRequest{Kind: "restartForUpdate"})
		return nil
	})
}

// CommitUpdate 只在正常退出路径提交已准备的更新。
func (s *Session) CommitUpdate() error {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.updateHandoff == nil || s.update.Phase == "cancelled" {
		return nil
	}
	return s.updateHandoff.Commit()
}
