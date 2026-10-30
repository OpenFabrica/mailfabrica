# MailFabrica

[English](README.md) | 简体中文

基于 **Go + Wails v3** 的桌面邮件客户端，前端使用 React、TypeScript 和 Vite。界面参考 Magpie 的配色与控件，采用 Foxmail 风格的紧凑三栏布局：邮箱、邮件列表和阅读窗格直接占满窗口。

[下载安装包](https://github.com/OpenFabrica/mailfabrica/releases/latest) · [更新日志](CHANGELOG.md) · [MCP 接入](docs/mcp.md) · [开发文档](docs/architecture.md)

## 功能

### 邮箱账户与同步

- 多账户 IMAP 收信、SMTP 发信，支持 TLS 和 STARTTLS。
- 按邮箱服务商预填服务器，支持 QQ、网易、Gmail、iCloud 等预设，也可自定义主机、端口和加密方式。
- 企业邮箱支持自有域名；网易企业邮箱提供客户端 ID 和备用服务器兼容处理。
- 每个账户独立进行后台同步，支持手动收取和定时收取。
- UID 增量同步、邮件标记同步和 UIDVALIDITY 变更检测；已缓存邮件可离线阅读。
- 侧栏显示服务商图标、完整邮箱地址、文件夹及未读数；支持创建文件夹。

### 邮件列表与阅读

- 全部收件箱、未读、红旗和发票视图，支持本地搜索、时间排序和日期分组。
- 紧凑双行列表、多选、已读/未读、红旗、移动、删除、标为垃圾邮件和整文件夹标为已读。
- 按归一化主题聚合邮件，提供会话概览和专用的组内上一封、下一封导航。
- HTML 与纯文本阅读，保留表格、排版和 CID 内嵌图片；可保存附件。
- 解码中文主题、发件人及收件人的 MIME 编码词，兼容已有缓存。
- 邮件头和快速回复区固定，正文独立滚动；常用菜单点击外部关闭。
- 正常 HTTP、HTTPS 和 mailto 链接经校验后交给系统应用打开，支持静态取消订阅链接。

### 写信与回复

- 独立写信窗口，新建、回复、全部回复、转发、保存草稿和继续编辑草稿。
- 收件人、抄送、密送均支持多个邮箱、地址标签、批量粘贴及联系人自动补全。
- 富文本支持字号、粗体、斜体、下划线、删除线、文字颜色、链接、列表、引用和清除格式。
- 添加和移除附件、账户签名、纯文本/富文本切换及已发送副本。
- 原邮件以独立引用区域显示，保留 HTML 排版和内嵌图片，可移除引用。
- 关闭写信窗口或退出应用时保护未保存内容；正在发送时提示等待。

### 联系人与往来时间线

- 添加、查看和删除联系人。
- 根据地址簿、账户及已缓存往来，按姓名或邮箱前缀匹配联系人；使用加权 Trie 排序并按邮箱去重。
- 在邮件阅读区点击“往来时间线”、发件人/收件人名称，或从联系人列表直接查看历史。
- 默认汇总全部账户，可按账户筛选；按月份及收发方向展示，并标识来源邮箱和文件夹。
- 合并同一 Message-ID 的副本，支持分页、刷新、打开原邮件及给联系人写信。

### AI 助手

- 支持 Anthropic 和 OpenAI 兼容协议，可配置接口地址、模型及 API 密钥。
- 起草、润色、正式/友好语气、缩短、扩写和翻译邮件正文。
- 根据当前邮件生成快速回复建议；预览后由用户采用或放弃。
- 聚合邮件一键汇总，生成时间线、结论和待办；明确显示来源封数及错误。
- AI 仅在用户主动操作时调用配置的服务，不自动发送邮件；相关邮件文本会发送至该服务。

### 日历与发票

- 月历同时展示邮件与日程，支持农历、今天定位和月份切换。
- 邮件以账户色圆点标记，系统日程以日历色竖线和背景区分；悬停查看标题、时间和来源。
- 可分别筛选邮箱和系统日历；macOS 通过 EventKit 读取系统日程。
- 发票视图按时间范围筛选邮件，可勾选导出附件，并生成 UTF-8 CSV 发票清单。

### 桌面体验与安全

- 简体中文/英文、浅色/深色/跟随系统主题，栏宽、折叠状态和偏好持久化。
- 可选菜单栏/系统托盘常驻：关闭主窗口后继续同步，点击图标恢复窗口。
- 菜单栏/托盘显示收件箱未读总数，零未读时隐藏数字；提供收信、设置和退出菜单。
- 邮件在禁止脚本的 iframe 中显示，移除活动内容并设置 CSP；不会执行邮件内 JavaScript。
- 可控制远程图片加载；消息正文与应用桥接隔离，附件和外部输入有大小限制。
- macOS 使用系统钥匙串、Windows 使用系统凭据管理器；Linux 当前默认使用权限受限的本地凭据文件，该文件不提供加密存储。

### CLI、MCP 与自动化

- 桌面、CLI、MCP 和本地 JSON 行控制通道共用一套 Go 命令引擎，当前提供 78 个命令。
- CLI 支持账户、同步、文件夹、列表、搜索、阅读、发送及直接执行命令。
- stdio MCP 提供账户、文件夹、查信、读信、发信、状态、命令发现和 `mail_execute` 通用调用。
- MCP 可独立运行，或通过 `--control` 连接桌面会话；后者与界面共享状态。
- 联系人时间线、聚合汇总、AI、日历和发票同样可通过通用命令入口调用。

## 平台与安装

从 [Releases](https://github.com/OpenFabrica/mailfabrica/releases/latest) 下载对应架构的包，校验文件见同页的 `SHA256SUMS`。

| 平台 | 架构与包 | 运行要求与更新方式 |
| --- | --- | --- |
| macOS | arm64 DMG，Developer ID 签名与 Apple 公证 | 构建目标为 macOS 13+；拖入 Applications。已安装的 Go 版支持下载、校验和退出后的自动替换。 |
| Windows | amd64 / arm64 ZIP，未签名 | 需要 WebView2；解压后运行 `mailfabrica.exe`。更新包下载校验后手动替换。 |
| Linux | amd64 / arm64 tar.gz，未签名 | 发行包使用 GTK3 / WebKitGTK 4.1；解压后运行 `mailfabrica`，更新时手动替换。 |

macOS 系统日历首次使用需要授权；Windows/Linux 当前不读取系统日历。Linux 托盘是否显示取决于桌面环境的 StatusNotifierItem 支持。

**从 0.1.5 或更早版本首次迁移时，请退出旧版并手动安装新包，保留账户与邮件数据目录。** 两代更新辅助进程的交接协议不同，详见[更新说明](docs/updates.md)。

## CLI 与 MCP

macOS 安装包内的 CLI 位于 `MailFabrica.app/Contents/MacOS/mailfabrica-cli`；Windows/Linux 的 CLI 与桌面程序放在同一目录。以下以 CLI 已加入 `PATH` 为例：

```sh
mailfabrica-cli --demo list
mailfabrica-cli --demo commands --json
mailfabrica-cli --demo exec contacts.timeline '{"email":"sender4@example.com"}'
mailfabrica-cli --demo mcp
```

连接桌面会话时，先使用 `mailfabrica --control 7981` 启动桌面程序，再配置 MCP 客户端：

```json
{
  "mcpServers": {
    "mailfabrica": {
      "command": "/absolute/path/to/mailfabrica-cli",
      "args": ["mcp", "--control", "7981"]
    }
  }
}
```

将 `command` 替换为实际路径。桌面正在使用数据目录时，通过控制端口共享同一会话，避免另开无界面进程同时写入该目录。协议和工具参数见 [MCP 文档](docs/mcp.md)。

## 开发与验证

需要 Go（版本见 `go.mod`）、Node.js 22 及平台原生编译环境。macOS 使用 Xcode Command Line Tools。Linux 发行包使用 `GOFLAGS=-tags=gtk3` 配合 GTK3/WebKitGTK 4.1；默认构建使用 GTK4/WebKitGTK 6。

```sh
make deps
make demo                 # 隔离演示邮箱，不读取真实账户或凭据
make check                # 前端构建/格式、Go vet、测试及 race 检查
make build                # 桌面程序与 CLI
make package              # 本机包；macOS 默认是开发用 ad-hoc 签名
python3 scripts/smoke.py   # 隔离演示数据上的收发、发票和日历闭环
```

前端热更新：

```sh
./dist/mailfabrica --demo --web 7982
npm run dev --prefix frontend -- --port 5184
```

访问 `http://127.0.0.1:5184`，前端代理连接同一个 Go 引擎。原生文件对话框、系统日历和桌面集成需在 Wails 应用中验证。

当前源码仅保留 Go + Wails 实现，版本由 `VERSION` 管理。macOS、Windows amd64/arm64、Linux amd64/arm64 均配置原生 CI 检查及隔离演示；这些检查不等于覆盖所有操作系统版本、邮件服务商和用户桌面环境。

## 数据兼容与文档

沿用账户 JSON、文件夹索引、`.eml` 原文、联系人、消息键和 FNV-1a 文件夹目录。旧 `ui.json` 中的相关偏好会被读取，原文件保留。macOS 默认目录为 `~/Library/Application Support/MailFabrica`，可用 `--data` 或 `MAILFABRICA_HOME` 指定目录。备份数据时也应保留对应凭据存储。

- [架构与领域规则](docs/architecture.md)、[迁移记录](docs/go-wails-migration.md)
- [账户设置](docs/account-settings.md)、[网易企业邮箱](docs/enterprise-mail.md)
- [邮件阅读](docs/mail-reading.md)、[HTML 正文](docs/html-preview.md)、[聚合邮件](docs/conversations.md)
- [联系人往来时间线](docs/contact-timeline.md)、[发票与日历命令](docs/cli.md)
- [MCP](docs/mcp.md)、[控制协议](docs/control-protocol.md)、[菜单栏常驻](docs/menu-bar.md)
- [本地构建与签名](docs/local-build.md)、[跨平台构建](docs/portable-release.md)、[更新](docs/updates.md)

## 许可

MIT OR Apache-2.0。Magpie 仅作为界面参考，未引入其网关、插件或遥测业务。
