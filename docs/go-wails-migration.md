# Go + Wails 迁移记录

2026-10-30。当前工作区 `feat/go-wails`，行为基准为 Rust 提交 `bfd5b8f`。
默认构建、桌面、CLI 与 MCP 已切换到 Go；运行不依赖 Rust 子进程或 FFI。旧 Rust 源码暂留作兼容性比对，尚未删除。

## 已实现

- Wails `v3.0.0`、React/TypeScript/Vite 与 Tiptap；领域与命令引擎不依赖桌面框架。
- 基准的 75 个命令，覆盖账户、同步、视图、邮件变更、阅读、写信、富文本、联系人、AI、发票、日历与更新。
- 保留 JSON/EML、消息键、FNV-1a 目录与系统凭据服务名；读取旧 UI 偏好，保留原文件。
- 每账户串行网络任务、UID 增量同步、UIDVALIDITY 缓存失效；IMAP/SMTP/MIME 由 Go 实现。
- 桌面多写信窗口与关闭保护；CLI/MCP/控制通道调用同一业务命令。
- Go 默认构建、质量检查、本机应用打包与跨平台 CI 配置。

实际模块、依赖、领域规则与流程见 [architecture.md](architecture.md)。命令的参数和 gate 见
`internal/engine/commands.json`；可用 `./dist/mailfabrica-cli --demo commands --json` 发现。

## UI 确认

最初选定 Magpie 原生感，随后用户要求按 Foxmail 提高密度并去掉内容区外层卡片。
最终采用三栏直接铺满客户区，栏间细线分隔；默认 48px 双行邮件列表、日期分组、46px 顶部工具栏与约 42px 快速回复。

Magpie 参考快照为 `8f0d21a009d0f0cb6d73e30898e0ed60453e1e4a`；仅研究界面与桌面集成，未复制其网关、插件或遥测业务。

## 本机验证

- 原 Rust 基准 `cargo xtask ci` 通过，作为迁移前证据。
- Go `vet`、单元测试、协议集成和 race 检查；前端 TypeScript、Prettier 与生产构建。
- 真实 TCP 假 IMAP/SMTP 服务器：连接、登录、同步、增量、标记、中文文件夹、发送与 QUIT 断线语义。
- 隔离演示宿主的 HTTP 闭环：阅读 → 中文富文本和附件 → 发送 → 已发送副本 → 发票文件 → 农历日历。
- 浏览器实际编辑/加粗/发送、文件夹创建、深浅色和中英切换，以及无卡片布局截图。
- macOS Wails 应用启动、原生桥接与 DOM 截图；生成本机 ad-hoc 签名 `.app`。

## 尚未验证的范围

真实邮箱和旧用户数据副本迁移、目标签名下的钥匙串访问、EventKit 授权、正式签名更新安装，以及 Windows/Linux 实机。
不把模拟服务器或交叉编译等同于这些验收。
原生系统截图管道不可用；控制通道 DOM 截图明确不包含正文 iframe，完整正文视觉由浏览器截图验证。

UI 控制协议升级为 v2；旧 egui GPU 布局和拖选不伪造兼容，差异见 [control-protocol.md](control-protocol.md)。
当前更改未提交、未推送、未发布。
