//! 后台生成线程：每个请求一条线程，结果带任务号送回引擎线程。
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

use crate::{Client, Config, Request};

/// AI 任务运行器；客户端可替换（测试用 `FakeClient`）。
pub struct Worker {
    client: Arc<dyn Client>,
    tx: mpsc::Sender<(u64, Result<String, String>)>,
    rx: mpsc::Receiver<(u64, Result<String, String>)>,
    stopped: Arc<AtomicBool>,
}

impl Worker {
    /// 建运行器，不联网。
    pub fn new(client: Arc<dyn Client>) -> Self {
        let (tx, rx) = mpsc::channel();
        Self { client, tx, rx, stopped: Arc::new(AtomicBool::new(false)) }
    }

    /// 在新线程里跑一次生成；线程起不来时直接返回错误。
    pub fn start(&self, id: u64, config: Config, key: String, request: Request) -> Result<(), String> {
        let (client, tx, stopped) = (self.client.clone(), self.tx.clone(), self.stopped.clone());
        std::thread::Builder::new()
            .name("mail-ai".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| client.complete(&config, &key, &request)))
                    .unwrap_or_else(|_| Err("The AI task failed unexpectedly".into()));
                if !stopped.load(Ordering::Relaxed) {
                    let _ = tx.send((id, result));
                }
            })
            .map(|_| ())
            .map_err(|_| "Could not start the AI task".into())
    }

    /// 取走已经完成的结果；调用方按任务号认领。
    pub fn poll(&self) -> Vec<(u64, Result<String, String>)> {
        self.rx.try_iter().collect()
    }

    /// 等到有结果或超时（CLI、测试用）。
    pub fn wait(&self, timeout: std::time::Duration) -> Vec<(u64, Result<String, String>)> {
        match self.rx.recv_timeout(timeout) {
            Ok(first) => std::iter::once(first).chain(self.rx.try_iter()).collect(),
            Err(_) => Vec::new(),
        }
    }
}

impl Drop for Worker {
    /// 关闭会话时不等网络，迟到的结果直接丢掉。
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
    }
}
