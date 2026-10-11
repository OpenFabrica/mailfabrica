# MailFabrica

[English](README.md) | 简体中文

Rust 原生的桌面邮件客户端。界面和用法借鉴 Foxmail，工程做法沿用 GridFabrica（分层 crate、egui 界面、一切皆命令、控制通道、MCP）。

## 截图

阅读窗格按原格式显示 HTML 邮件，效果接近浏览器；同主题的邮件在列表里合成一行：

![主界面：HTML 邮件和按主题归类的列表](docs/screenshots/main.png)

点开归类的那一行，列出这个主题的全部邮件：

![主题组：同一主题的所有邮件](docs/screenshots/group.png)

回复 HTML 邮件时原文按原格式引用，发出去和 Gmail 的引用格式一样：

![回复：原文按原格式引用](docs/screenshots/reply.png)

截图来自演示模式（`--demo`），里面的账户和邮件都是虚构的。

## 功能
- **三栏界面**：账户/文件夹树（未读数）· 按"今天/昨天/本周/本月/更早"分组的邮件列表 · 阅读窗格 + 快速回复。
- **工具栏**：收取、写邮件、回复、全部回复、转发、删除、标记（已读/未读/红旗）、移动到、垃圾邮件、搜索。
- **统一视图**：所有收件箱、未读邮件、红旗邮件（跨账户）。
- **写信**：多窗口，收件人自动补全（地址簿），附件，存草稿 / 从草稿继续编辑，签名，回复引用、转发带原附件。
- **账户**：输入邮箱自动填服务器（QQ/Foxmail、163、126、yeah、新浪、阿里、139、Gmail、Outlook、iCloud、Yahoo、Zoho），可手动改；密码存系统钥匙串。
- **协议**：IMAP4rev1（UID 增量同步、MOVE/UIDPLUS、中文文件夹名、网易 ID 命令）+ SMTP（465 TLS / 587 STARTTLS，AUTH PLAIN/LOGIN）。
- **本地存储**：每账户一个目录，JSON 索引 + `.eml` 原文 + 地址簿，离线可读。
- **HTML 邮件**：用 blitz 排版、CPU 渲染，按原格式显示（CSS 布局、按钮、图片）；远程图片只从公网地址加载。
- **按主题归类**：同主题（忽略 `Re:`、`Fwd:`、`回复：` 等前缀）的邮件合成一行，可在列表菜单或设置里关闭。
- **中英文界面、浅色/深色主题**。
- **可编程**：所有操作都是命令，可通过控制通道、命令行、MCP 调用。

## 运行
```sh
# 演示模式（内存假服务器，不需要真实账户）
cargo run --release -p mailfabrica -- --demo

# 正常使用（数据在 ~/Library/Application Support/MailFabrica，可用 --data 或 MAILFABRICA_HOME 改）
cargo run --release -p mailfabrica

# 命令行
cargo run -p mailfabrica-cli -- --demo list
cargo run -p mailfabrica-cli -- account add me@qq.com --password-stdin
cargo run -p mailfabrica-cli -- sync && cargo run -p mailfabrica-cli -- list --unread
```
QQ、163、126 等邮箱要先在网页版设置里开启 IMAP/SMTP，并用 **授权码** 代替登录密码。Gmail 用应用专用密码。

命令行见 `docs/cli.md`，MCP 见 `docs/mcp.md`，控制通道见 `docs/control-protocol.md`。

## 开发
- 架构与约定：`AGENTS.md`；发版：`docs/release.md`。
- 本地 macOS 应用与 DMG：`cargo xtask package`（见 `docs/local-build.md`）；预览 logo：`cargo xtask icon`。
- 提交前：`cargo xtask ci`（fmt、clippy -D warnings、测试、分层检查）。

## 致谢
本项目在 [Claude Code](https://www.anthropic.com/claude-code)（Anthropic 的 Claude）协助下开发。

## 许可
MIT OR Apache-2.0。
