//! 更新领域状态：显式检查、下载、保存、验证与安装准备，正常退出后交接辅助进程自动替换。
use crate::{EngineError, Result, Session, UiRequest};
use mailfabrica_update::{Client, Event, Release, Task, Worker};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Arc;

/// 更新流程的可观察阶段；失败后允许重新检查或重新下载。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdatePhase {
    #[default]
    Idle,
    Checking,
    Current,
    Manual,
    Available,
    Downloading,
    Verifying,
    Installing,
    Ready,
    Failed,
    Cancelled,
}

/// 界面、CLI 和 MCP 共用的只读更新状态。
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateState {
    pub phase: UpdatePhase,
    pub release: Option<Release>,
    pub received: u64,
    pub installer: Option<PathBuf>,
    pub error: Option<String>,
    pub manual_url: Option<String>,
}

impl UpdateState {
    /// 忙碌时禁止重复启动任务；独立于邮件同步状态。
    pub fn busy(&self) -> bool {
        matches!(self.phase, UpdatePhase::Checking | UpdatePhase::Downloading | UpdatePhase::Verifying | UpdatePhase::Installing)
    }
}

/// 会话私有更新运行状态，任务号阻止迟到结果覆盖后续请求。
pub(crate) struct Updates {
    worker: Worker,
    state: UpdateState,
    active: Option<u64>,
    next: u64,
    target: Option<mailfabrica_update::InstallTarget>,
    handoff: Option<mailfabrica_update::Handoff>,
}

impl Updates {
    /// 不隐式检查更新；用户触发命令后才联网。
    pub(crate) fn new(client: Arc<dyn Client>) -> Self {
        Self { worker: Worker::new(client), state: UpdateState::default(), active: None, next: 0, target: None, handoff: None }
    }

    /// 登记并启动任务；启动失败立即进入可重试状态。
    fn start(&mut self, task: Task, phase: UpdatePhase) -> Result<()> {
        self.next = self.next.wrapping_add(1);
        self.active = Some(self.next);
        self.state.phase = phase;
        self.state.error = None;
        if let Err(error) = self.worker.start(self.next, task) {
            self.fail(error.clone());
            return Err(EngineError::Other(error));
        }
        Ok(())
    }

    /// 所有失败路径都清除活动任务，防止界面永久忙碌。
    fn fail(&mut self, error: String) {
        self.active = None;
        self.state.phase = UpdatePhase::Failed;
        self.state.installer = None;
        self.state.error = Some(error);
    }
}

impl Session {
    /// 仅宿主程序可配置实际安装位置；公开命令不能指定任意替换路径。
    pub fn configure_update_target(&mut self, target: mailfabrica_update::InstallTarget) {
        self.updates.target = Some(target);
    }

    /// 宿主正常退出时提交已经准备好的更新；异常退出不执行替换。
    pub fn commit_update(&self) -> Result<()> {
        if let Some(handoff) = &self.updates.handoff {
            if self.composes.iter().any(|c| c.dirty) {
                return Err(EngineError::Other("Save or close unsaved messages before updating".into()));
            }
            handoff.commit().map_err(EngineError::Other)?;
        }
        Ok(())
    }

    /// 取消尚未交接的安装；迟到的准备结果没有提交标记，辅助进程不会替换应用。
    pub(crate) fn cancel_update(&mut self) {
        self.updates.handoff = None;
        self.updates.active = None;
        self.updates.state.phase = UpdatePhase::Cancelled;
        self.updates.state.error = None;
        self.revision += 1;
    }

    /// 当前更新状态，供界面与命令查询，不产生网络或磁盘副作用。
    pub fn update(&self) -> &UpdateState {
        &self.updates.state
    }

    /// 检查比正在运行版本更新的稳定版；先丢弃先前选择，避免复用过时元数据。
    pub(crate) fn check_update(&mut self) -> Result<()> {
        self.updates.handoff = None;
        self.updates.state = UpdateState::default();
        let result = self.updates.start(Task::Check(env!("CARGO_PKG_VERSION").into()), UpdatePhase::Checking);
        self.revision += 1;
        result
    }

    /// 下载已经检查出的安装包；参数不允许传入任意 URL、版本或本地路径。
    pub(crate) fn download_update(&mut self) -> Result<()> {
        if self.composes.iter().any(|c| c.dirty) {
            return Err(EngineError::Other("Save or close unsaved messages before updating".into()));
        }
        let release = self.update().release.clone().ok_or_else(|| EngineError::Other("Check for an update first".into()))?;
        self.updates.state.received = 0;
        self.updates.state.installer = None;
        let result = self.updates.start(Task::Download(release), UpdatePhase::Downloading);
        self.revision += 1;
        result
    }

    /// 再次安装前重新检查完整性和发布者，防止缓存被篡改。
    pub(crate) fn reopen_installer(&mut self) -> Result<()> {
        if self.composes.iter().any(|c| c.dirty) {
            return Err(EngineError::Other("Save or close unsaved messages before updating".into()));
        }
        let release = self.update().release.clone().ok_or_else(|| EngineError::Other("No downloaded installer".into()))?;
        let path = self.update().installer.clone().ok_or_else(|| EngineError::Other("No verified installer".into()))?;
        let result = self.updates.start(Task::Verify(release, path), UpdatePhase::Verifying);
        self.revision += 1;
        result
    }

    /// 引擎线程保存下载结果；背景验证与准备完成后才请求正常退出交接。
    pub(crate) fn pump_updates(&mut self) -> bool {
        let events = self.updates.worker.poll();
        let mut changed = false;
        for (id, event) in events {
            if self.updates.active != Some(id) {
                continue;
            }
            changed = true;
            match event {
                Event::Manual(url) => {
                    self.updates.active = None;
                    self.updates.state.phase = UpdatePhase::Manual;
                    self.updates.state.manual_url = Some(url.clone());
                    self.ui_requests.push(UiRequest::OpenUrl(url));
                }
                Event::Checked(release) => {
                    self.updates.active = None;
                    self.updates.state.phase = if release.is_some() { UpdatePhase::Available } else { UpdatePhase::Current };
                    self.updates.state.release = release;
                }
                Event::Progress(bytes) => self.updates.state.received = bytes,
                Event::Downloaded(bytes) => {
                    let result = self.update().release.clone().ok_or("Missing release metadata".to_string()).and_then(|release| {
                        let path = self.store.save_installer(&release.asset, &bytes).map_err(|e| e.to_string())?;
                        self.updates.start(Task::Verify(release, path), UpdatePhase::Verifying).map_err(|e| e.to_string())
                    });
                    if let Err(error) = result {
                        self.updates.fail(error);
                    }
                }
                Event::Verified(path) => {
                    // 下载期间可能又打开或编辑写信窗口，不能在准备完成后丢弃新输入。
                    if self.composes.iter().any(|c| c.dirty) {
                        self.updates.fail("Save or close unsaved messages before updating".into());
                        continue;
                    }
                    self.updates.active = None;
                    self.updates.state.phase = UpdatePhase::Ready;
                    self.updates.state.installer = Some(path.clone());
                    if let Some(target) = self.updates.target.clone() {
                        let result = self.update().release.clone().ok_or_else(|| "Missing update metadata".to_string()).and_then(|release| {
                            self.updates.start(Task::Prepare(release, path, target), UpdatePhase::Installing).map_err(|e| e.to_string())
                        });
                        if let Err(error) = result {
                            self.updates.fail(error);
                        }
                    }
                }
                Event::Prepared(handoff) => {
                    if self.composes.iter().any(|c| c.dirty) {
                        self.updates.fail("Save or close unsaved messages before updating".into());
                        continue;
                    }
                    self.updates.active = None;
                    self.updates.handoff = Some(handoff);
                    self.updates.state.phase = UpdatePhase::Installing;
                    self.ui_requests.push(UiRequest::RestartForUpdate);
                }
                Event::Failed(error) => self.updates.fail(error),
            }
        }
        if changed {
            self.revision += 1;
        }
        changed
    }
}

#[cfg(test)]
mod tests;
