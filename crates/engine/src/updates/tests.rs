use super::*;
use crate::{FakeConnector, core::MemorySecrets};
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// 固定结果的发布源，可切换下载和发布者错误，不访问网络或系统信任服务。
struct Fake {
    release: Option<Release>,
    corrupt: AtomicBool,
    reject: AtomicBool,
    offline: bool,
}

impl Client for Fake {
    /// 模拟慢请求，让重复操作门禁有确定的验证窗口。
    fn check(&self, _: &str) -> std::result::Result<Option<Release>, String> {
        std::thread::sleep(Duration::from_millis(30));
        if self.offline { Err("Network unavailable".into()) } else { Ok(self.release.clone()) }
    }
    /// 进度与实际字节都来自下载结果；损坏文件不能进入保存阶段。
    fn download(&self, _: &Release, progress: &mut dyn FnMut(u64) -> bool) -> std::result::Result<Vec<u8>, String> {
        if !progress(3) {
            return Err("Cancelled".into());
        }
        Ok(if self.corrupt.load(Ordering::Relaxed) { b"bad" } else { b"dmg" }.to_vec())
    }
    /// 模拟可信和其他发布者两条系统验证路径。
    fn verify_publisher(&self, _: &std::path::Path) -> std::result::Result<(), String> {
        if self.reject.load(Ordering::Relaxed) { Err("Wrong publisher".into()) } else { Ok(()) }
    }
}

/// 构造可用版本；摘要对应三字节安装器测试数据。
fn source(release: bool) -> Arc<Fake> {
    Arc::new(Fake {
        release: release.then(|| Release {
            version: "9.0.0".into(),
            notes: "修复与改进".into(),
            page: "https://github.com/OpenFabrica/mailfabrica/releases/tag/v9.0.0".into(),
            asset: "MailFabrica-9.0.0-arm64.dmg".into(),
            url: "https://github.com/OpenFabrica/mailfabrica/releases/download/v9.0.0/MailFabrica-9.0.0-arm64.dmg".into(),
            size: 3,
            sha256: "00cbbd0ddbda2762798f7009838ed34ca1f12b93965813c7df22943bc62166d1".into(),
        }),
        corrupt: AtomicBool::new(false),
        reject: AtomicBool::new(false),
        offline: false,
    })
}

/// 独立存储、零邮箱，确保更新不依赖账户存在。
fn session(source: Arc<dyn Client>) -> Session {
    let tmp = tempfile::tempdir().expect("temp");
    let mut s = Session::open(tmp.path().into(), Arc::new(MemorySecrets::new()), Arc::new(FakeConnector::demo("demo@mailfabrica.dev", "demo")))
        .expect("session");
    s.prefs.sync_interval_minutes = 0;
    s._tmp = Some(tmp);
    s.updates = Updates::new(source);
    s
}

/// 完整命令路径：检查、进度、保存、验证、打开；重复触发禁用，缓存被篡改时重新打开失败。
#[test]
fn commands_download_verify_open_and_revalidate() {
    let mut s = session(source(true));
    assert!(s.run("update.download", json!({})).is_err());
    assert!(s.run("update.check", json!({"url":"https://example.com"})).is_err());
    s.run("update.check", json!({})).expect("check");
    assert!(s.update().busy());
    assert!(s.run("update.check", json!({})).is_err());
    assert_eq!(s.run("app.status", json!({})).expect("status")["busy"], true);
    s.wait_idle(Duration::from_secs(2));
    assert_eq!(s.update().phase, UpdatePhase::Available);
    s.run("update.download", json!({})).expect("download");
    assert!(s.run("update.download", json!({})).is_err());
    s.wait_idle(Duration::from_secs(2));
    assert_eq!(s.update().phase, UpdatePhase::Ready);
    assert_eq!(s.update().received, 3);
    let path = s.update().installer.clone().expect("path");
    assert_eq!(std::fs::read(&path).expect("cached"), b"dmg");
    assert!(s.take_ui_requests().is_empty());
    assert_eq!(s.run("update.status", json!({})).expect("query")["phase"], "ready");
    std::fs::write(&path, b"bad").expect("tamper");
    s.run("update.open", json!({})).expect("reverify");
    s.wait_idle(Duration::from_secs(2));
    assert_eq!(s.update().phase, UpdatePhase::Failed);
    assert!(s.take_ui_requests().is_empty());
    assert!(s.run("update.open", json!({})).is_err());
}

/// 哈希不符和错误发布者都不可打开；修复网络/来源后无需重启会话即可重试。
#[test]
fn failures_never_open_and_download_can_retry() {
    let fake = source(true);
    fake.corrupt.store(true, Ordering::Relaxed);
    let mut s = session(fake.clone());
    s.run("update.check", json!({})).expect("check");
    s.wait_idle(Duration::from_secs(2));
    s.run("update.download", json!({})).expect("download");
    s.wait_idle(Duration::from_secs(2));
    assert_eq!(s.update().phase, UpdatePhase::Failed);
    assert!(!s.store.root().join("updates").exists());
    assert!(s.take_ui_requests().is_empty());
    fake.corrupt.store(false, Ordering::Relaxed);
    fake.reject.store(true, Ordering::Relaxed);
    s.run("update.download", json!({})).expect("retry");
    s.wait_idle(Duration::from_secs(2));
    assert_eq!(s.update().phase, UpdatePhase::Failed);
    assert!(s.update().installer.is_none());
    assert!(s.take_ui_requests().is_empty());
    fake.reject.store(false, Ordering::Relaxed);
    s.run("update.download", json!({})).expect("retry trusted");
    s.wait_idle(Duration::from_secs(2));
    assert_eq!(s.update().phase, UpdatePhase::Ready);
    assert!(s.take_ui_requests().is_empty());
}

/// 无发布/已最新与网络失败分开显示；旧任务的迟到结果不能覆盖新任务。
#[test]
fn current_failure_and_stale_results_are_distinct() {
    let mut s = session(source(false));
    s.run("update.check", json!({})).expect("check");
    s.wait_idle(Duration::from_secs(2));
    assert_eq!(s.update().phase, UpdatePhase::Current);
    let fake = Fake { release: None, corrupt: AtomicBool::new(false), reject: AtomicBool::new(false), offline: true };
    s.updates = Updates::new(Arc::new(fake));
    s.run("update.check", json!({})).expect("offline");
    s.wait_idle(Duration::from_secs(2));
    assert_eq!(s.update().phase, UpdatePhase::Failed);
    assert!(!s.update().busy());
    s.updates = Updates::new(source(true));
    s.run("update.check", json!({})).expect("late");
    s.updates.active = Some(99);
    std::thread::sleep(Duration::from_millis(80));
    assert!(!s.pump_updates());
    assert_eq!(s.update().phase, UpdatePhase::Checking);
    assert!(s.update().release.is_none());
}

/// 手动更新平台只提供官方页面，不错误地进入安装器下载/Apple 签名检查。
#[test]
fn manual_platform_update_returns_page_and_disables_download() {
    struct Manual;
    impl Client for Manual {
        /// 指定可返回给 GUI/CLI/MCP 的官方页面。
        fn manual_url(&self) -> Option<&str> {
            Some("https://github.com/OpenFabrica/mailfabrica/releases/latest")
        }
        /// 手动流程不应调用网络发布检查。
        fn check(&self, _: &str) -> std::result::Result<Option<Release>, String> {
            Err("Unexpected network check".into())
        }
        /// 手动流程没有可下载安装器。
        fn download(&self, _: &Release, _: &mut dyn FnMut(u64) -> bool) -> std::result::Result<Vec<u8>, String> {
            Err("No installer".into())
        }
        /// 手动流程不会验证任意本地文件。
        fn verify_publisher(&self, _: &std::path::Path) -> std::result::Result<(), String> {
            Err("No installer".into())
        }
    }
    let mut s = session(Arc::new(Manual));
    s.run("update.check", json!({})).expect("check");
    s.wait_idle(Duration::from_secs(2));
    assert_eq!(s.update().phase, UpdatePhase::Manual);
    assert_eq!(s.take_ui_requests(), vec![UiRequest::OpenUrl("https://github.com/OpenFabrica/mailfabrica/releases/latest".into())]);
    assert!(s.run("update.download", json!({})).is_err());
    assert_eq!(s.run("update.status", json!({})).expect("status")["manualUrl"], "https://github.com/OpenFabrica/mailfabrica/releases/latest");
}

/// 写信尚未保存时，更新不能触发退出；下载期间又写信也必须保留内容。
#[test]
fn update_preserves_unsaved_composes() {
    let mut s = session(source(true));
    s.run("update.check", json!({})).expect("check");
    s.wait_idle(Duration::from_secs(2));
    s.composes.push(crate::Compose { id: 99, dirty: true, ..Default::default() });
    assert!(s.run("update.download", json!({})).is_err());
    assert!(s.take_ui_requests().iter().all(|r| *r != UiRequest::RestartForUpdate));
}

/// 取消准备立即解除忙碌并使旧任务失效，不允许迟到结果再次发起退出。
#[test]
fn cancelling_preparation_invalidates_the_active_task() {
    let mut s = session(source(true));
    s.updates.state.phase = UpdatePhase::Installing;
    s.updates.active = Some(42);
    s.run("update.cancel", json!({})).expect("cancel");
    assert_eq!(s.update().phase, UpdatePhase::Cancelled);
    assert!(!s.update().busy());
    assert!(s.updates.active.is_none() && s.updates.handoff.is_none());
    assert!(s.take_ui_requests().is_empty());
    assert!(s.run("update.cancel", json!({})).is_err());
}
