# MailFabrica

当前工程为 Go + Wails v3，前端使用 React、TypeScript、Vite 和 Tiptap。

- 项目协作规则见 `AGENTS.md`；其中历史工程的分层路径和构建命令已被本次迁移替代，当前实现以 `docs/architecture.md` 和 `Makefile` 为准。
- 版本、标签、发布与下一开发版本的规则见 `docs/release.md`。版本来源是 `VERSION`。
- 保留已有用户改动；所有演示、测试和自动化验证使用独立数据目录。

## 常用命令

```sh
make deps
make check
make build
make demo
python3 scripts/smoke.py
python3 scripts/native-smoke.py  # 仅 macOS 图形桌面
./dist/mailfabrica-cli --demo commands --json
./dist/mailfabrica-cli --demo mcp
```

`make check` 执行前端构建、格式检查、Go vet、测试和 race 检查。命令参数和后台任务均应通过真实引擎验证，不将构建成功等同于界面可用。

## 代码入口与约束

- `internal/mail`：账户、地址、文件夹和消息类型，不依赖桌面框架、网络或存储。
- `internal/engine`：`Session.Execute`、命令编排、读取快照和后台状态；命令目录为 `commands.json`。
- `internal/store`：兼容 JSON/EML 存储。FNV-1a 文件夹名、消息键和凭据服务名属于兼容性约定，不能随意更换。
- `internal/protocol`：IMAP、SMTP、MIME 和内存演示服务器；不直接持有 Store。
- `internal/platform`：凭据和 macOS 系统日历适配。
- `internal/control`：CLI/MCP/JSON 行/HTTP 验收的共享边界。
- `internal/desktop`、`cmd/mailfabrica`：Wails 桥接、原生窗口、菜单栏和应用生命周期。
- `frontend/src`：按功能组织组件；业务变更通过命令和快照，避免在 UI 中复制规则。
- `scripts/`：检查、打包、签名公证和隔离验收。Windows/Linux 包不签名。

后台任务在会话锁内应用结果，每账户网络任务串行。正文下载、发送、AI 和更新需等待可观察的完成状态，不能只验证“任务已开始”。

HTML 邮件在禁用脚本的 iframe 中显示，相关实现为 `email-content.tsx` 和 `email-links.tsx`。不要将正文并入应用组件，不开放任意脚本或桥接调用。UI 改动需查看实际浏览器或原生窗口图像；控制通道 DOM 截图不包含正文 iframe。

## 自动化与数据

控制通道仅监听回环地址，协议见 `docs/control-protocol.md`。`ui.click` 的目标对应前端 `data-testid`；`ui.inspect` 返回视图、布局和阅读状态。合成点击不等于真实系统指针验收。

MCP 的默认后端为独立引擎，`--control PORT` 连接桌面会话。不要让多个写入进程同时使用真实数据目录。

演示数据在 `internal/engine/demo.go`，协议演示服务器在 `internal/protocol/memory.go`；引擎测试使用 `testSession` 和隔离临时目录。测试不读取真实凭据、日历或邮件。

README 为中英文双版本，功能和使用方式需同步。主分支为 `master`，仓库为 `OpenFabrica/mailfabrica`。
