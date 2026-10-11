# MailFabrica 控制通道协议

启动桌面程序时打开控制端口：

```sh
mailfabrica --control 7981            # 或环境变量 MAILFABRICA_CONTROL_PORT=7981
```

然后向 `127.0.0.1:7981` 发按行分隔的 JSON。每个请求行得到一个回复行：

```json
{"id": 1, "method": "engine.execute", "params": {"command": "mail.sync", "params": {}}}
{"id": 1, "ok": true, "result": null}
```

出错时回复 `{"id": …, "ok": false, "error": "…"}`。默认端口是 7981。只监听回环地址 `127.0.0.1`。

## 方法

| 方法 | 参数 | 作用 |
|---|---|---|
| `engine.execute` | `{command, params}` | 执行任意引擎命令（命令表见 `engine.commands` 或 `mailfabrica-cli commands`） |
| `engine.commands` | `{search?}` | 全部命令：id、标签、菜单、快捷键、参数说明、当前是否可用 |
| `engine.journal` | | 已执行的命令（可重放） |
| `ui.inspect` | | 界面状态：对话框、写信窗口、选中邮件、布局矩形 |
| `ui.screenshot` | `{path?}` | 窗口 PNG（不给 `path` 时返回 base64） |
| `ui.click` | `{x, y, button?, count?}` | 在窗口坐标处点击 |
| `ui.key` | `{key, shift?, cmd?, alt?, ctrl?}` | 键盘按键 |
| `ui.text` | `{text}` | 输入文字 |
| `app.quit` | | 退出程序 |

其他方法名如果恰好是命令 id（例如 `mail.sync`），就直接执行这个命令，参数就是 `params`。

`engine.execute` 在界面程序里是异步的：`mail.sync`、`compose.send`、`message.open` 只是启动后台任务就返回。
要等结果，轮询 `app.status`（看 `busy` 变成 `false`）和 `message.get`（看 `state` 变成 `ready`）。
无界面的 `mailfabrica-cli mcp` 会自己等后台任务结束。

## 无界面的做法

不开窗口时用 `mailfabrica-cli`（加 `--demo` 可以先试用），见 [`cli.md`](cli.md)；
给 AI agent 用 MCP，见 [`mcp.md`](mcp.md)。
