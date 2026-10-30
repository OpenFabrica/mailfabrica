# Go + Wails 迁移记录

2026-10-30。迁移行为基准为 Rust 提交 `bfd5b8f`，自 0.1.6 起为默认技术栈。
默认构建、桌面、CLI 与 MCP 均使用 Go。旧工程及其构建工具已从当前源码树移除；历史实现可通过 Git 标签与提交查阅，兼容逻辑保留在 Go 模块中。

## 已实现

- Wails `v3.0.0`、React/TypeScript/Vite 与 Tiptap；领域与命令引擎不依赖桌面框架。
- 基准的 75 个命令和新增的联系人往来、自动补全及聚合邮件汇总命令，覆盖账户、同步、视图、邮件变更、阅读、写信、富文本、联系人、AI、发票、日历与更新。
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
- macOS Wails 应用启动、原生桥接与真实窗口截图；Developer ID 签名、公证、票据和 Gatekeeper 验证通过。
- 本机原有 4 个账户、815 封缓存邮件保留，替换前后原始 `.eml` 哈希一致；原生邮件阅读区可打开联系人往来时间线。
- 菜单栏常驻、窗口关闭隐藏与恢复、后台未读数字及零未读状态通过原生回归验收。

## 尚未验证的范围

EventKit 首次授权、旧 Rust 版到 Go 版的自动更新交接，以及 Windows/Linux 用户实机的图形桌面集成。旧版首次迁移采用手动安装，详见 [updates.md](updates.md)。
本机已有数据及应用替换不等于覆盖所有邮件服务商、系统版本和桌面环境。跨平台 CI 的测试与隔离演示不代替这些验收。
控制通道 DOM 截图明确不包含正文 iframe；完整正文视觉使用浏览器和 macOS 原生窗口截图验证。

UI 控制协议升级为 v2；旧 egui GPU 布局和拖选不伪造兼容，差异见 [control-protocol.md](control-protocol.md)。
