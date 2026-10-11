# MailFabrica — 给代理（agent）的说明

MailFabrica 是一个 Rust 原生的桌面邮件客户端，界面和使用习惯借鉴 Foxmail（三栏布局、收取/写邮件/回复/转发工具栏、
快速回复、地址簿、按邮箱自动配置服务器）。技术栈和工程约定沿用 `../openfabrica/gridfabrica`：分层 crate、
egui/eframe 界面、一切皆命令、控制通道、MCP、`cargo xtask`。

## 分层（`cargo xtask layers` 强制检查）
| 层 | crate | 作用 |
|---|---|---|
| L0 | `core` | 类型：地址、账户、服务商预设、文件夹、邮件元数据、日期、错误、密码存储接口 |
| L1 | `mime` | 解析原始邮件 → 元数据/阅读视图；组装待发送邮件；引用/转发正文 |
| L1 | `proto` | 阻塞式 IMAP4rev1 + SMTP 客户端（rustls，不用异步运行时） |
| L2 | `store` | 本地存储：每账户一个目录，JSON 索引 + `.eml` 原文 + 联系人 |
| L3 | `sync` | 后台工作线程（每账户一条），`Connector`/`Remote` 抽象，内存假服务器 `FakeConnector` |
| L5 | `engine` | `Session`、命令注册表、视图、写信状态；界面/CLI/MCP 唯一入口 |
| L6 | `ui-egui`、`mcp` | Foxmail 风格界面 + 控制通道；MCP 服务器 |
| L7 | `apps/mailfabrica`、`apps/mailfabrica-cli` | 桌面程序；命令行 |

- L6 以下不能依赖 egui/eframe/winit/rfd/wgpu。**界面 crate 可以整体替换。**
- `mime` 和 `proto` 互不依赖。后台线程不碰磁盘，只发事件；只有引擎线程持有 `Store`。

## 一切皆命令
用户能做的事 = `crates/engine/src/cmd*` 里的一个命令（id、标签、菜单、快捷键、参数说明、`enabled`、`run`）+ 测试。
命令表见 `crates/engine/src/cmd.rs`（或 `cmd/mod.rs`）的模块注释。界面只读 `Session` 状态，改状态只走
`session.run(id, params)`。程序化调用不弹对话框：只有 `account.add`/`account.update` 空参数时请求对话框。

## 永不崩溃
- 非测试代码不用 `unwrap()`、`expect()`、`panic!`、`unreachable!`、`todo!`、`unimplemented!`；不用 `unsafe`。
- 服务器回复、邮件内容、命令参数都是不可信输入：用 `get()`，不做可能 panic 的切片，限制分配大小和循环次数。
- `Session::execute` 和后台任务都用 `catch_unwind` 兜底，但这只是安全网。
- 每个 crate 根上都有 `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]`。

## 素材
不用 Foxmail/腾讯或任何厂商的图标、图片、字体。图标全部在 `crates/ui-egui/src/icons.rs` 里用代码画。
中文字体在运行时从系统加载，不放进仓库。

## 运行与检查
- 演示模式（内存假服务器，不需要真实账户）：`cargo run -p mailfabrica -- --demo --control 7981`
- 控制通道：向 `127.0.0.1:7981` 发 JSON 行，例如
  `{"id":1,"method":"engine.execute","params":{"command":"compose.reply","params":{}}}`、
  `{"id":2,"method":"ui.screenshot","params":{"path":"/tmp/shot.png"}}`。协议见 `docs/control-protocol.md`。
- 命令行：`cargo run -p mailfabrica-cli -- --demo list`；MCP：`mailfabrica-cli mcp`（见 `docs/mcp.md`）。
- **改界面就看截图**，不要只看代码。
- 提交前：`cargo xtask ci`（fmt、clippy -D warnings、测试、分层检查）。
- Shell 注意：`mv`/`cp` 有交互别名，用 `/bin/mv -f`、`/bin/cp -f`；macOS 没有 `timeout`，用 `perl -e 'alarm 60; exec @ARGV' cmd`。
- 并行代理：每个代理用自己的 `CARGO_TARGET_DIR`，只改自己负责的 crate，结束后删掉自己的 target 目录。

## 代码风格
- 注释用中文；代码标识符、命令 id 用英文。
- 每个新函数/结构体顶部有一行说明；不直观的逻辑写清楚"为什么"。
- rustfmt `max_width = 150`。
