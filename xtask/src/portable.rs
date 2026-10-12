//! 原生 Windows/Linux portable 发布包；不在 macOS 上交叉拼装未运行验证的二进制。
use crate::{cargo, icon, metadata, root, run};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::Command;

/// 平台名与 CPU 名的统一发布映射；amd64 等价于 Rust 的 x86_64。
fn platform(os: &str, arch: &str) -> Result<(&'static str, &'static str, &'static str), String> {
    let arch = match arch {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        _ => return Err(format!("unsupported CPU: {arch}")),
    };
    match os {
        "windows" => Ok(("windows", arch, "zip")),
        "linux" => Ok(("linux", arch, "tar.gz")),
        _ => Err(format!("portable packaging is not supported on {os}")),
    }
}

/// 构建、校验运行版本并归档两个程序、共享图标、许可证和本机安装说明。
pub(crate) fn build() -> Result<(), String> {
    let (os, arch, extension) = platform(std::env::consts::OS, std::env::consts::ARCH)?;
    if std::env::var_os("CARGO_BUILD_TARGET").is_some() {
        return Err("unset CARGO_BUILD_TARGET; package on the native target runner".into());
    }
    let meta = metadata()?;
    let version = meta
        .get("packages")
        .and_then(|p| p.as_array())
        .and_then(|p| p.iter().find(|p| p.get("name").and_then(|n| n.as_str()) == Some("mailfabrica")))
        .and_then(|p| p.get("version"))
        .and_then(|v| v.as_str())
        .ok_or("cannot read the application version")?;
    let target = meta.get("target_directory").and_then(|t| t.as_str()).ok_or("cannot read cargo target_directory")?;
    let revision = git(&["rev-parse", "HEAD"])?;
    let dirty = !git(&["status", "--porcelain"])?.is_empty();
    if std::env::var("MAILFABRICA_RELEASE").ok().as_deref() == Some("1") {
        if dirty || version.contains("-dev") {
            return Err("release packages require a clean, stable version".into());
        }
        let tag = format!("v{version}");
        if git(&["rev-parse", &format!("{tag}^{{commit}}")])? != revision {
            return Err("release tag must point to the current commit".into());
        }
        let mut ancestry = Command::new("git");
        ancestry.current_dir(root()).args(["merge-base", "--is-ancestor", &tag, "origin/master"]);
        run(ancestry, "release tag must belong to origin/master")?;
    }
    let full_version = if version.contains("-dev") {
        format!("{version}-{}{}", git(&["rev-parse", "--short=7", "HEAD"])?, if dirty { "+" } else { "" })
    } else {
        version.into()
    };
    let name = format!("MailFabrica-{full_version}-{os}-{arch}");
    let dist = root().join("dist");
    let destination = dist.join(&name);
    if destination.exists() {
        return Err(format!("package already exists: {}", destination.display()));
    }
    let mut compile = cargo();
    compile.args(["build", "--release", "--locked", "-p", "mailfabrica", "-p", "mailfabrica-cli"]);
    run(compile, "build native release binaries")?;
    fs::create_dir_all(&dist).map_err(|e| e.to_string())?;
    let stage = tempfile::tempdir_in(&dist).map_err(|e| e.to_string())?;
    let payload = stage.path().join(&name);
    fs::create_dir_all(&payload).map_err(|e| e.to_string())?;
    let mut hashes = serde_json::Map::new();
    for binary in ["mailfabrica", "mailfabrica-cli"] {
        let filename = if os == "windows" { format!("{binary}.exe") } else { binary.into() };
        let path = payload.join(&filename);
        fs::copy(Path::new(target).join("release").join(&filename), &path).map_err(|e| e.to_string())?;
        let mut smoke = Command::new(&path);
        smoke.arg(if binary == "mailfabrica" { "--version" } else { "version" });
        let output = capture(&mut smoke)?;
        let expected = if binary == "mailfabrica" { format!("MailFabrica {version}") } else { format!("mailfabrica-cli {version}") };
        if output.trim() != expected {
            return Err(format!("{filename} reports the wrong application version"));
        }
        hashes.insert(filename, serde_json::json!(hash_file(&path)?));
    }
    for license in ["LICENSE-MIT", "LICENSE-APACHE"] {
        fs::copy(root().join(license), payload.join(license)).map_err(|e| e.to_string())?;
    }
    icon::render(256, &payload.join("MailFabrica-logo.png"))?;
    let instructions = if os == "windows" {
        "解压整个目录后运行 mailfabrica.exe。命令行程序为 mailfabrica-cli.exe。\n"
    } else {
        "解压后运行 ./mailfabrica。命令行程序为 ./mailfabrica-cli。\n需要 glibc 2.35 或更新的 Linux、Vulkan/OpenGL 驱动、fontconfig 和 libxkbcommon-x11；中文字体建议安装 Noto CJK。\nUbuntu/Debian 可安装运行依赖：sudo apt install libfontconfig1 libxkbcommon-x11-0 libvulkan1 mesa-vulkan-drivers fonts-noto-cjk\n可将程序安装到 ~/.local/bin，将图标安装到 ~/.local/share/icons/hicolor/256x256/apps/mailfabrica.png，将 mailfabrica.desktop 安装到 ~/.local/share/applications。\n"
    };
    fs::write(payload.join("README.txt"),format!("MailFabrica {full_version}\n{instructions}\n此平台通过官方发布页手动下载、解压并替换应用。\nhttps://github.com/OpenFabrica/mailfabrica/releases/latest\n")).map_err(|e| e.to_string())?;
    if os == "linux" {
        fs::write(payload.join("mailfabrica.desktop"),"[Desktop Entry]\nType=Application\nName=MailFabrica\nComment=Native desktop mail client\nExec=mailfabrica\nIcon=mailfabrica\nTerminal=false\nCategories=Network;Email;\n").map_err(|e| e.to_string())?;
    }
    let manifest = serde_json::json!({"version":full_version,"revision":revision,"dirty":dirty,"platform":os,"arch":arch,"sha256":hashes});
    let record = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
    fs::write(payload.join("build.json"), &record).map_err(|e| e.to_string())?;
    fs::write(stage.path().join(format!("{name}.json")), record).map_err(|e| e.to_string())?;
    let archive = stage.path().join(format!("{name}.{extension}"));
    if os == "windows" {
        // 路径通过子进程环境传递，PowerShell 只执行固定命令，不把用户目录插进脚本文本。
        let mut command = Command::new("powershell.exe");
        command.args(["-NoProfile", "-NonInteractive", "-Command",
            "$ErrorActionPreference='Stop'; Compress-Archive -LiteralPath $env:MAILFABRICA_PACKAGE_PAYLOAD -DestinationPath $env:MAILFABRICA_PACKAGE_ARCHIVE -CompressionLevel Optimal"])
            .env("MAILFABRICA_PACKAGE_PAYLOAD", &payload).env("MAILFABRICA_PACKAGE_ARCHIVE", &archive);
        run(command, "archive Windows portable package")?;
    } else {
        let mut command = Command::new("tar");
        command.args(["-czf"]).arg(&archive).arg("-C").arg(stage.path()).arg("--").arg(&name);
        run(command, "archive Linux portable package")?;
    }
    fs::write(stage.path().join(format!("{name}.sha256")), format!("{}  {name}.{extension}\n", hash_file(&archive)?)).map_err(|e| e.to_string())?;
    fs::remove_dir_all(payload).map_err(|e| e.to_string())?;
    fs::rename(stage.path(), &destination).map_err(|e| e.to_string())?;
    println!("Package: {}", destination.join(format!("{name}.{extension}")).display());
    Ok(())
}

/// 对已生成的有限文件流式哈希，避免把整个安装包读入内存。
fn hash_file(path: &Path) -> Result<String, String> {
    const MAX: u64 = 256 << 20;
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?.take(MAX + 1);
    let mut total = 0_u64;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 << 10];
    loop {
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > MAX {
            return Err("package file exceeds 256 MiB".into());
        }
        hash.update(buffer.get(..count).ok_or("invalid hash buffer")?);
    }
    Ok(format!("{:x}", hash.finalize()))
}

/// 捕获完整参数执行的命令结果，只接受成功退出的输出。
fn capture(command: &mut Command) -> Result<String, String> {
    let output = command.output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    String::from_utf8(output.stdout).map_err(|e| e.to_string())
}

/// 从当前 checkout 读取 Git 门禁数据，不接受外部 shell 片段。
fn git(args: &[&str]) -> Result<String, String> {
    capture(Command::new("git").current_dir(root()).args(args)).map(|s| s.trim().into())
}

#[cfg(test)]
mod tests {
    use super::*;
    /// 发布文件名采用同一映射，未支持的平台/架构不会被静默当成 amd64。
    #[test]
    fn native_platform_names_are_explicit() {
        assert_eq!(platform("windows", "x86_64").expect("Windows"), ("windows", "amd64", "zip"));
        assert_eq!(platform("windows", "aarch64").expect("Windows ARM"), ("windows", "arm64", "zip"));
        assert_eq!(platform("linux", "aarch64").expect("Linux ARM"), ("linux", "arm64", "tar.gz"));
        assert!(platform("linux", "riscv64").is_err());
        assert!(platform("macos", "aarch64").is_err());
    }
}
