# 控制通道

启动：`./dist/mailfabrica --demo --control 7981`。仅监听 `127.0.0.1`，每个 JSON 行请求得到一个 JSON 行响应：

```json
{"id":1,"method":"engine.execute","params":{"command":"message.list","params":{"limit":20}}}
```

成功为 `{id, ok:true, result}`，失败为 `{id, ok:false, error}`。`MAILFABRICA_CONTROL_PORT` 可设置默认端口。

| 方法 | 作用 |
| --- | --- |
| `engine.execute {command, params}` | 执行稳定业务命令 |
| `engine.commands {search?}` | 命令目录、参数说明和当前 enabled |
| `engine.journal` | 脱敏命令历史，密码/API 密钥不会回显 |
| `ui.inspect {window?}` | 会话、可见布局、对话框、阅读状态及协议能力 |
| `ui.screenshot {path?, window?}` | 原生 WebView 生成 DOM PNG；不给路径时返回 base64 |
| `ui.click {target? 或 x,y, count?, window?}` | 合成 DOM 点击，target 对应 `data-testid` |
| `ui.text {text, window?}` | 为聚焦编辑区域输入内容 |
| `ui.key {key, cmd?, ctrl?, alt?, shift?, window?}` | 合成按键 |
| `ui.calendar {on?, month?, day?}` | 切换日历，支持月份/日期 |
| `ui.readerMode {mode:"html" 或 "plain"}` | 阅读模式 |
| `ui.readerReload {}` | 重载当前正文 |
| `ui.readerScroll {top}` | 正文滚动，范围 0–200000 |
| `ui.readerSelect {start,end}` | 按 Unicode 字符偏移选择正文 |
| `ui.readerCopy {}` | 复制当前正文选区 |
| `ui.readerHitTest {x,y}` | 查询正文局部坐标语义，不移动真实指针 |
| `ui.focusWindow {}` | 显式聚焦主窗口 |
| `app.quit {}` | 请求退出 |

其他已注册命令 id 也可直接作为 method 使用。网络操作异步开始，轮询 `app.status.busy` 和 `message.get.state` 等结果；CLI/MCP 后端会等待适用任务完成。

## Wails 协议 v2 的 UI 差异

业务命令继续保留原 id、消息键及主要参数。UI 检查返回 `protocolVersion:2`、`backend:wails-webview` 和 `capabilities`，旧 egui GPU 矩形/光标字段不再伪造。

- 三栏坐标是 WebView 客户端坐标。默认目标 `window:"main"`；写信窗口可显式指定 `window:"compose-<id>"`。
- `ui.screenshot` 是 DOM 导出，返回 `backend:dom-without-email-body`、`includesEmailBody:false`，不包含隔离正文 iframe。完整正文视觉需浏览器或真实窗口截图。
- `ui.drag` 不再模拟 egui 输入队列；正文选择使用 `ui.readerSelect`，真实拖选和悬停使用系统输入。
- 合成点击、DOM 截图、hitTest 不能证明真实系统指针、遮挡状态或原生窗口焦点。
- 正文 iframe 禁止脚本。固定阅读操作由宿主执行；接口不提供任意 JavaScript。

命令参数和日志属于敏感边界；控制端口只应在需要时启用，不提供公网监听选项。

菜单栏常驻的固定原生入口：`ui.tray {}` 返回启用状态、原生图标是否已创建、未读数、显示标签和主窗口状态；`ui.closeWindow {}` 走真实窗口关闭钩子；`ui.showWindow {}` 或 `ui.focusWindow {}` 恢复主窗口。常驻设置仍通过 `engine.execute → app.prefs {closeToTray: true}` 修改。

`ui.readerActivateLink {text}` 只激活正文中唯一且文字完全匹配的静态链接，走与用户点击相同的协议校验与系统浏览器处理，不接受脚本。
