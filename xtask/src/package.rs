//! 本地 macOS 打包：构建原生 release、生成代码图标、临时签名并产出经过校验的应用和 DMG。

use std::fs;
use std::path::Path;
use std::process::Command;

use crate::notary;
use crate::{cargo, icon, metadata, root, run};

/// 从当前 checkout 构建两个程序并打包；失败时清理临时目录，保留已有产物。
pub(crate) fn build() -> Result<(), String> {
    let meta = metadata()?;
    let packages = meta.get("packages").and_then(|p| p.as_array()).ok_or("cargo metadata has no packages")?;
    let version = packages
        .iter()
        .find(|p| p.get("name").and_then(|v| v.as_str()) == Some("mailfabrica"))
        .and_then(|p| p.get("version"))
        .and_then(|v| v.as_str())
        .ok_or("cannot read app version")?;
    let target = meta.get("target_directory").and_then(|p| p.as_str()).ok_or("cargo metadata has no target_directory")?;
    // 显式拒绝交叉编译环境，避免把别的平台二进制误放进本机应用包。
    if std::env::var_os("CARGO_BUILD_TARGET").is_some() {
        return Err("unset CARGO_BUILD_TARGET for native macOS packaging".into());
    }
    let revision = git(&["rev-parse", "HEAD"])?;
    let short = git(&["rev-parse", "--short=7", "HEAD"])?;
    let dirty = !git(&["status", "--porcelain"])?.is_empty();
    let notarization = notary::Config::load(version, dirty)?;
    if notarization.is_some() && git(&["rev-parse", &format!("v{version}^{{commit}}")])? != revision {
        return Err("the release version tag must point to the current commit".into());
    }
    if notarization.is_some() {
        let mut ancestry = Command::new("git");
        ancestry.current_dir(root()).args(["merge-base", "--is-ancestor", &format!("v{version}"), "master"]);
        run(ancestry, "release tag must belong to master")?;
    }
    let full_version = if version.contains("-dev") { format!("{version}-{short}{}", if dirty { "+" } else { "" }) } else { version.to_string() };
    let arch = if std::env::consts::ARCH == "aarch64" { "arm64" } else { std::env::consts::ARCH };
    let name = format!("MailFabrica-{full_version}-{arch}");
    let dist = root().join("dist");
    let destination = dist.join(&name);
    if destination.exists() {
        return Err(format!("package already exists: {}", destination.display()));
    }
    let mut command = cargo();
    command.args(["build", "--release", "--locked", "-p", "mailfabrica", "-p", "mailfabrica-cli"]);
    run(command, "cargo build --release --locked -p mailfabrica -p mailfabrica-cli")?;
    fs::create_dir_all(&dist).map_err(|e| e.to_string())?;
    let stage = tempfile::tempdir_in(&dist).map_err(|e| e.to_string())?;
    let payload = stage.path().join("payload");
    let app = payload.join("MailFabrica.app");
    let contents = app.join("Contents");
    let binaries = contents.join("MacOS");
    let resources = contents.join("Resources");
    fs::create_dir_all(&binaries).map_err(|e| e.to_string())?;
    fs::create_dir_all(&resources).map_err(|e| e.to_string())?;
    for executable in ["mailfabrica", "mailfabrica-cli"] {
        let binary = binaries.join(executable);
        fs::copy(Path::new(target).join("release").join(executable), &binary).map_err(|e| format!("copy {executable}: {e}"))?;
        tool("/usr/bin/lipo", &["-verify_arch".as_ref(), arch.as_ref(), binary.as_os_str()])?;
        sign(&binary, notarization.as_ref())?;
    }
    let iconset = stage.path().join("MailFabrica.iconset");
    fs::create_dir_all(&iconset).map_err(|e| e.to_string())?;
    for size in [16, 32, 128, 256, 512] {
        icon::render(size, &iconset.join(format!("icon_{size}x{size}.png")))?;
        icon::render(size * 2, &iconset.join(format!("icon_{size}x{size}@2x.png")))?;
    }
    icon::render(1024, &stage.path().join("MailFabrica-logo.png"))?;
    let icns = resources.join("MailFabrica.icns");
    tool("/usr/bin/iconutil", &["-c".as_ref(), "icns".as_ref(), iconset.as_os_str(), "-o".as_ref(), icns.as_os_str()])?;
    let numeric = version.split('-').next().unwrap_or(version);
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>MailFabrica</string>
<key>CFBundleDisplayName</key><string>MailFabrica</string>
<key>CFBundleIdentifier</key><string>dev.mailfabrica.MailFabrica</string>
<key>CFBundleExecutable</key><string>mailfabrica</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>{numeric}</string>
<key>CFBundleVersion</key><string>{numeric}</string>
<key>CFBundleGetInfoString</key><string>MailFabrica {full_version}</string>
<key>CFBundleIconFile</key><string>MailFabrica.icns</string>
<key>NSHighResolutionCapable</key><true/>
<key>NSPrincipalClass</key><string>NSApplication</string>
<key>LSApplicationCategoryType</key><string>public.app-category.productivity</string>
</dict></plist>
"#
    );
    let info = contents.join("Info.plist");
    fs::write(&info, plist).map_err(|e| e.to_string())?;
    for license in ["LICENSE-MIT", "LICENSE-APACHE"] {
        fs::copy(root().join(license), resources.join(license)).map_err(|e| e.to_string())?;
    }
    tool("/usr/bin/plutil", &["-lint".as_ref(), info.as_os_str()])?;
    sign(&app, notarization.as_ref())?;
    tool("/usr/bin/codesign", &["--verify".as_ref(), "--deep".as_ref(), "--strict".as_ref(), app.as_os_str()])?;
    let version_output = output(Command::new(binaries.join("mailfabrica")).arg("--version"))?;
    if version_output.trim() != format!("MailFabrica {version}") {
        return Err("packaged binary version does not match Cargo.toml".into());
    }
    std::os::unix::fs::symlink("/Applications", payload.join("Applications")).map_err(|e| e.to_string())?;
    let notice = if notarization.is_some() {
        "此包经过 Developer ID 签名和 Apple 公证。"
    } else {
        "此包使用本地临时签名，未进行 Apple 公证。"
    };
    fs::write(payload.join("README.txt"), format!("MailFabrica {full_version}\n将 MailFabrica.app 拖入 Applications 后启动。\n命令行程序位于 MailFabrica.app/Contents/MacOS/mailfabrica-cli。\n{notice}\n")).map_err(|e| e.to_string())?;
    let audit = dist.join(format!("notary-{name}"));
    let mut submissions = Vec::new();
    if let Some(config) = &notarization {
        // 先公证并 staple 应用，再把带票据的应用放入 DMG，才能保证拖出后离线验证也成功。
        let archive = stage.path().join("application.zip");
        tool("/usr/bin/ditto", &["-c".as_ref(), "-k".as_ref(), "--keepParent".as_ref(), app.as_os_str(), archive.as_os_str()])?;
        submissions.push(config.submit(&archive, &audit)?);
        config.staple(&app, true)?;
        fs::remove_file(archive).map_err(|e| e.to_string())?;
    }
    let dmg = stage.path().join(format!("{name}.dmg"));
    tool(
        "/usr/bin/hdiutil",
        &[
            "create".as_ref(),
            "-volname".as_ref(),
            "MailFabrica".as_ref(),
            "-srcfolder".as_ref(),
            payload.as_os_str(),
            "-format".as_ref(),
            "UDZO".as_ref(),
            dmg.as_os_str(),
        ],
    )?;
    tool("/usr/bin/hdiutil", &["verify".as_ref(), dmg.as_os_str()])?;
    if let Some(config) = &notarization {
        config.sign(&dmg, false)?;
        submissions.push(config.submit(&dmg, &audit)?);
        config.staple(&dmg, false)?;
    }
    let manifest = serde_json::json!({
        "version": full_version, "revision": revision, "dirty": dirty, "arch": arch,
        "createdUnix": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(|e| e.to_string())?.as_secs(),
        "sha256": { "mailfabrica": sha256(&binaries.join("mailfabrica"))?, "mailfabrica-cli": sha256(&binaries.join("mailfabrica-cli"))?, "dmg": sha256(&dmg)? },
        "signature": if notarization.is_some() { "Developer ID Application" } else { "ad-hoc" },
        "notarized": notarization.is_some(), "notarizationSubmissions": submissions,
    });
    fs::write(stage.path().join("build.json"), serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    fs::rename(&app, stage.path().join("MailFabrica.app")).map_err(|e| e.to_string())?;
    fs::remove_dir_all(&payload).map_err(|e| e.to_string())?;
    fs::remove_dir_all(&iconset).map_err(|e| e.to_string())?;
    fs::rename(stage.path(), &destination).map_err(|e| e.to_string())?;
    println!("Application: {}", destination.join("MailFabrica.app").display());
    println!("DMG: {}", destination.join(format!("{name}.dmg")).display());
    println!("Build manifest: {}", destination.join("build.json").display());
    Ok(())
}

/// 正式模式对每个组件独立签名，本地模式保留无证书的临时签名。
fn sign(path: &Path, config: Option<&notary::Config>) -> Result<(), String> {
    match config {
        Some(config) => config.sign(path, true),
        None => tool("/usr/bin/codesign", &["--force".as_ref(), "--sign".as_ref(), "-".as_ref(), "--timestamp=none".as_ref(), path.as_os_str()]),
    }
}

/// 在当前 checkout 读取 git 元数据，失败时保留命令的诊断。
fn git(args: &[&str]) -> Result<String, String> {
    output(Command::new("git").current_dir(root()).args(args)).map(|s| s.trim().to_string())
}

/// 不经 shell 执行工具，路径中的空格和符号都作为完整参数传递。
fn tool(program: &str, args: &[&std::ffi::OsStr]) -> Result<(), String> {
    let mut command = Command::new(program);
    command.args(args);
    run(command, program)
}

/// 捕获只读命令的输出，检查退出码并报告失败原因。
fn output(command: &mut Command) -> Result<String, String> {
    let result = command.output().map_err(|e| e.to_string())?;
    if !result.status.success() {
        return Err(String::from_utf8_lossy(&result.stderr).into_owned());
    }
    String::from_utf8(result.stdout).map_err(|e| e.to_string())
}

/// 记录实际交付文件的 SHA-256。
fn sha256(path: &Path) -> Result<String, String> {
    let hash = output(Command::new("/usr/bin/shasum").args(["-a", "256"]).arg(path))?;
    hash.split_whitespace().next().filter(|s| s.len() == 64).map(str::to_string).ok_or_else(|| "invalid SHA-256 output".into())
}
