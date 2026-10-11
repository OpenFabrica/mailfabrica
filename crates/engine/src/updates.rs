//! 更新领域状态：显式检查、下载、保存、验证、打开安装器；不替换运行中的应用。
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
    Available,
    Downloading,
    Verifying,
    Ready,
    Failed,
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
}

impl UpdateState {
    /// 忙碌时禁止重复启动任务；独立于邮件同步状态。
    pub fn busy(&self) -> bool {
        matches!(self.phase, UpdatePhase::Checking | UpdatePhase::Downloading | UpdatePhase::Verifying)
    }
}

/// 会话私有更新运行状态，任务号阻止迟到结果覆盖后续请求。
pub(crate) struct Updates {
    worker: Worker,
    state: UpdateState,
    active: Option<u64>,
    next: u64,
}

impl Updates {
    /// 不隐式检查更新；用户触发命令后才联网。
    pub(crate) fn new(client: Arc<dyn Client>) -> Self {
        Self { worker: Worker::new(client), state: UpdateState::default(), active: None, next: 0 }
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
    /// 当前更新状态，供界面与命令查询，不产生网络或磁盘副作用。
    pub fn update(&self) -> &UpdateState {
        &self.updates.state
    }

    /// 检查比正在运行版本更新的稳定版；先丢弃先前选择，避免复用过时元数据。
    pub(crate) fn check_update(&mut self) -> Result<()> {
        self.updates.state = UpdateState::default();
        let result = self.updates.start(Task::Check(env!("CARGO_PKG_VERSION").into()), UpdatePhase::Checking);
        self.revision += 1;
        result
    }

    /// 下载已经检查出的安装包；参数不允许传入任意 URL、版本或本地路径。
    pub(crate) fn download_update(&mut self) -> Result<()> {
        let release = self.update().release.clone().ok_or_else(|| EngineError::Other("Check for an update first".into()))?;
        self.updates.state.received = 0;
        self.updates.state.installer = None;
        let result = self.updates.start(Task::Download(release), UpdatePhase::Downloading);
        self.revision += 1;
        result
    }

    /// 再次打开安装器前重新检查哈希、签名和 Gatekeeper，防止缓存被篡改。
    pub(crate) fn reopen_installer(&mut self) -> Result<()> {
        let release = self.update().release.clone().ok_or_else(|| EngineError::Other("No downloaded installer".into()))?;
        let path = self.update().installer.clone().ok_or_else(|| EngineError::Other("No verified installer".into()))?;
        let result = self.updates.start(Task::Verify(release, path), UpdatePhase::Verifying);
        self.revision += 1;
        result
    }

    /// 引擎线程保存下载结果；签名检查移交后台，只有最终验证成功才提出打开请求。
    pub(crate) fn pump_updates(&mut self) -> bool {
        let events = self.updates.worker.poll();
        let mut changed = false;
        for (id, event) in events {
            if self.updates.active != Some(id) {
                continue;
            }
            changed = true;
            match event {
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
                    self.updates.active = None;
                    self.updates.state.phase = UpdatePhase::Ready;
                    self.updates.state.installer = Some(path.clone());
                    self.ui_requests.push(UiRequest::OpenInstaller(path));
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
