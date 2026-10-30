# 跨平台构建与发行边界

默认工具链为 Go + Node/Vite。`.github/workflows/portable.yml` 配置 macOS、Windows amd64/arm64 和 Linux amd64/arm64 原生构建、检查、隔离演示验证与产物上传；该工作流不自动发布 GitHub Release。

Linux CI 使用 `GOFLAGS=-tags=gtk3`，安装 GTK3 和 WebKitGTK 4.1；默认 GTK4 构建需要对应原生库。Windows 桌面运行需要 WebView2。

macOS 使用 Developer ID 签名和 Apple 公证，命令见 [local-build.md](local-build.md)。应用与 DMG 分别装订票据并检查 Gatekeeper。
Windows 与 Linux 按用户要求始终生成未签名 portable 包；`scripts/portable.py` 支持两种架构，Apple 参数不能用于这两个平台。

本机执行 `make package` 仅生成本机包。交叉编译只证明目标代码能够编译，不能代替窗口、文件对话框、字体、邮件正文和系统集成的目标系统验收。

正式发行前应分别取得各平台构建和真实运行证据，完成 macOS Developer ID 签名、公证、DMG 与更新验证，再按明确授权上传 Release。当前迁移的本机验证结果见 `docs/go-wails-migration.md`。
