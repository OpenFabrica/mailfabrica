# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

# MailFabrica

- 工程约定、分层、运行方式（先读）：@AGENTS.md
- 版本号与发版规则：@docs/release.md

## 常用命令

- 全量检查（提交前必须过）：`cargo xtask ci` = `cargo fmt --check` + `clippy -D warnings` + 全部测试 + `cargo xtask layers`。
- 只看分层依赖表：`cargo xtask layers`。
- 单个 crate 的测试：`cargo test -p mailfabrica-engine`；单个测试按名字子串过滤：`cargo test -p mailfabrica-engine rich_reply`。
  crate 包名是 `mailfabrica-<目录名>`（`core`、`mime`、`proto`、`rich`、`store`、`sync`、`html`、`engine`、`ui-egui`、`mcp`）；
  程序是 `mailfabrica`、`mailfabrica-cli`。
- 演示模式 + 控制通道：`cargo run -p mailfabrica -- --demo --control 7981`。演示邮件的键形如 `demo-mailfabrica-dev/<uid>/INBOX`，
  用 `cargo run -p mailfabrica-cli -- --demo list --flat` 查。
- 控制通道 `ui.click {target}` 用的目标名由界面代码里的 `app.target(名字, rect)` 登记（如 `compose.<id>.send`）；
  `ui.inspect` 输出界面和视图状态；`ui.drag` 可拖动窗口、选文字。

## 整体结构

- **一条数据流**：界面 / CLI / MCP / 控制通道 → `Session::execute(命令 id, JSON 参数)`（`crates/engine/src/cmd/*`）
  → 改 `Session` 状态，需要联网时把任务交给每账户一条的 `sync::Worker` 线程 → 线程只发 `Event`，不碰磁盘
  → `Session::pump()`（`engine/src/events.rs`）把事件写进 `Store`、刷新视图、做后续动作。
  界面每帧调 `pump()`；无界面时（`crates/mcp/src/headless.rs`，CLI 和 MCP 都用）等后台空闲再返回；测试里用 `s.wait_idle(WAIT)`。
- **命令表**：`crates/engine/src/cmd/mod.rs` 的模块注释是全部命令的参考表，加命令或改参数时同步改它。
  `cmd!(query …)` 是只读查询。新命令自动出现在 CLI `exec`、MCP `mail_execute`、控制通道 `engine.execute` 里。
- **视图**：`MailView.messages`（已排序）+ `rows`（按主题归类，`engine/src/view.rs::build_rows`，开关是偏好 `groupBySubject`）
  + `selected` / `focused` / `group`。打开主题组 = 选中组里全部邮件、不聚焦单封。
- **正文按需下载**：`BodyState`（Missing/Loading/Ready/Failed）。回复/转发时原邮件正文还没到，先建窗口，
  记在 `awaiting`，正文到了由 `refill_compose` 补引用（用户改过的部分不覆盖）。
- **写信状态全在引擎**（`Compose`）：`body` 纯文本 + `doc: RichDoc`（超文本时 `body == doc.text()`）。
  超文本回复/转发 HTML 邮件时，原文不进可编辑正文，而是只读的 `Compose::quote`（HTML + 纯文本 + 内嵌图片）；
  发送时拼成 Gmail 式 `gmail_quote`，内嵌图片走 multipart/related；存草稿再打开时靠 `data-mf-quote` 标记拆回来。
- **HTML 显示**：`crates/html` 在专用线程里用 blitz 排版、vello_cpu 画 512px 高的图块；
  `ui-egui/src/htmlbody.rs` 的 `HtmlPane` 把图块贴成纹理，阅读窗格和写信窗口的原文引用都用它。渲染器出错时退回 `RichDoc` 渲染。
  - blitz 锁在 `=0.3.0-beta.2`。已知的坑：没有 `base_url` 时 `//host` 地址会 panic（用占位 base）；`<link>` 样式表会阻塞绘制（解析前去掉）；
    `NetProvider` 每个请求都必须回调（失败给空数据）；链接可点区域用 `node_client_rects`，不要用 `hit()`（带 padding 的按钮会错位）。
  - 远程图片（`html/src/net.rs`）只连公网地址：在 DNS 解析这一步过滤，不用环境变量代理。`198.18.0.0/15` 故意放行（Surge/Clash fake-ip）。
- **日历**：`engine/src/calendar.rs`。系统日历由 `CalendarSource` 提供：macOS 用 `/usr/bin/osascript -l JavaScript` 调 EventKit
  （不写 `unsafe` 绑定），后台线程读、`pump()` 收；其他系统 `supported: false`。演示和测试用 `DemoCalendar`，测试别碰真实日历。
  打包时 Info.plist 有日历用途说明，Developer ID 签名带 `com.apple.security.personal-information.calendars`（`xtask/src/notary.rs`）。
- **控制通道**：`apps/mailfabrica/src/control_server.rs`（TCP JSON 行）→ `crates/ui-egui/src/control.rs`（方法实现）。
  MCP（`crates/mcp`）的后端要么是进程内 `Session`，要么用 `--control PORT` 连正在运行的程序。

## 测试与演示数据

- 引擎集成测试在 `crates/engine/src/tests.rs`：`setup()` 用 `FakeConnector`（内存假服务器）建会话，`Session::demo()` 是演示数据。
- 演示数据在 `crates/sync/src/demo.rs`，不少测试按封数和主题断言；加或改演示邮件后要跑引擎测试。

## 仓库

- 主分支是 `master`（`docs/release.md` 门禁里写的 `origin/main` 按 `master` 理解）。远程 `git@github.com:OpenFabrica/mailfabrica.git`，公开仓库。
- `README.md` 是英文版，`README.zh-CN.md` 是中文版，改一个要同步另一个。`docs/` 下的文档是中文。
