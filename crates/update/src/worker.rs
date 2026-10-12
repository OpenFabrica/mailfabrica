//! 一个会话至多一个更新任务；结果带任务号，关闭后取消读取并丢弃事件。
use crate::{Client, Release, verify_bytes};
use std::io::Read;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

/// 网络、下载和系统验证任务；不写邮件存储或安装器文件。
pub enum Task {
    Check(String),
    Download(Release),
    Verify(Release, PathBuf),
}

/// 更新线程返回的可观察结果，任务号由 Worker 附加。
pub enum Event {
    Checked(Option<Release>),
    Manual(String),
    Progress(u64),
    Downloaded(Vec<u8>),
    Verified(PathBuf),
    Failed(String),
}

/// 更新任务运行器；客户端可替换以测试网络、完整性和签名失败。
pub struct Worker {
    client: Arc<dyn Client>,
    tx: mpsc::Sender<(u64, Event)>,
    rx: mpsc::Receiver<(u64, Event)>,
    stopped: Arc<AtomicBool>,
}

impl Worker {
    /// 默认客户端由引擎注入；此处不隐式启动网络请求。
    pub fn new(client: Arc<dyn Client>) -> Self {
        let (tx, rx) = mpsc::channel();
        Self { client, tx, rx, stopped: Arc::new(AtomicBool::new(false)) }
    }

    /// 开启一个限时任务；线程创建失败直接返回错误，不留下永久忙碌状态。
    pub fn start(&self, id: u64, task: Task) -> Result<(), String> {
        let (client, tx, stopped) = (self.client.clone(), self.tx.clone(), self.stopped.clone());
        std::thread::Builder::new()
            .name("mail-update".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match task {
                    Task::Check(current) => match client.manual_url() {
                        Some(url) => Ok(Event::Manual(url.into())),
                        None => client.check(&current).map(Event::Checked),
                    },
                    Task::Download(release) => {
                        let bytes = client
                            .download(&release, &mut |count| !stopped.load(Ordering::Relaxed) && tx.send((id, Event::Progress(count))).is_ok())?;
                        verify_bytes(&release, &bytes)?;
                        Ok(Event::Downloaded(bytes))
                    }
                    Task::Verify(release, path) => {
                        if release.size == 0 || release.size > crate::MAX_INSTALLER_BYTES {
                            return Err("Invalid installer size".into());
                        }
                        let mut bytes = Vec::new();
                        let file = std::fs::File::open(&path).map_err(|_| "Could not read the saved installer")?;
                        file.take(release.size.saturating_add(1)).read_to_end(&mut bytes).map_err(|_| "Could not read the saved installer")?;
                        verify_bytes(&release, &bytes)?;
                        client.verify_publisher(&path)?;
                        Ok(Event::Verified(path))
                    }
                }))
                .unwrap_or_else(|_| Err("Update task failed unexpectedly".into()));
                if !stopped.load(Ordering::Relaxed) {
                    let event = result.unwrap_or_else(Event::Failed);
                    let _ = tx.send((id, event));
                }
            })
            .map(|_| ())
            .map_err(|_| "Could not start the update task".into())
    }

    /// 取走已经到达的事件；调用方按当前任务号过滤旧结果。
    pub fn poll(&self) -> Vec<(u64, Event)> {
        self.rx.try_iter().collect()
    }
}

impl Drop for Worker {
    /// 关闭会话不等待网络，也不再自动打开安装器。
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
    }
}
