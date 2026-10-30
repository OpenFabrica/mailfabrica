package desktop

import (
	"encoding/json"
	"fmt"
	"github.com/openfabrica/mailfabrica/internal/engine"
	"github.com/wailsapp/wails/v3/pkg/application"
	"github.com/wailsapp/wails/v3/pkg/events"
)

// GuardComposeWindow 在原生关闭操作中保护尚未保存的写信内容。
func GuardComposeWindow(app *application.App, session *engine.Session, window *application.WebviewWindow, id uint64) {
	window.RegisterHook(events.Common.WindowClosing, func(event *application.WindowEvent) {
		var state struct {
			Composes []engine.Compose `json:"composes"`
		}
		if json.Unmarshal(session.Snapshot(), &state) != nil {
			event.Cancel()
			return
		}
		var current *engine.Compose
		for _, c := range state.Composes {
			if c.ID == id {
				current = &c
				break
			}
		}
		if current == nil {
			return
		}
		execute := func(command string) {
			p, _ := json.Marshal(map[string]any{"id": id})
			if _, e := session.Execute(command, p); e != nil {
				app.Dialog.Error().SetMessage(e.Error()).Show()
			}
		}
		if current.Sending {
			event.Cancel()
			return
		}
		if !current.Dirty {
			execute("compose.discard")
			return
		}
		event.Cancel()
		dialog := app.Dialog.Question().SetTitle("未保存的邮件").SetMessage("保存草稿后关闭，还是丢弃这封邮件？")
		dialog.AddButton("保存草稿").OnClick(func() { execute("compose.saveDraft") })
		dialog.AddButton("丢弃").OnClick(func() { execute("compose.discard") })
		dialog.AddButton("取消")
		dialog.Show()
	})
}

// ComposeWindowName 返回控制通道使用的明确窗口标识。
func ComposeWindowName(id uint64) string { return fmt.Sprintf("compose-%d", id) }
