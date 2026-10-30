// MailFabrica 桌面入口，邮件业务统一经 Go 引擎执行。
package main

import (
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"github.com/openfabrica/mailfabrica/frontend"
	"github.com/openfabrica/mailfabrica/internal/control"
	"github.com/openfabrica/mailfabrica/internal/desktop"
	"github.com/openfabrica/mailfabrica/internal/engine"
	"github.com/openfabrica/mailfabrica/internal/update"
	"github.com/wailsapp/wails/v3/pkg/application"
	"io/fs"
	"log/slog"
	"net/http"
	"os"
	"os/signal"
	"strconv"
	"sync"
	"time"
)

// main 解析桌面参数，默认使用系统数据目录，演示使用独立临时目录。
func main() {
	if e := run(); e != nil {
		fmt.Fprintln(os.Stderr, e)
		os.Exit(1)
	}
}

// run 完成资源、控制通道、窗口与退出清理装配。
func run() error {
	if len(os.Args) == 3 && os.Args[1] == "--apply-update" {
		return update.RunHelper(os.Args[2])
	}

	demo := flag.Bool("demo", false, "使用隔离的演示邮箱")
	data := flag.String("data", "", "数据根目录")
	port := flag.Int("control", 0, "回环控制端口")
	web := flag.Int("web", 0, "浏览器验收端口（仅回环）")
	flag.Parse()
	if *port == 0 {
		if v := os.Getenv("MAILFABRICA_CONTROL_PORT"); v != "" {
			n, e := strconv.Atoi(v)
			if e != nil {
				return fmt.Errorf("控制端口无效")
			}
			*port = n
		}
	}
	root, e := engine.DataRoot(*data)
	if e != nil {
		return e
	}
	if *demo && *data == "" {
		root, e = os.MkdirTemp("", "mailfabrica-go-demo-*")
		if e != nil {
			return e
		}
		defer os.RemoveAll(root)
	}
	session, e := engine.New(root, *demo)
	if e != nil {
		return e
	}
	defer session.Close()
	executable, err := os.Executable()
	if err != nil {
		return err
	}
	session.ConfigureUpdateTarget(executable)
	ctx, cancel := signal.NotifyContext(context.Background(), os.Interrupt)
	defer cancel()
	assets, e := frontend.Assets()
	if e != nil {
		return e
	}
	if *web != 0 {
		return runWeb(ctx, session, assets, *web, *port)
	}
	service := &desktop.Service{Session: session}
	var tray *desktop.Tray
	app := application.New(application.Options{Name: "MailFabrica", Description: "MailFabrica 邮件客户端", ShouldQuit: func() bool { return tray == nil || tray.ShouldQuit() }, LogLevel: slog.LevelWarn, Services: []application.Service{application.NewService(service)}, Assets: application.AssetOptions{Handler: application.BundledAssetFileServer(assets)}, Mac: application.MacOptions{ApplicationShouldTerminateAfterLastWindowClosed: false}, Windows: application.WindowsOptions{DisableQuitOnLastWindowClosed: true}, Linux: application.LinuxOptions{DisableQuitOnLastWindowClosed: true}})
	service.App = app
	window := app.Window.NewWithOptions(application.WebviewWindowOptions{Name: "main", Title: "MailFabrica", Width: 1380, Height: 860, MinWidth: 760, MinHeight: 560, URL: "/", Mac: application.MacWindow{TitleBar: application.MacTitleBarDefault}})
	tray, e = desktop.NewTray(app, session, window)
	if e != nil {
		return e
	}
	menu := app.NewMenu()
	menu.AddRole(application.AppMenu)
	menu.AddRole(application.EditMenu)
	menu.AddRole(application.WindowMenu)
	app.Menu.Set(menu)
	var mu sync.Mutex
	composes := map[uint64]*application.WebviewWindow{}
	handle := func(method string, params json.RawMessage) (any, error) {
		switch method {
		case "app.quit":
			app.Quit()
			return nil, nil
		case "ui.focusWindow", "ui.showWindow":
			tray.Show()
			return nil, nil
		case "ui.closeWindow":
			window.Close()
			return nil, nil
		case "ui.tray":
			return tray.State(), nil
		default:
			return service.UI(method, params)
		}
	}
	if *port != 0 {
		listener, e := control.Serve(ctx, session, *port, handle)
		if e != nil {
			return e
		}
		defer listener.Close()
	}
	go func() {
		ticker := time.NewTicker(100 * time.Millisecond)
		defer ticker.Stop()
		for {
			select {
			case <-ctx.Done():
				return
			case <-ticker.C:
				tray.Sync()
				for _, r := range session.AcknowledgeRequests() {
					switch r.Kind {
					case "openCompose":
						mu.Lock()
						w := composes[r.ID]
						mu.Unlock()
						if w != nil {
							w.Focus()
						} else {
							w = app.Window.NewWithOptions(application.WebviewWindowOptions{Name: desktop.ComposeWindowName(r.ID), Title: "写邮件 · MailFabrica", Width: 820, Height: 760, MinWidth: 550, MinHeight: 500, URL: fmt.Sprintf("/?compose=%d", r.ID)})
							desktop.GuardComposeWindow(app, session, w, r.ID)
							mu.Lock()
							composes[r.ID] = w
							mu.Unlock()
						}
					case "closeCompose":
						mu.Lock()
						w := composes[r.ID]
						delete(composes, r.ID)
						mu.Unlock()
						if w != nil {
							w.Close()
						}
					case "dialog":
						app.Event.Emit("mailfabrica:dialog", r.Name)
					case "restartForUpdate":
						app.Quit()
					case "openInstaller":
						_ = app.Browser.OpenFile(r.Name)
					}
				}
			}
		}
	}()
	e = app.Run()
	if e == nil {
		if commitErr := session.CommitUpdate(); commitErr != nil {
			e = commitErr
		}
	}
	cancel()
	return e
}

// runWeb 为浏览器验收提供嵌入构建资源和真实 Go 会话。
func runWeb(ctx context.Context, s *engine.Session, assets fs.FS, port, controlPort int) error {
	if port < 1 || port > 65535 {
		return fmt.Errorf("Web 端口无效")
	}
	service := &desktop.Service{Session: s}
	mux := http.NewServeMux()
	mux.Handle("/api/", control.HTTPHandler(service))
	mux.Handle("/", http.FileServerFS(assets))
	server := &http.Server{Addr: fmt.Sprintf("127.0.0.1:%d", port), Handler: mux, ReadHeaderTimeout: 10 * time.Second, ReadTimeout: 30 * time.Second, WriteTimeout: 30 * time.Second}
	if controlPort != 0 {
		listener, e := control.Serve(ctx, s, controlPort, nil)
		if e != nil {
			return e
		}
		defer listener.Close()
	}
	go func() {
		<-ctx.Done()
		shutdown, cancel := context.WithTimeout(context.Background(), 3*time.Second)
		defer cancel()
		_ = server.Shutdown(shutdown)
	}()
	fmt.Printf("MailFabrica: http://127.0.0.1:%d\n", port)
	e := server.ListenAndServe()
	if e == http.ErrServerClosed {
		return nil
	}
	return e
}
