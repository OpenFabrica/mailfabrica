# 应用更新

Go 更新适配只查询固定 `OpenFabrica/mailfabrica` GitHub 发布源，不接受调用方提供任意下载 URL。选择本机平台/架构、要求稳定版本、校验服务端 SHA-256 摘要与大小；macOS 还检查 Apple Team `HQ537XMLJY`、代码签名与 Gatekeeper。

命令为 `update.check`、`update.download`、`update.open`、`update.cancel`、`update.status`。状态保留下载进度、安装器路径和可重试错误。

macOS 正常安装的 `.app`：下载校验 → 只读挂载 DMG → 校验应用标识与发布者 → 同卷暂存 → 启动辅助副本 → 主程序正常退出时写交接标记 → 辅助程序替换并重启。没有交接标记时不替换；改名失败恢复旧包。新应用成功建立前端快照后才清理旧包备份。

开发二进制和 portable 平台使用验证后的安装器手动安装。当前本地 ad-hoc 包不满足正式发布者验证，也不会被伪装为可自动安装的正式发行物。

替换和缺少交接标记的分支在临时目录测试；真实签名、公证、安装和跨版本重启需要正式发布环境验收，不能用这些测试代替。
