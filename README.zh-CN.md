# MailFabrica

[English](README.md) | 简体中文

Go + Wails v3 桌面邮件客户端。前端使用 React、TypeScript 和 Vite；界面借鉴 Magpie 的配色与控件，并按 Foxmail 的紧凑三栏方式组织邮箱、邮件列表和阅读窗格。

## 功能

- 多账户 IMAP/SMTP、服务商检测与企业邮箱预设、TLS/STARTTLS、中文文件夹、UID 增量同步与离线阅读。
- 统一收件箱、未读、红旗、发票视图，搜索、主题归类、多选、标记、移动和删除。
- 隔离 HTML 阅读、远程图片开关、CID 内嵌图片、附件保存和纯文本模式。
- 独立写信窗口、回复/全部回复/转发、富文本、附件、签名、草稿与联系人。
- Anthropic / OpenAI 兼容 AI 助手，建议先预览再采用；密钥使用原有系统凭据服务。
- 发票附件与 CSV 导出、邮件日历与农历、macOS EventKit 系统日历。
- 中英界面、明暗主题、紧凑列表和持久化偏好。
- 共用 Go 命令引擎的桌面、CLI、MCP 和回环 JSON 行控制通道。
- 固定 GitHub 来源、SHA-256 与 macOS 发布者验证；应用包支持正常退出后的更新交接。

## 开发与运行

需要 Go（版本见 `go.mod`）、Node.js 22 和平台原生编译环境。macOS 需要 Xcode Command Line Tools；Linux 默认使用 GTK4/WebKitGTK 6，也可用 `GOFLAGS=-tags=gtk3` 配合 GTK3/WebKitGTK 4.1。

```sh
make deps
make demo                 # 隔离演示邮箱，不读取真实账户和凭据
make check                # 前端构建/格式、Go vet、测试、race 检查
make package              # 生成本机应用；macOS 为 dist/MailFabrica.app

# 正常数据目录：只在你明确运行正常模式时打开
./dist/mailfabrica
./dist/mailfabrica --data /path/to/mail

# CLI
./dist/mailfabrica-cli --demo list
./dist/mailfabrica-cli --demo commands --json
./dist/mailfabrica-cli --demo mcp
```

Go 构建与运行不依赖 Rust、Cargo 或 Rust 子进程。版本由 `VERSION` 管理。

前端热更新开发可在两个终端分别运行：

```sh
./dist/mailfabrica --demo --web 7982
npm run dev --prefix frontend -- --port 5184
```

浏览器访问 `http://127.0.0.1:5184`；它通过本地开发代理连接同一 Go 引擎。原生文件对话框与系统剪贴板需在 Wails 桌面程序中验收。

## 数据兼容

沿用原账户 JSON、文件夹索引、`.eml` 原文、联系人、消息键及 FNV-1a 文件夹目录。旧 `ui.json` 的主题、语言、宽度与日历来源过滤会读取到新偏好；原文件不被覆盖。macOS 默认目录仍为 `~/Library/Application Support/MailFabrica`，可用 `--data` 或 `MAILFABRICA_HOME` 覆盖。凭据服务名保持 `MailFabrica`。

迁移测试使用独立演示目录。请先使用数据副本验证自己的邮箱；QQ、网易等通常需要开启 IMAP 并使用授权码。AI 只有在用户点击相应操作时才发送所需邮件内容。

## 文档与验证边界

- [架构与领域规则](docs/architecture.md)
- [迁移记录与命令目录](docs/go-wails-migration.md)
- [本地构建](docs/local-build.md)、[CLI](docs/cli.md)、[MCP](docs/mcp.md)、[控制协议](docs/control-protocol.md)
- [邮件阅读](docs/mail-reading.md)、[更新](docs/updates.md)、[跨平台构建](docs/portable-release.md)

本地 `.app` 使用临时 ad-hoc 签名，便于开发测试；它不是已公证的正式发行版。Windows/Linux 的构建配置和本机 macOS 的验证结果分别报告，不将交叉编译当作目标系统运行验收。

## 许可

MIT OR Apache-2.0。Magpie 作为界面研究参考，未引入其网关、插件或遥测业务。
