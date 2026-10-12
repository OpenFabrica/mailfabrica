//! 安装交接：后台准备同盘文件，独立辅助进程等待正常退出，再替换并验证新进程启动。
use crate::{
    Release, archive,
    transaction::{self, Entry},
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// 宿主注入实际 GUI 可执行文件和启动参数；不来自公开命令参数。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstallTarget {
    pub executable: PathBuf,
    pub args: Vec<String>,
    pub data: Option<PathBuf>,
}

/// 辅助进程的私有计划；所有目录都在安装目录的同盘私有子目录内。
#[derive(Serialize, Deserialize)]
struct Plan {
    target: InstallTarget,
    root: PathBuf,
    bundle: bool,
    entries: Vec<Entry>,
    version: String,
}

/// 持有父进程锁；只有宿主正常退出并写入提交标记后，辅助进程才允许替换。
pub struct Handoff {
    stage: PathBuf,
    _guard: fs::File,
}

impl Handoff {
    /// 返回私有准备目录，供宿主诊断和隔离安装验收读取交接结果。
    pub fn directory(&self) -> &Path {
        &self.stage
    }

    /// 正常退出的最后阶段提交交接；异常终止不会自动安装。
    pub fn commit(&self) -> Result<(), String> {
        fs::write(self.stage.join("commit"), b"apply").map_err(|_| "Could not commit the update restart".into())
    }
}

impl Drop for Handoff {
    /// 取消交接时等待辅助进程退出并清理准备文件；正常提交由新版首帧确认后清理。
    fn drop(&mut self) {
        if self.stage.join("commit").exists() {
            return;
        }
        let stage = self.stage.clone();
        let _ = std::thread::Builder::new().name("update-abort-cleanup".into()).spawn(move || {
            for _ in 0..60 {
                std::thread::sleep(Duration::from_secs(1));
                if stage.join("commit").exists() || !stage.exists() || fs::remove_dir_all(&stage).is_ok() {
                    break;
                }
            }
        });
    }
}

/// 从实际二进制推导替换边界；拒绝开发 target、符号链接和只读磁盘映像安装。
fn installation(target: &InstallTarget) -> Result<(PathBuf, bool, String), String> {
    let executable = fs::canonicalize(&target.executable).map_err(|_| "Could not locate the running application")?;
    if executable != target.executable {
        return Err("Application path must be canonical".into());
    }
    if cfg!(target_os = "macos") {
        let macos = executable.parent().ok_or("Invalid application bundle")?;
        let contents = macos.parent().ok_or("Invalid application bundle")?;
        let bundle = contents.parent().ok_or("Invalid application bundle")?;
        if macos.file_name().and_then(|s| s.to_str()) != Some("MacOS")
            || contents.file_name().and_then(|s| s.to_str()) != Some("Contents")
            || executable.file_name().and_then(|s| s.to_str()) != Some("mailfabrica")
            || bundle.extension().and_then(|s| s.to_str()) != Some("app")
        {
            return Err("Automatic updates require an installed MailFabrica.app bundle".into());
        }
        if executable.starts_with("/Volumes") {
            return Err("Move MailFabrica into Applications before updating".into());
        }
        let data = target.data.as_ref().map(fs::canonicalize).transpose().map_err(|_| "Could not resolve the mail data directory")?;
        if data.as_ref().is_some_and(|p| p.starts_with(bundle)) {
            return Err("Mail data must be stored outside the application bundle".into());
        }
        Ok((
            bundle.parent().ok_or("Missing application directory")?.into(),
            true,
            bundle.file_name().and_then(|s| s.to_str()).ok_or("Invalid bundle name")?.into(),
        ))
    } else {
        let name = if cfg!(windows) { "mailfabrica.exe" } else { "mailfabrica" };
        if executable.file_name().and_then(|s| s.to_str()) != Some(name) {
            return Err("Unexpected application executable".into());
        }
        let root = executable.parent().ok_or("Missing application directory")?;
        if root.ends_with("target/debug") || root.ends_with("target/release") {
            return Err("Run an installed release package to update".into());
        }
        Ok((root.into(), false, name.into()))
    }
}

/// 将已验证安装包重新校验后准备到同盘私有目录；失败不退出旧应用、不改变目标文件。
pub fn prepare(release: &Release, package: &Path, target: InstallTarget) -> Result<Handoff, String> {
    let (root, bundle, name) = installation(&target)?;
    let mut bytes = Vec::new();
    fs::File::open(package)
        .map_err(|_| "Could not read downloaded update")?
        .take(crate::MAX_INSTALLER_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read downloaded update")?;
    crate::verify_bytes(release, &bytes)?;
    crate::publisher::verify(package)?;
    let temporary = tempfile::Builder::new()
        .prefix(".mailfabrica-update-")
        .tempdir_in(&root)
        .map_err(|_| "Application directory is not writable; install MailFabrica in a user-writable location")?;
    let stage = temporary.path();
    fs::create_dir(stage.join("payload")).map_err(|_| "Could not prepare update directory")?;
    let immutable = stage.join(&release.asset);
    fs::write(&immutable, &bytes).map_err(|_| "Could not save private update package")?;
    if bundle {
        prepare_bundle(&immutable, &stage.join("payload").join(&name))?;
    } else {
        archive::extract(&immutable, release, &stage.join("payload"))?;
    }
    let mut entries = Vec::new();
    let binary = |p: PathBuf| if bundle { p.join("Contents/MacOS/mailfabrica") } else { p };
    let files: Vec<String> = if bundle {
        vec![name.clone()]
    } else {
        archive::FILES
            .iter()
            .filter(|n| stage.join("payload").join(n).is_file() && (**n == name || root.join(n).exists()))
            .map(|n| (*n).into())
            .collect()
    };
    for name in files {
        let existing = binary(root.join(&name));
        entries.push(Entry {
            name: name.clone(),
            old_hash: existing.exists().then(|| archive::hash_file(&existing)).transpose()?,
            new_hash: archive::hash_file(&binary(stage.join("payload").join(name)))?,
        });
    }
    let new_executable =
        if bundle { stage.join("payload").join(&name).join("Contents/MacOS/mailfabrica") } else { stage.join("payload").join(&name) };
    check_version(&new_executable, &release.version)?;
    let plan = Plan { target, root, bundle, entries, version: release.version.clone() };
    transaction::validate(&plan.root, stage, &plan.entries, bundle)?;
    fs::write(stage.join("plan.json"), serde_json::to_vec(&plan).map_err(|_| "Could not serialize update plan")?)
        .map_err(|_| "Could not save update plan")?;
    let guard = fs::File::create_new(stage.join("parent.lock")).map_err(|_| "Could not create update lock")?;
    guard.try_lock().map_err(|_| "Could not lock update transaction")?;
    let helper = stage.join(if cfg!(windows) { "helper.exe" } else { "helper" });
    fs::copy(&new_executable, &helper).map_err(|_| "Could not prepare update helper")?;
    let log = fs::File::create_new(stage.join("helper.log")).map_err(|_| "Could not create update log")?;
    Command::new(&helper)
        .arg("--apply-update")
        .arg(stage)
        .stdin(Stdio::null())
        .stdout(log.try_clone().map_err(|_| "Could not create update log")?)
        .stderr(log)
        .spawn()
        .map_err(|_| "Could not start update helper")?;
    Ok(Handoff { stage: temporary.keep(), _guard: guard })
}

/// 校验已签名 DMG 内的完整应用，再用 ditto 保留 bundle 元数据复制至同盘准备目录。
#[cfg(target_os = "macos")]
fn prepare_bundle(package: &Path, destination: &Path) -> Result<(), String> {
    let mount = tempfile::Builder::new().prefix("mailfabrica-mount-").tempdir().map_err(|_| "Could not create update mount point")?;
    let mount_path = mount.path();
    crate::publisher::run(Command::new("/usr/bin/hdiutil").args(["attach", "-readonly", "-nobrowse", "-mountpoint"]).arg(mount_path).arg(package))?;
    /// 所有校验或复制失败路径都卸载本次挂载的映像。
    struct Mounted<'a>(&'a Path);
    impl Drop for Mounted<'_> {
        fn drop(&mut self) {
            let _ = crate::publisher::run(Command::new("/usr/bin/hdiutil").arg("detach").arg(self.0));
        }
    }
    let _mounted = Mounted(mount_path);
    let app = mount_path.join("MailFabrica.app");
    verify_bundle(&app)?;
    crate::publisher::run(Command::new("/usr/bin/ditto").arg(&app).arg(destination))?;
    verify_bundle(destination)
}

/// Portable 平台不会进入 DMG 分支。
#[cfg(not(target_os = "macos"))]
fn prepare_bundle(_: &Path, _: &Path) -> Result<(), String> {
    Err("DMG updates require macOS".into())
}

/// 应用本体必须同 Team、完整签名、公证并通过执行信任检查。
#[cfg(target_os = "macos")]
fn verify_bundle(path: &Path) -> Result<(), String> {
    crate::publisher::run(Command::new("/usr/bin/codesign").args(["--verify", "--deep", "--strict"]).arg(path))?;
    let signature = crate::publisher::run(Command::new("/usr/bin/codesign").args(["-d", "--verbose=4"]).arg(path))?;
    crate::publisher::require_team(&signature)?;
    if !signature.lines().any(|line| line.trim() == "Identifier=dev.mailfabrica.MailFabrica") {
        return Err("The update bundle has an unexpected application identifier".into());
    }
    crate::publisher::run(Command::new("/usr/sbin/spctl").args(["--assess", "--type", "execute"]).arg(path))?;
    Ok(())
}

/// 限时启动版本检查，执行的是已通过发布来源与哈希校验的新二进制。
fn check_version(executable: &Path, version: &str) -> Result<(), String> {
    let output = tempfile::tempfile().map_err(|_| "Could not create version check output")?;
    let mut child = Command::new(executable)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(output.try_clone().map_err(|_| "Could not clone version output")?)
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "The new application could not start on this system")?;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) => return Err("The new application version check failed".into()),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("The new application version check timed out".into());
            }
        }
    }
    use std::io::{Seek, SeekFrom};
    let mut output = output;
    output.seek(SeekFrom::Start(0)).map_err(|_| "Could not read version output")?;
    let mut text = String::new();
    output.take(1024).read_to_string(&mut text).map_err(|_| "Invalid version output")?;
    if text.trim() != format!("MailFabrica {version}") {
        return Err("The update executable reports an unexpected version".into());
    }
    Ok(())
}

/// 只读取本次准备目录的有界计划，要求准备目录与安装目录同盘且辅助程序身份正确。
fn read_plan(stage: &Path) -> Result<Plan, String> {
    let stage = fs::canonicalize(stage).map_err(|_| "Invalid update directory")?;
    if !stage.file_name().and_then(|s| s.to_str()).is_some_and(|s| s.starts_with(".mailfabrica-update-")) {
        return Err("Invalid update directory name".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(stage.join("plan.json"))
        .map_err(|_| "Missing update plan")?
        .take(64 << 10)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read update plan")?;
    let plan: Plan = serde_json::from_slice(&bytes).map_err(|_| "Invalid update plan")?;
    let (root, bundle, _) = installation(&plan.target)?;
    if stage.parent() != Some(root.as_path()) || plan.root != root || plan.bundle != bundle {
        return Err("Update directory is outside the installation boundary".into());
    }
    Ok(plan)
}

/// 辅助进程等待父锁释放，确认正常提交，替换后等待新 GUI 首帧确认；失败回退旧应用。
pub fn run_helper(stage: &Path) -> Result<(), String> {
    let stage = fs::canonicalize(stage).map_err(|_| "Invalid update directory")?;
    let expected = stage.join(if cfg!(windows) { "helper.exe" } else { "helper" });
    if std::env::current_exe().and_then(fs::canonicalize).map_err(|_| "Could not locate update helper")? != expected {
        return Err("Update helper must run from the prepared directory".into());
    }
    let plan = read_plan(&stage)?;
    let guard = fs::File::options().read(true).write(true).open(stage.join("parent.lock")).map_err(|_| "Missing update parent lock")?;
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if guard.try_lock().is_ok() {
            break;
        }
        if Instant::now() >= deadline {
            return Err("Application did not exit; no files were replaced".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if fs::read(stage.join("commit")).ok().as_deref() != Some(b"apply") {
        return Err("Application did not commit the update; no files were replaced".into());
    }
    let result = (|| {
        transaction::validate(&plan.root, &stage, &plan.entries, plan.bundle)?;
        #[cfg(target_os = "macos")]
        if plan.bundle {
            for entry in &plan.entries {
                verify_bundle(&stage.join("payload").join(&entry.name))?;
            }
        }
        transaction::apply(&plan.root, &stage, &plan.entries)?;
        restart_and_confirm(&plan.target, &stage)
    })();
    if let Err(error) = result {
        transaction::rollback(&plan.root, &stage, &plan.entries)?;
        fs::write(stage.join("error.txt"), &error).map_err(|_| "Could not save update failure")?;
        Command::new(&plan.target.executable)
            .args(&plan.target.args)
            .arg("--update-error")
            .arg(&error)
            .spawn()
            .map_err(|_| "Previous application restored but could not restart")?;
        return Err(error);
    }
    fs::write(stage.join("success"), b"ok").map_err(|_| "Could not record update success")?;
    Ok(())
}

/// 首帧确认才算升级成功；新进程不启动或挂起时先收割它，再恢复原版本。
fn restart_and_confirm(target: &InstallTarget, stage: &Path) -> Result<(), String> {
    let mut child = Command::new(&target.executable)
        .args(&target.args)
        .arg("--update-ack")
        .arg(stage)
        .stdin(Stdio::null())
        .spawn()
        .map_err(|_| "Could not restart the updated application")?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if fs::read(stage.join("ready")).ok().as_deref() == Some(b"ready") {
            return Ok(());
        }
        match child.try_wait() {
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("The updated application did not become ready; restoring the previous version".into());
            }
        }
    }
}

/// 新 GUI 画完首帧后确认运行路径；后台清理成功事务，失败备份留待恢复。
pub fn acknowledge(stage: &Path) -> Result<(), String> {
    let stage = fs::canonicalize(stage).map_err(|_| "Invalid update confirmation directory")?;
    let plan = read_plan(&stage)?;
    if std::env::current_exe().and_then(fs::canonicalize).map_err(|_| "Could not locate updated application")? != plan.target.executable {
        return Err("Update confirmation came from a different application".into());
    }
    fs::write(stage.join("ready"), b"ready").map_err(|_| "Could not confirm update startup")?;
    std::thread::Builder::new()
        .name("update-cleanup".into())
        .spawn(move || {
            for _ in 0..60 {
                std::thread::sleep(Duration::from_secs(1));
                if stage.join("success").is_file() && fs::remove_dir_all(&stage).is_ok() {
                    break;
                }
            }
        })
        .map(|_| ())
        .map_err(|_| "Could not start update cleanup".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    /// 只有父锁释放且正常提交才可安装，异常关闭不能被误认为同意替换。
    #[test]
    fn handoff_requires_commit_and_parent_exit() {
        let tmp = tempfile::tempdir().expect("tmp");
        let path = tmp.path().join("parent.lock");
        let guard = fs::File::create_new(&path).expect("lock");
        guard.try_lock().expect("lock parent");
        let child = fs::File::options().read(true).write(true).open(&path).expect("child");
        assert!(child.try_lock().is_err());
        let handoff = Handoff { stage: tmp.path().into(), _guard: guard };
        assert!(!tmp.path().join("commit").exists());
        handoff.commit().expect("commit");
        drop(handoff);
        assert!(child.try_lock().is_ok());
        assert_eq!(fs::read(tmp.path().join("commit")).expect("marker"), b"apply");
    }
}
