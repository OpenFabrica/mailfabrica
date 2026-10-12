//! 更新基础设施（L3）：固定 GitHub 发布源、限时下载、文件完整性和 Apple 发布者验证。
//! 不依赖界面或邮件存储；引擎保存安装包，后台验证与准备，独立辅助进程在正常退出后替换应用。
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod archive;
mod github;
mod install;
mod publisher;
mod transaction;
mod worker;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use github::GitHub;
pub use install::{Handoff, InstallTarget, acknowledge, prepare, run_helper};
pub use worker::{Event, Task, Worker};

pub const REPOSITORY: &str = "https://github.com/OpenFabrica/mailfabrica";
pub const MAX_INSTALLER_BYTES: u64 = 128 << 20;

/// 正式版本及本机安装包；仅由经过校验的发布元数据构造。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    pub version: String,
    pub notes: String,
    pub page: String,
    pub asset: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

/// 可替换的发布基础设施；下载进度回调返回 false 时必须取消读取。
pub trait Client: Send + Sync {
    /// 没有可验证安装器的平台返回手动更新页；测试客户端默认走安装器流程。
    fn manual_url(&self) -> Option<&str> {
        None
    }
    /// 返回比 current 更新的正式版；未发布或当前已最新时返回 None。
    fn check(&self, current: &str) -> Result<Option<Release>, String>;
    /// 下载指定安装器；调用方再检查长度和哈希，失败时不保存或打开。
    fn download(&self, release: &Release, progress: &mut dyn FnMut(u64) -> bool) -> Result<Vec<u8>, String>;
    /// 检查已保存安装器的系统信任和发布者，禁止仅凭哈希打开文件。
    fn verify_publisher(&self, path: &std::path::Path) -> Result<(), String>;
}

/// 防止截断、额外内容和哈希不符的文件进入安装流程。
pub fn verify_bytes(release: &Release, bytes: &[u8]) -> Result<(), String> {
    if release.size == 0 || release.size > MAX_INSTALLER_BYTES || bytes.len() as u64 != release.size {
        return Err("Installer size does not match the release".into());
    }
    let hash = format!("{:x}", Sha256::digest(bytes));
    if hash != release.sha256 {
        return Err("Installer SHA-256 verification failed".into());
    }
    Ok(())
}
