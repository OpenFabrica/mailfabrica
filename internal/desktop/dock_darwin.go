package desktop

/*
#cgo LDFLAGS: -framework AppKit
int mailfabrica_set_dock_visible(int visible);
int mailfabrica_dock_visible(void);
*/
import "C"

import "log/slog"

// setDockVisible 在 Wails UI 线程切换应用的 Dock 和应用切换器入口，保留菜单栏托盘。
func setDockVisible(visible bool) {
	value := C.int(0)
	if visible {
		value = 1
	}
	if C.mailfabrica_set_dock_visible(value) == 0 {
		slog.Warn("Dock 显示状态切换失败", "visible", visible)
	}
}

// dockVisible 在 Wails UI 线程读取系统实际应用策略，供固定控制接口验收。
func dockVisible() bool {
	return C.mailfabrica_dock_visible() != 0
}
