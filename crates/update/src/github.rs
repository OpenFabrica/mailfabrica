//! 只从固定仓库选择本机架构的稳定版；GitHub digest 是必需字段。
use crate::{Client, MAX_INSTALLER_BYTES, REPOSITORY, Release};
use semver::Version;
use serde::Deserialize;
use std::io::Read;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 官方 GitHub 发布源；不接收自定义地址、认证令牌或任意资产 URL。
#[derive(Default)]
pub struct GitHub;

/// GitHub 的最新发布响应，缺少必需字段时拒绝更新。
#[derive(Deserialize)]
struct Metadata {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    html_url: String,
    #[serde(default)]
    body: Option<String>,
    assets: Vec<Asset>,
}

/// GitHub 安装包资产及其服务端 SHA-256 摘要。
#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    size: u64,
    digest: Option<String>,
}

/// 本机分发架构；其他平台没有 DMG 更新入口。
fn architecture() -> Result<&'static str, String> {
    if !cfg!(target_os = "macos") {
        return Err("In-app installer updates are available on macOS".into());
    }
    match std::env::consts::ARCH {
        "aarch64" => Ok("arm64"),
        "x86_64" => Ok("x86_64"),
        _ => Err("No installer is available for this architecture".into()),
    }
}

/// 所有请求均使用 HTTPS，有限重定向、读取和连接时限，不发送用户邮件数据。
fn agent(timeout: Duration) -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder().unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::aws_lc_rs::default_provider())).build();
    ureq::Agent::config_builder()
        .tls_config(tls)
        .https_only(true)
        .max_redirects(5)
        .timeout_global(Some(timeout))
        .timeout_connect(Some(Duration::from_secs(10)))
        .user_agent("MailFabrica-updater")
        .build()
        .into()
}

/// 解析版本并选择严格匹配仓库、版本和架构的唯一安装包。
fn select(metadata: Metadata, current: &str, arch: &str) -> Result<Option<Release>, String> {
    if metadata.draft || metadata.prerelease {
        return Ok(None);
    }
    let version = metadata.tag_name.strip_prefix('v').ok_or("Invalid release tag")?;
    let published = Version::parse(version).map_err(|_| "Invalid release version")?;
    let running = Version::parse(current).map_err(|_| "Invalid application version")?;
    if !published.pre.is_empty() || published <= running {
        return Ok(None);
    }
    if metadata.html_url != format!("{REPOSITORY}/releases/tag/v{version}") {
        return Err("Unexpected release repository".into());
    }
    let name = format!("MailFabrica-{version}-{arch}.dmg");
    let matches: Vec<_> = metadata.assets.into_iter().filter(|a| a.name == name).collect();
    if matches.len() != 1 {
        return Err("No matching installer is available for this Mac".into());
    }
    let asset = matches.into_iter().next().ok_or("No matching installer")?;
    if asset.browser_download_url != format!("{REPOSITORY}/releases/download/v{version}/{name}") {
        return Err("Unexpected installer source".into());
    }
    if asset.size == 0 || asset.size > MAX_INSTALLER_BYTES {
        return Err("Installer exceeds the allowed size".into());
    }
    let sha256 = asset
        .digest
        .as_deref()
        .and_then(|d| d.strip_prefix("sha256:"))
        .filter(|d| d.len() == 64 && d.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or("Release is missing a valid SHA-256 digest")?
        .to_ascii_lowercase();
    Ok(Some(Release {
        version: version.into(),
        notes: metadata.body.unwrap_or_default().chars().take(20_000).collect(),
        page: metadata.html_url,
        asset: asset.name,
        url: asset.browser_download_url,
        size: asset.size,
        sha256,
    }))
}

impl Client for GitHub {
    /// Windows/Linux 使用 portable 包，显式引导到官方页面，不绕过 macOS 发布者校验。
    fn manual_url(&self) -> Option<&str> {
        (!cfg!(target_os = "macos")).then_some("https://github.com/OpenFabrica/mailfabrica/releases/latest")
    }
    /// 404 表示尚无正式发布；其他网络或元数据错误必须让用户知道，可重试。
    fn check(&self, current: &str) -> Result<Option<Release>, String> {
        let arch = architecture()?;
        let response = match agent(Duration::from_secs(20))
            .get("https://api.github.com/repos/OpenFabrica/mailfabrica/releases/latest")
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .call()
        {
            Ok(response) => response,
            Err(ureq::Error::StatusCode(404)) => return Ok(None),
            Err(_) => return Err("Could not check GitHub releases; check your network and retry".into()),
        };
        let mut bytes = Vec::new();
        response.into_body().into_reader().take((2 << 20) + 1).read_to_end(&mut bytes).map_err(|_| "Could not read release metadata")?;
        if bytes.len() > 2 << 20 {
            return Err("Release metadata is too large".into());
        }
        let metadata = serde_json::from_slice(&bytes).map_err(|_| "Invalid release metadata")?;
        select(metadata, current, arch)
    }

    /// 流式读取有限大小的安装器，每 100ms 上报一次；不输出 CDN 的临时令牌。
    fn download(&self, release: &Release, progress: &mut dyn FnMut(u64) -> bool) -> Result<Vec<u8>, String> {
        let expected = format!("{REPOSITORY}/releases/download/v{}/{}", release.version, release.asset);
        if release.url != expected || release.size == 0 || release.size > MAX_INSTALLER_BYTES {
            return Err("Invalid installer metadata".into());
        }
        let response =
            agent(Duration::from_secs(180)).get(&release.url).call().map_err(|_| "Could not download the installer; check your network and retry")?;
        read_installer(response.into_body().into_reader(), release.size, progress)
    }

    /// 只接受通过 Gatekeeper 的指定 Apple Team 发布者。
    fn verify_publisher(&self, path: &std::path::Path) -> Result<(), String> {
        crate::publisher::verify(path)
    }
}

/// 限制长度并保留实际进度；提前结束和额外字节都由最终完整性检查拒绝。
fn read_installer(mut reader: impl Read, size: u64, progress: &mut dyn FnMut(u64) -> bool) -> Result<Vec<u8>, String> {
    let mut data = Vec::new();
    let mut buffer = [0_u8; 64 << 10];
    let mut last = Instant::now();
    if !progress(0) {
        return Err("Download cancelled".into());
    }
    loop {
        let count = reader.read(&mut buffer).map_err(|_| "Installer download was interrupted")?;
        if count == 0 {
            break;
        }
        if data.len() as u64 + count as u64 > size || size > MAX_INSTALLER_BYTES {
            return Err("Installer exceeds its declared size".into());
        }
        data.extend_from_slice(buffer.get(..count).ok_or("Invalid download buffer")?);
        if last.elapsed() >= Duration::from_millis(100) {
            if !progress(data.len() as u64) {
                return Err("Download cancelled".into());
            }
            last = Instant::now();
        }
    }
    if !progress(data.len() as u64) {
        return Err("Download cancelled".into());
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    /// 构造一个正式发布，字段和生产 API 一致。
    fn fixture(version: &str) -> Metadata {
        serde_json::from_value(serde_json::json!({"tag_name":format!("v{version}"),"draft":false,"prerelease":false,
            "html_url":format!("{REPOSITORY}/releases/tag/v{version}"),"body":"Release notes","assets":[{
            "name":format!("MailFabrica-{version}-arm64.dmg"),"browser_download_url":format!("{REPOSITORY}/releases/download/v{version}/MailFabrica-{version}-arm64.dmg"),
            "size":3,"digest":format!("sha256:{:x}",Sha256::digest(b"dmg"))}]})).expect("fixture")
    }

    /// 按 semver 比较，不降级、不把预发布当稳定版、不混用 Intel 安装器。
    #[test]
    fn stable_version_and_native_asset_selection() {
        assert!(select(fixture("0.10.0"), "0.9.0", "arm64").expect("newer").is_some());
        assert!(select(fixture("0.1.0"), "0.1.0", "arm64").expect("equal").is_none());
        assert!(select(fixture("0.1.0"), "0.2.0-dev", "arm64").expect("older").is_none());
        assert!(select(fixture("0.1.0"), "0.1.0-dev", "arm64").expect("dev").is_some());
        assert!(select(fixture("0.2.0-rc.1"), "0.1.0", "arm64").expect("rc").is_none());
        assert!(select(fixture("0.2.0"), "0.1.0", "x86_64").is_err());
        let mut draft = fixture("0.2.0");
        draft.draft = true;
        assert!(select(draft, "0.1.0", "arm64").expect("draft").is_none());
    }

    /// 元数据缺摘要、文件过大或来源错误时，不能启动安装。
    #[test]
    fn rejects_untrusted_metadata_and_corrupted_downloads() {
        for change in 0..4 {
            let mut meta = fixture("0.2.0");
            match change {
                0 => meta.assets[0].digest = None,
                1 => meta.assets[0].size = MAX_INSTALLER_BYTES + 1,
                2 => meta.assets[0].browser_download_url = "https://example.com/file.dmg".into(),
                _ => meta.html_url = "https://github.com/other/repo".into(),
            }
            assert!(select(meta, "0.1.0", "arm64").is_err());
        }
        let release = select(fixture("0.2.0"), "0.1.0", "arm64").expect("select").expect("release");
        assert!(crate::verify_bytes(&release, b"dmg").is_ok());
        for bytes in [b"bad".as_slice(), b"dm", b"dmgg"] {
            assert!(crate::verify_bytes(&release, bytes).is_err());
        }
        assert!(read_installer(b"dmgg".as_slice(), 3, &mut |_| true).is_err());
        assert!(read_installer(b"dmg".as_slice(), 3, &mut |_| false).is_err());
    }
}
