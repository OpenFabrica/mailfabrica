# MailFabrica MCP 服务器

`mailfabrica-cli mcp` 通过 [Model Context Protocol](https://modelcontextprotocol.io) 把 MailFabrica 交给 AI agent：
stdin/stdout 上按行分隔的 JSON-RPC 2.0（协议版本 `2025-06-18`；`2025-03-26`、`2024-11-05` 的客户端会得到它自己的版本）。
日志写 stderr。

两种后端：

| 模式 | 命令 | 驱动什么 |
|---|---|---|
| 无界面（默认） | `mailfabrica-cli mcp [--data DIR] [--demo]` | 进程内的引擎会话，没有窗口。收信、发信、下载正文会等到完成再返回。 |
| 远程 | `mailfabrica-cli mcp --control 7981` | 用 `mailfabrica --control 7981` 启动的桌面程序（回环 TCP JSON 行，见 [control-protocol.md](control-protocol.md)）。 |

`--demo` 用内存里的演示邮箱，不碰网络也不碰真实数据，适合先试一遍。无界面模式和桌面程序同时用同一个数据目录可能冲突，
想同时操作请用远程模式。

## Claude Code / Claude Desktop 配置

```sh
claude mcp add mailfabrica -- mailfabrica-cli mcp
# 或者驱动你正在看的桌面程序：
claude mcp add mailfabrica-app -- mailfabrica-cli mcp --control 7981
```

JSON 配置（`.mcp.json`、Claude Desktop、其他客户端）：

```json
{
  "mcpServers": {
    "mailfabrica": {
      "command": "mailfabrica-cli",
      "args": ["mcp"]
    }
  }
}
```

连桌面程序时用 `"args": ["mcp", "--control", "7981"]`；先试用演示邮箱用 `["--demo", "mcp"]`。

## 工具

邮件用键 `account/uid/folder` 标识（例如 `me-qq-com/1234/INBOX`），`mail_list` 返回的 `key` 可以直接传给其他工具。

| 工具 | 参数 | 作用 |
|---|---|---|
| `mail_execute` | `command`, `params?` | 执行**任意**命令（`message.flag`、`message.move`、`compose.reply`、`contacts.add` ……） |
| `mail_commands` | `search?` | 全部命令的 id、标签、参数说明、当前是否可用 |
| `mail_accounts` | — | 账户列表（不含密码） |
| `mail_folders` | `account` | 文件夹、角色、未读数、总数 |
| `mail_list` | `account?`, `folder?`, `limit?`, `unreadOnly?`, `group?` | 邮件列表，新的在前；给 `account` 不给 `folder` 时是收件箱；都不给是当前视图。`group: true` 时按主题归类，结果在 `rows` 里（每行有封数、未读数、发件人、全部 `keys` 和最新一封 `message`），`limit` 按行算 |
| `mail_read` | `key` | 邮件头 + 正文文本 + 附件列表；本地没有正文时先下载 |
| `mail_search` | `query`, `limit?` | 在当前范围搜索（文件夹范围会搜该账户全部文件夹）；搜完会清掉搜索词 |
| `mail_send` | `to`, `subject`, `body`, `cc?`, `account?` | 写信并发送（`compose.new` → `compose.set` → `compose.send`）；失败时返回服务器的错误 |
| `mail_status` | — | 同步状态：是否忙碌、进度文字、上次同步时间、各账户错误 |
| `ui_screenshot` | `path?` | 窗口截图（仅远程模式） |
| `ui_inspect` | — | 界面状态（仅远程模式） |

AI 助手和发票导出没有单独的工具，用 `mail_execute` 调命令：`ai.config`（设置服务和密钥）、`ai.compose` / `ai.accept` / `ai.dismiss`（写信助手）、
`ai.quickReply` / `ai.clearReply` / `ai.status`（快速回复）、`invoice.list` / `invoice.export` / `invoice.status`（发票）；
`view.scope` 传 `{"kind": "invoices"}` 切到发票范围。参数见 `mail_commands`。
`ai.*` 只在用户已配置 AI 服务后可用，调用时邮件内容会发给该服务。

`to`、`cc` 可以是字符串（`"a@b.com, c@d.com"`）或字符串数组。`ui_*` 工具只在远程模式的 `tools/list` 里出现。

结果是一个文本块，内容是美化过的 JSON；失败时 `isError: true` 并带可读的信息。未知工具、缺必填参数、参数类型错误是
JSON-RPC `-32602`；JSON 格式错误是 `-32700`；未知方法是 `-32601`。

## 资源

- `mailfabrica://commands`：全部命令（`engine.commands`）。
- `mailfabrica://status`：同步状态（`app.status`）。

`prompts/list` 返回空列表。

## 会话示例

```text
→ {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}
→ {"jsonrpc":"2.0","method":"notifications/initialized"}
→ {"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"mail_list","arguments":{"limit":5}}}
→ {"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"mail_read","arguments":{"key":"demo-mailfabrica-dev/3/INBOX"}}}
→ {"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"mail_send","arguments":{"to":"a@b.com","subject":"你好","body":"测试"}}}
← {"jsonrpc":"2.0","id":4,"result":{"content":[{"type":"text","text":"{\n  \"sent\": true, ..."}],"isError":false}}
```

用管道快速试：

```sh
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | mailfabrica-cli --demo mcp
```
