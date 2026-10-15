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
| `ui.drag` | `{x, y, toX, toY, shift?}` | 按住左键从 (x, y) 拖到 (toX, toY) 再松开（选文字） |
| `ui.key` | `{key, shift?, cmd?, alt?, ctrl?}` | 键盘按键 |
| `ui.text` | `{text}` | 输入文字 |
| `ui.calendar` | `{on?, month?, day?}` | 打开/关闭日历视图，见下面的“日历” |
| `app.quit` | | 退出程序 |

其他方法名如果恰好是命令 id（例如 `mail.sync`），就直接执行这个命令，参数就是 `params`。

`engine.execute` 在界面程序里是异步的：`mail.sync`、`compose.send`、`message.open` 只是启动后台任务就返回。
要等结果，轮询 `app.status`（看 `busy` 变成 `false`）和 `message.get`（看 `state` 变成 `ready`）。
无界面的 `mailfabrica-cli mcp` 会自己等后台任务结束。

## 按主题归类（学 Foxmail）

偏好 `groupBySubject`（默认开）打开时，列表里主题相同的邮件合成一行：去掉开头的 `Re:`、`Fwd:`、`回复：`、
`转发：`、`答复:`、`Re[2]:` 这类前缀，压缩空白，不分大小写后比较；空主题不归类。行按组里最新一封的日期排，
行上显示最新一封的主题、预览和时间，发件人后面有封数角标；组里有未读就显示未读点，有红旗就显示红旗。

| 命令 / 字段 | 说明 |
|---|---|
| `app.prefs {groupBySubject: bool}` | 开关归类（列表右上角视图菜单和设置里的同一个开关），改了马上重排列表，存进 `prefs.json` |
| `view.selectGroup {group}` / `{subject}` / `{key}` | 打开主题组：`group` 是归类键，`subject` 是任意写法的主题（会归一化），`key` 是组里某封邮件。选中组里全部邮件、不聚焦单封，阅读窗格显示这一组的列表。返回 `{group, subject, count, unread, flagged, keys}` |
| `view.groups`（查询） | `{enabled, rows, open, groups: [{group, subject, count, unread, flagged, keys}]}` |
| `view.next` / `view.prev` | 按行走：主题组算一行，走到组上就打开整组；返回 `{key, group?}` |
| `ui.inspect` → `grouping`、`rows`、`group`、`groups` | 开关、列表行数、打开的组（归类键）、全部主题组 |
| `ui.inspect` → `reader.view`、`reader.groupBack` | 阅读窗格显示 `group`（组列表）还是 `message`；打开的邮件属于哪个组（顶上有返回横条） |

打开主题组时 `focused` 为空、`selected` 是组里全部邮件，所以删除、移动、标记已读/红旗、垃圾邮件作用于整组；
回复、全部回复、转发不可用（要先在组列表里点开一封）。

可点击目标：列表行 `list.row.<最新一封的键>`、视图菜单 `list.viewMenu`（菜单里的 `list.viewMenu.groupBySubject`）、
组列表里的每封 `reader.group.<键>`、返回横条 `reader.group.back`。

## 日历

`ui.calendar {on?, month?, day?}` 打开或关闭日历视图（省略 `on` 时切换；给了 `month` 或 `day` 就打开）。
`month` 写成 `2026-10`，翻到这个月；`day` 写成 `2026-10-16`，翻到那个月并选中这天。返回下面 `ui.inspect` 里的 `calendar` 对象。

日历视图保留工具栏和状态栏，左边是小月历和来源过滤，右边是一个月的格子（星期日开头，6 周）。
格子里先放系统日历事件，再放这天收到的邮件；放不下时最后一行是"还有 N 项"。中文界面在右上角显示农历。
打开日历和点刷新时会用 `calendar.load {force: true}` 重读系统日历；翻到别的月时读新的范围。
取消勾选的邮箱、系统日历存在 `ui.json` 的 `calendarHiddenAccounts`、`calendarHiddenCalendars` 里。

| 字段 / 目标 | 说明 |
|---|---|
| `ui.inspect` → `calendar` | `{on, month, since, until, selected, hiddenAccounts, hiddenCalendars, days, events, mails, popup, system: {supported, busy, error, calendars}, error}`；`events`、`mails` 是可见范围里的个数，`popup` 是 `{kind: "event", id, date}` 或 `{kind: "day", date}` |
| `ui.inspect` → `panes.calendar` | 大日历区域的矩形（不在日历模式时为 `null`；日历模式下 `list`、`reader` 为 `null`） |
| `sidebar.calendar`、`sidebar.mail` | 侧边栏底部的邮件 / 日历切换 |
| `calendar.back`、`calendar.mode` | 日历左栏底部的同一个切换（`calendar.back` 回到邮件） |
| `calendar.prev`、`calendar.today`、`calendar.next` | 右上角"‹ 今天 ›" |
| `calendar.refresh` | 左上角刷新（强制重读系统日历）；`calendar.retry` 是拒绝授权提示下的"重试" |
| `calendar.mini.prev`、`calendar.mini.next`、`calendar.mini.<YYYY-MM-DD>` | 小月历的翻页和日期（点日期 = 大日历翻到那个月并选中） |
| `calendar.day.<YYYY-MM-DD>` | 大日历的格子：单击选中，双击（`ui.click {target, count: 2}`）弹出这天的全部内容 |
| `calendar.more.<YYYY-MM-DD>` | 格子里的"还有 N 项"，弹出这天的全部内容 |
| `calendar.event.<id>` | 事件条（跨天事件只登记第一格），点开看详情 |
| `calendar.mail.<键>` | 邮件行：离开日历，`view.scope` 到邮件所在文件夹再 `view.select` 打开它 |
| `calendar.popup.event.<id>`、`calendar.popup.mail.<键>` | 弹出框里的事件和邮件 |
| `calendar.account.<账户 id>`、`calendar.cal.<日历 id>` | 来源过滤的勾选框 |

弹出框点外面或按 Esc 关闭。

## 无界面的做法

不开窗口时用 `mailfabrica-cli`（加 `--demo` 可以先试用），见 [`cli.md`](cli.md)；
给 AI agent 用 MCP，见 [`mcp.md`](mcp.md)。
