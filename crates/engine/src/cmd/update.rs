//! 更新命令不接受自定义来源；界面、CLI、MCP 共用相同状态与执行路径。
use super::{CommandSpec, always, bad, cmd};
use crate::{Result, Session, UpdatePhase};
use serde_json::{Value as Json, json};

/// 更新操作和状态查询的命令表。
pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!("update.check", "Check for Updates", ["Help"], None, "{}", idle, check),
        cmd!("update.download", "Download and Install Update", [], None, "{}", available, download),
        cmd!("update.open", "Install Update", [], None, "{}", ready, open),
        cmd!("update.cancel", "Cancel Update", [], None, "{}", preparing, cancel),
        cmd!(query "update.status", "Update Status", [], None, "{}", always, status),
    ]
}

/// 准备阶段允许取消；辅助进程尚未获得正常退出提交。
fn preparing(s: &Session) -> std::result::Result<(), String> {
    if s.update().phase == UpdatePhase::Installing { Ok(()) } else { Err("No installation is being prepared".into()) }
}

/// 取消准备任务，不退出当前应用。
fn cancel(s: &mut Session, p: &Json) -> Result<Json> {
    empty("update.cancel", p)?;
    s.cancel_update();
    Ok(json!(s.update()))
}

/// 任务进行中禁止第二次触发；任何终止阶段都可重新检查。
fn idle(s: &Session) -> std::result::Result<(), String> {
    if s.update().busy() { Err("An update task is already running".into()) } else { Ok(()) }
}

/// 检查出正式版之后才可下载，失败时保留元数据以便重试。
fn available(s: &Session) -> std::result::Result<(), String> {
    idle(s)?;
    if s.update().release.is_some() && matches!(s.update().phase, UpdatePhase::Available | UpdatePhase::Failed | UpdatePhase::Cancelled) {
        Ok(())
    } else {
        Err("Check for an available update first".into())
    }
}

/// 已有验证通过的安装包才可再次安装。
fn ready(s: &Session) -> std::result::Result<(), String> {
    idle(s)?;
    if matches!(s.update().phase, UpdatePhase::Ready | UpdatePhase::Cancelled) && s.update().installer.is_some() {
        Ok(())
    } else {
        Err("No verified installer".into())
    }
}

/// 拒绝传入额外参数，避免把命令当成任意下载/打开文件的接口。
fn empty(id: &str, p: &Json) -> Result<()> {
    if p.as_object().is_some_and(|p| p.is_empty()) { Ok(()) } else { Err(bad(id, "this command accepts only {}")) }
}

/// 显式检查，不自动弹窗；GUI 自己打开更新对话框。
fn check(s: &mut Session, p: &Json) -> Result<Json> {
    empty("update.check", p)?;
    s.check_update()?;
    Ok(json!(s.update()))
}

/// 显式下载，验证及准备完成后交接自动安装。
fn download(s: &mut Session, p: &Json) -> Result<Json> {
    empty("update.download", p)?;
    s.download_update()?;
    Ok(json!(s.update()))
}

/// 重新验证并安装已下载的更新包。
fn open(s: &mut Session, p: &Json) -> Result<Json> {
    empty("update.open", p)?;
    s.reopen_installer()?;
    Ok(json!(s.update()))
}

/// 查询过程与最终结果，支持远程轮询进度。
fn status(s: &mut Session, p: &Json) -> Result<Json> {
    empty("update.status", p)?;
    Ok(json!(s.update()))
}
