//go:build !darwin

package desktop

// setDockVisible 在没有 macOS Dock 的平台不改变现有窗口与任务栏行为。
func setDockVisible(bool) {}

// dockVisible 仅 macOS 状态响应使用，其他平台没有对应入口。
func dockVisible() bool { return false }
