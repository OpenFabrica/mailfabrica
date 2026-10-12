//! Portable 归档仅接受发布目录中的已知普通文件，禁止链接、路径穿越和解压膨胀。
use crate::Release;
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, io::Read, path::Path};

pub(crate) const FILES: &[&str] = &[
    "mailfabrica",
    "mailfabrica-cli",
    "mailfabrica.exe",
    "mailfabrica-cli.exe",
    "build.json",
    "README.txt",
    "LICENSE-MIT",
    "LICENSE-APACHE",
    "MailFabrica-logo.png",
    "mailfabrica.desktop",
];
const MAX_EXPANDED: u64 = 256 << 20;

/// 规范化 Windows 的反斜杠，要求准确的顶层目录和单层白名单文件。
fn filename(raw: &str, root: &str) -> Result<Option<String>, String> {
    let normalized = raw.replace('\\', "/");
    if normalized.trim_end_matches('/') == root {
        return Ok(None);
    }
    let name = normalized.strip_prefix(&format!("{root}/")).ok_or("Unexpected update archive root")?;
    if !FILES.contains(&name) {
        return Err("Update archive contains an unexpected path".into());
    }
    Ok(Some(name.into()))
}

/// 逐文件有限读取并独占创建；重复条目不能覆盖先前验证的文件。
fn save(reader: &mut impl Read, path: &Path, declared: u64, total: &mut u64) -> Result<(), String> {
    if declared > MAX_EXPANDED || total.saturating_add(declared) > MAX_EXPANDED {
        return Err("Expanded update exceeds 256 MiB".into());
    }
    let mut file = fs::File::create_new(path).map_err(|_| "Duplicate or invalid update file")?;
    let size = std::io::copy(&mut reader.take(declared + 1), &mut file).map_err(|_| "Could not extract update file")?;
    if size != declared {
        return Err("Update archive file has an invalid size".into());
    }
    *total += size;
    file.sync_all().map_err(|_| "Could not save update file".into())
}

/// 安全解压 ZIP/TAR.GZ，并核对归档内构建记录的版本、平台、架构及二进制哈希。
pub(crate) fn extract(path: &Path, release: &Release, destination: &Path) -> Result<(), String> {
    let root = release.asset.strip_suffix(".zip").or_else(|| release.asset.strip_suffix(".tar.gz")).ok_or("Unknown update archive")?;
    let file = fs::File::open(path).map_err(|_| "Could not open update archive")?;
    let mut total = 0;
    let mut names = BTreeSet::new();
    if release.asset.ends_with(".zip") {
        let mut archive = zip::ZipArchive::new(file).map_err(|_| "Invalid ZIP update")?;
        if archive.len() > 32 {
            return Err("Too many update archive entries".into());
        }
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).map_err(|_| "Invalid ZIP entry")?;
            if entry.is_symlink() || entry.unix_mode().is_some_and(|m| m & 0o170000 != 0 && m & 0o170000 != 0o100000 && m & 0o170000 != 0o040000) {
                return Err("Update archive links are forbidden".into());
            }
            let name = filename(entry.name(), root)?;
            match name {
                None if entry.is_dir() => continue,
                Some(name) if !entry.is_dir() => {
                    let size = entry.size();
                    save(&mut entry, &destination.join(&name), size, &mut total)?;
                    names.insert(name);
                }
                _ => return Err("Invalid update archive directory".into()),
            }
        }
    } else {
        let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
        for (index, entry) in archive.entries().map_err(|_| "Invalid TAR update")?.enumerate() {
            if index >= 32 {
                return Err("Too many update archive entries".into());
            }
            let mut entry = entry.map_err(|_| "Invalid TAR entry")?;
            let raw = entry.path().map_err(|_| "Invalid TAR path")?.to_string_lossy().into_owned();
            let kind = entry.header().entry_type();
            let name = filename(&raw, root)?;
            match name {
                None if kind.is_dir() => continue,
                Some(name) if kind.is_file() => {
                    let size = entry.size();
                    save(&mut entry, &destination.join(&name), size, &mut total)?;
                    names.insert(name);
                }
                _ => return Err("Update archive links and special files are forbidden".into()),
            }
        }
    }
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(destination.join("build.json")).map_err(|_| "Missing build record")?).map_err(|_| "Invalid build record")?;
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        _ => "unsupported",
    };
    if manifest["version"] != release.version
        || manifest["platform"] != std::env::consts::OS
        || manifest["arch"] != arch
        || manifest["dirty"] != false
    {
        return Err("Update build record does not match this platform and release".into());
    }
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    for binary in [format!("mailfabrica{suffix}"), format!("mailfabrica-cli{suffix}")] {
        let path = destination.join(&binary);
        let hash = hash_file(&path)?;
        if manifest["sha256"][&binary].as_str() != Some(&hash) {
            return Err("Update binary checksum mismatch".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).map_err(|_| "Could not set executable permissions")?;
        }
    }
    Ok(())
}

/// 有限流式哈希用于阶段间再次验证，避免重新分配整个二进制。
pub(crate) fn hash_file(path: &Path) -> Result<String, String> {
    let file = fs::File::open(path).map_err(|_| "Could not read update binary")?;
    let mut reader = file.take(MAX_EXPANDED + 1);
    let mut hash = Sha256::new();
    let mut count = 0_u64;
    let mut buffer = [0_u8; 64 << 10];
    loop {
        let n = reader.read(&mut buffer).map_err(|_| "Could not read update binary")?;
        if n == 0 {
            break;
        }
        count += n as u64;
        if count > MAX_EXPANDED {
            return Err("Update binary exceeds 256 MiB".into());
        }
        hash.update(buffer.get(..n).ok_or("Invalid checksum buffer")?);
    }
    Ok(format!("{:x}", hash.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    /// 跨平台路径写法可接受；所有越界目录、替代根和未知文件都拒绝。
    #[test]
    fn paths_are_flat_and_allowlisted() {
        assert_eq!(filename("package\\mailfabrica.exe", "package").expect("Windows"), Some("mailfabrica.exe".into()));
        for path in [
            "/package/mailfabrica",
            "package/../mailfabrica",
            "package/sub/mailfabrica",
            "other/mailfabrica",
            "package/accounts.json",
            "package/C:/mailfabrica",
        ] {
            assert!(filename(path, "package").is_err(), "{path}");
        }
    }
    /// 重复条目和宣称超大内容不能覆盖缓存或导致无界写入。
    #[test]
    fn extraction_is_bounded_and_exclusive() {
        let tmp = tempfile::tempdir().expect("tmp");
        let path = tmp.path().join("file");
        save(&mut b"ok".as_slice(), &path, 2, &mut 0).expect("save");
        assert!(save(&mut b"bad".as_slice(), &path, 3, &mut 0).is_err());
        assert!(save(&mut b"bad".as_slice(), &tmp.path().join("huge"), MAX_EXPANDED + 1, &mut 0).is_err());
        assert_eq!(fs::read(path).expect("original"), b"ok");
    }
}
