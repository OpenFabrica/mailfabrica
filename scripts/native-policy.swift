import AppKit
import Foundation

// 从独立进程读取 AppKit 的应用策略，避免仅以应用内部记录判定 Dock 已隐藏。
guard CommandLine.arguments.count == 2,
      let pid = pid_t(CommandLine.arguments[1]),
      let app = NSRunningApplication(processIdentifier: pid) else {
    fputs("无法读取验收进程的应用策略\n", stderr)
    exit(1)
}
print(app.activationPolicy.rawValue)
