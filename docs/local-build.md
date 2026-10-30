# 本地构建

```sh
make deps
make check
make package
./dist/MailFabrica.app/Contents/MacOS/mailfabrica --demo --control 7981
```

`make package` 只打包本机平台。macOS 输出 `dist/MailFabrica.app` 与 `dist/mailfabrica-cli`，代码生成图标并使用 ad-hoc 签名；Windows 输出 portable ZIP，Linux 输出 tar.gz。
Go 与前端产物均来自当前 worktree，不调用 Cargo。构建版本读取 `VERSION`。macOS 构建将编译/链接目标设为 13.0；这不代表已在 macOS 13 实机验收。

真实数据目录可用 `--data DIR` 或 `MAILFABRICA_HOME` 指定。`--demo` 不给目录时创建并回收自己的临时目录。

开发时可用 `make dev` 打开原生演示程序。需要前端热更新时，在两个终端运行 `./dist/mailfabrica --demo --web 7982` 与 `npm run dev --prefix frontend -- --port 5184`，浏览器访问 `http://127.0.0.1:5184`。

`python3 scripts/smoke.py` 会启动独立 Go 宿主，验证邮件、富文本发送、附件、发票文件和日历数据；只使用临时演示目录。

## macOS DMG、Developer ID 签名与公证

使用本机已有证书和钥匙串 profile，不将 Apple 密码、API 私钥写入脚本：

```sh
APPLE_SIGN_IDENTITY='Developer ID Application: 你的名称 (TEAMID)' \
APPLE_NOTARY_PROFILE='已有的钥匙串 profile' \
make package PACKAGE_ARGS='--notarize'
```

输出在 `dist/MailFabrica-<版本>-<源码摘要>-<时间>-<架构>/`，包含 `.app`、拖入 Applications 的 DMG、`build.json` 与 Apple 公证回执/日志。
按照 [Apple 公证流程](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)，先签名和公证应用并装订票据，再制作/签名/公证 DMG 并装订票据；均检查 Gatekeeper。CLI 随应用放在 `Contents/MacOS` 并单独签名。

该命令只制作安装包，不提交、改版本、推送或创建 Release。允许明确用于本地验收的开发版与未提交源码，清单记录 dirty、完整源码摘要及产物哈希；正式公开发行仍遵循 [release.md](release.md) 的版本与提交门禁。
不要在运行中的开发 `.app` 上原地重新签名；签名安装包始终写入独立目录。

## Windows / Linux 未签名包

```sh
make frontend
python3 scripts/portable.py --target windows --arch amd64
python3 scripts/portable.py --target windows --arch arm64
python3 scripts/portable.py --target linux --arch amd64
python3 scripts/portable.py --target linux --arch arm64
```

Windows 使用无 CGO 的 Go 交叉构建，输出 ZIP；Linux 使用 Docker Debian 12 + GTK3/WebKitGTK 4.1 的隔离构建容器，输出 tar.gz。
两者不调用签名工具，应用和 CLI 均随包附带。Linux 在容器内运行协议/引擎测试及 HTTP 演示闭环；它不代替目标机器的图形桌面验收。Windows 运行需要 WebView2，交叉编译不代表实机测试通过。
