package desktop

import (
	"fmt"
	"log/slog"
	"runtime"
	"strconv"
	"sync/atomic"

	"github.com/openfabrica/mailfabrica/internal/engine"
	"github.com/wailsapp/wails/v3/pkg/application"
	"github.com/wailsapp/wails/v3/pkg/events"
)

// Tray 装配常驻托盘与主窗口生命周期，业务状态和偏好仍由引擎维护。
type Tray struct {
	app        *application.App
	session    *engine.Session
	window     *application.WebviewWindow
	tray       *application.SystemTray
	countItem  *application.MenuItem
	last       engine.DesktopState
	applied    bool
	icon       []byte
	forcedQuit atomic.Bool
	askingQuit atomic.Bool
	ready      atomic.Bool
}

// NewTray 注册关闭拦截与 Dock 重开；未启用时不创建系统托盘。
func NewTray(app *application.App, session *engine.Session, window *application.WebviewWindow) (*Tray, error) {
	icon, err := trayIcon(0, runtime.GOOS == "darwin")
	if err != nil {
		return nil, err
	}
	t := &Tray{app: app, session: session, window: window, icon: icon}
	window.RegisterHook(events.Common.WindowClosing, func(event *application.WindowEvent) {
		event.Cancel()
		if !session.DesktopState().CloseToTray {
			app.Quit()
			return
		}
		t.Sync()
		application.InvokeSync(func() {
			window.Hide()
			setDockVisible(false)
		})
	})
	if runtime.GOOS == "darwin" {
		app.Event.OnApplicationEvent(events.Mac.ApplicationShouldHandleReopen, func(*application.ApplicationEvent) { t.Show() })
	}
	app.Event.OnApplicationEvent(events.Common.ApplicationStarted, func(*application.ApplicationEvent) {
		t.ready.Store(true)
		t.Sync()
	})
	return t, nil
}

// Show 复用主窗口并恢复最小化状态，不把完整邮件窗口定位成托盘浮窗。
func (t *Tray) Show() {
	// App.Show 不自动切换线程；Cocoa 的 unhide 必须和窗口恢复一并运行在主线程。
	application.InvokeSync(func() {
		setDockVisible(true)
		t.app.Show()
		if t.window.IsMinimised() {
			t.window.UnMinimise()
		}
		t.window.Show()
		t.window.Focus()
	})
}

// Sync 只在偏好、语言或未读数改变时更新原生菜单，在 UI 线程串行操作托盘。
func (t *Tray) Sync() {
	if !t.ready.Load() {
		return
	}
	state := t.session.DesktopState()
	application.InvokeSync(func() {
		if t.applied && state.CloseToTray == t.last.CloseToTray && state.Unread == t.last.Unread && state.Language == t.last.Language {
			return
		}
		previous := t.last
		t.last = state
		t.applied = true
		if !state.CloseToTray {
			if t.tray != nil {
				if !t.window.IsVisible() {
					t.Show()
				}
				t.tray.Destroy()
				t.tray = nil
				t.countItem = nil
			}
			return
		}
		en := state.Language == "en"
		text := func(chinese, english string) string {
			if en {
				return english
			}
			return chinese
		}
		if t.tray == nil {
			t.tray = t.app.SystemTray.New()
			if runtime.GOOS == "darwin" {
				t.tray.SetTemplateIcon(t.icon)
			} else {
				t.tray.SetIcon(t.icon)
			}
			t.tray.OnClick(t.Show)
			t.tray.OnRightClick(t.tray.OpenMenu)
		}
		if t.countItem == nil || previous.Language != state.Language {
			menu := t.app.NewMenu()
			t.countItem = menu.Add("").SetEnabled(false)
			menu.Add(text("打开 MailFabrica", "Open MailFabrica")).OnClick(func(*application.Context) { t.Show() })
			menu.Add(text("收取邮件", "Sync mail")).OnClick(func(*application.Context) {
				if _, err := t.session.Execute("mail.sync", nil); err != nil {
					t.app.Dialog.Error().SetMessage(err.Error()).Show()
				}
			})
			menu.Add(text("设置…", "Settings…")).OnClick(func(*application.Context) {
				t.Show()
				if _, err := t.session.Execute("app.dialog", []byte(`{"name":"settings"}`)); err != nil {
					t.app.Dialog.Error().SetMessage(err.Error()).Show()
				}
			})
			menu.AddSeparator()
			menu.Add(text("退出 MailFabrica", "Quit MailFabrica")).OnClick(func(*application.Context) { t.app.Quit() })
			t.tray.SetMenu(menu)
		}
		summary := fmt.Sprintf(text("%d 封未读邮件", "%d unread messages"), state.Unread)
		t.countItem.SetLabel(summary)
		t.tray.SetTooltip("MailFabrica · " + summary)
		label := ""
		if state.Unread > 0 {
			label = strconv.Itoa(state.Unread)
		}
		t.tray.SetLabel(label)
		if runtime.GOOS != "darwin" {
			icon, err := trayIcon(state.Unread, false)
			if err != nil {
				slog.Warn("托盘图标更新失败", "error", err)
			} else {
				t.tray.SetIcon(icon)
			}
		}
	})
}

// ShouldQuit 让显式退出绕过常驻隐藏，同时保护正在发送或未保存的邮件。
func (t *Tray) ShouldQuit() bool {
	if t.forcedQuit.Load() {
		return true
	}
	state := t.session.DesktopState()
	if !state.Sending && state.UnsavedComposes == 0 {
		return true
	}
	if !t.askingQuit.CompareAndSwap(false, true) {
		return false
	}
	if state.Sending {
		dialog := t.app.Dialog.Info().SetTitle("邮件正在发送").SetMessage("请等待发送或保存草稿完成后再退出。")
		dialog.AddButton("确定").OnClick(func() { t.askingQuit.Store(false) })
		dialog.Show()
		return false
	}
	dialog := t.app.Dialog.Question().SetTitle("未保存的邮件").SetMessage("仍有未保存的写信，退出将丢失这些修改。")
	dialog.AddButton("取消").SetAsDefault().OnClick(func() { t.askingQuit.Store(false) })
	dialog.AddButton("丢弃并退出").OnClick(func() { t.forcedQuit.Store(true); t.app.Quit() })
	dialog.Show()
	return false
}

// State 返回固定控制接口需要的原生托盘和窗口状态，不暴露邮件内容。
func (t *Tray) State() any {
	if !t.ready.Load() {
		state := t.session.DesktopState()
		return map[string]any{"enabled": state.CloseToTray, "created": false, "unread": state.Unread, "label": "", "windowVisible": false, "windowMinimised": false}
	}
	var result any
	application.InvokeSync(func() {
		state := t.session.DesktopState()
		label := ""
		if t.tray != nil {
			label = t.tray.Label()
		}
		status := map[string]any{"enabled": state.CloseToTray, "created": t.tray != nil, "unread": state.Unread, "label": label, "windowVisible": t.window.IsVisible(), "windowMinimised": t.window.IsMinimised()}
		if runtime.GOOS == "darwin" {
			status["dockVisible"] = dockVisible()
		}
		result = status
	})
	return result
}
