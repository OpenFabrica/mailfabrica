//! 同一文件系统内备份与替换；只移动计划中的应用文件，失败时恢复已完成的步骤。
use std::{
    fs,
    path::{Path, PathBuf},
};

/// 事务条目使用叶子名称，目标目录以外的用户文件永不进入替换计划。
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Entry {
    pub name: String,
    pub old_hash: Option<String>,
    pub new_hash: String,
}

/// 替换前逐项检查类型、旧内容和新内容，防止并发更新或缓存篡改。
pub(crate) fn validate(root: &Path, stage: &Path, entries: &[Entry], bundle: bool) -> Result<(), String> {
    if entries.is_empty() || entries.len() > 10 {
        return Err("Invalid replacement plan".into());
    }
    let mut unique = std::collections::BTreeSet::new();
    for entry in entries {
        if !(crate::archive::FILES.contains(&entry.name.as_str())
            || bundle && entry.name.ends_with(".app") && Path::new(&entry.name).components().count() == 1)
            || !unique.insert(&entry.name)
        {
            return Err("Invalid replacement filename".into());
        }
        let dest = root.join(&entry.name);
        if let Ok(meta) = fs::symlink_metadata(&dest)
            && (meta.file_type().is_symlink() || if bundle { !meta.is_dir() } else { !meta.is_file() })
        {
            return Err("Application target is not a regular installation".into());
        }
        let binary = |p: PathBuf| if bundle { p.join("Contents/MacOS/mailfabrica") } else { p };
        if let Some(hash) = &entry.old_hash {
            if crate::archive::hash_file(&binary(dest))? != *hash {
                return Err("Application changed since the update started".into());
            }
        } else if dest.exists() {
            return Err("Application target appeared during the update".into());
        }
        if crate::archive::hash_file(&binary(stage.join("payload").join(&entry.name)))? != entry.new_hash {
            return Err("Prepared update checksum changed".into());
        }
    }
    Ok(())
}

/// 单项 rename 失败时恢复所有先前移动；保留恢复失败的备份并报告路径。
pub(crate) fn apply(root: &Path, stage: &Path, entries: &[Entry]) -> Result<(), String> {
    fs::create_dir(stage.join("backup")).map_err(|_| "Could not create application backup")?;
    for entry in entries {
        let target = root.join(&entry.name);
        if target.exists() && fs::rename(&target, stage.join("backup").join(&entry.name)).is_err() {
            rollback(root, stage, entries)?;
            return Err("Could not replace the application; close other running copies and retry".into());
        }
        if fs::rename(stage.join("payload").join(&entry.name), &target).is_err() {
            rollback(root, stage, entries)?;
            return Err("Could not install the prepared application; the previous version was restored".into());
        }
    }
    Ok(())
}

/// 将新文件移回准备目录后恢复旧文件；没有备份的未处理条目保持原样。
pub(crate) fn rollback(root: &Path, stage: &Path, entries: &[Entry]) -> Result<(), String> {
    for entry in entries.iter().rev() {
        let target = root.join(&entry.name);
        let backup = stage.join("backup").join(&entry.name);
        let payload = stage.join("payload").join(&entry.name);
        if backup.exists() || entry.old_hash.is_none() && !payload.exists() {
            if target.exists() {
                fs::rename(&target, &payload).map_err(|_| format!("Could not restore application; backup retained at {}", backup.display()))?;
            }
            if backup.exists() {
                fs::rename(&backup, &target).map_err(|_| format!("Could not restore application; backup retained at {}", backup.display()))?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    /// 实际第二项失败会恢复第一项，账户数据和未知文件不会被移动或覆盖。
    #[test]
    fn partial_failure_restores_previous_files_and_preserves_data() {
        let tmp = tempfile::tempdir().expect("tmp");
        let root = tmp.path().join("installed");
        let stage = tmp.path().join("stage");
        fs::create_dir_all(&root).expect("root");
        fs::create_dir_all(stage.join("payload")).expect("payload");
        for (name, bytes) in [("mailfabrica", b"old".as_slice()), ("mailfabrica-cli", b"cli"), ("accounts.json", b"data")] {
            fs::write(root.join(name), bytes).expect("file");
        }
        fs::write(stage.join("payload/mailfabrica"), b"new").expect("new");
        let entries =
            ["mailfabrica", "mailfabrica-cli"].map(|name| Entry { name: name.into(), old_hash: Some("old".into()), new_hash: "new".into() });
        assert!(apply(&root, &stage, &entries).is_err());
        assert_eq!(fs::read(root.join("mailfabrica")).expect("restored"), b"old");
        assert_eq!(fs::read(root.join("mailfabrica-cli")).expect("cli restored"), b"cli");
        assert_eq!(fs::read(root.join("accounts.json")).expect("data"), b"data");
    }
}
