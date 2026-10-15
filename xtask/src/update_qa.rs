//! 在有明确 QA 标识的临时安装中验证真实辅助进程替换、重启和用户文件保留。
use mailfabrica_update::{InstallTarget, Release};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    path::Path,
    time::{Duration, Instant},
};

/// 验收命令只能替换系统临时目录中带 mailfabrica-update-qa- 标识的程序。
pub(crate) fn run(args: &[String]) -> Result<(), String> {
    if args.len() != 4 {
        return Err("usage: cargo xtask update-qa <package> <version> <qa-executable> <port>".into());
    }
    let package = fs::canonicalize(args.first().ok_or("Missing package")?).map_err(|e| e.to_string())?;
    let version = args.get(1).ok_or("Missing version")?;
    let executable = fs::canonicalize(args.get(2).ok_or("Missing QA executable")?).map_err(|e| e.to_string())?;
    let port: u16 = args.get(3).ok_or("Missing QA port")?.parse().map_err(|_| "Invalid QA port")?;
    let temporary = fs::canonicalize(std::env::temp_dir()).map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    let is_temporary = executable.starts_with(&temporary) || executable.starts_with("/private/tmp");
    #[cfg(not(target_os = "macos"))]
    let is_temporary = executable.starts_with(&temporary);
    if !is_temporary || !executable.components().any(|p| p.as_os_str().to_str().is_some_and(|s| s.starts_with("mailfabrica-update-qa-"))) {
        return Err("Update QA must use a disposable mailfabrica-update-qa- installation in the system temporary directory".into());
    }
    let asset = package.file_name().and_then(|s| s.to_str()).ok_or("Invalid package name")?;
    let bytes = fs::read(&package).map_err(|e| e.to_string())?;
    let release = Release {
        version: version.clone(),
        notes: String::new(),
        page: format!("{}/releases/tag/v{version}", mailfabrica_update::REPOSITORY),
        asset: asset.into(),
        url: format!("{}/releases/download/v{version}/{asset}", mailfabrica_update::REPOSITORY),
        size: bytes.len() as u64,
        sha256: format!("{:x}", Sha256::digest(&bytes)),
    };
    let root = if cfg!(target_os = "macos") {
        executable.ancestors().nth(4).ok_or("Invalid QA bundle")?
    } else {
        executable.parent().ok_or("Invalid QA root")?
    };
    let data = root.join("qa-mail-data");
    fs::create_dir(&data).map_err(|e| e.to_string())?;
    let marker = data.join("accounts.json");
    fs::write(&marker, b"preserve-qa-user-data").map_err(|e| e.to_string())?;
    let target = InstallTarget { executable: executable.clone(), args: vec!["--demo".into(), "--control".into(), port.to_string()], data: None };
    let handoff = mailfabrica_update::prepare(&release, &package, target)?;
    let stage = handoff.directory().to_path_buf();
    handoff.commit()?;
    drop(handoff);
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        if let Ok(value) = rpc(port, "ui.inspect", serde_json::json!({}))
            && value["messages"].as_u64().unwrap_or(0) > 0
            && (stage.join("success").is_file() || !stage.exists())
        {
            rpc(port, "app.quit", serde_json::json!({}))?;
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!("Updated GUI did not become ready; inspect {}", root.display()));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    if fs::read(&marker).map_err(|e| e.to_string())? != b"preserve-qa-user-data" {
        return Err("User data changed during automatic replacement".into());
    }
    // 给辅助进程首帧确认与退出留时间，Windows 不允许删除仍在运行的辅助程序。
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let pending: Vec<_> = fs::read_dir(root)
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .filter(|p| p.file_name().to_str().is_some_and(|s| s.starts_with(".mailfabrica-update-")))
            .collect();
        if pending.is_empty() {
            break;
        }
        if pending.iter().any(|p| p.path().join("error.txt").exists()) {
            return Err("Automatic update rolled back unexpectedly".into());
        }
        // 成功标记由辅助进程写入；GUI 已退出时不依赖它的清理线程。
        if pending.iter().all(|p| p.path().join("success").is_file()) {
            break;
        }
        if Instant::now() >= deadline {
            return Err("Update helper did not confirm successful startup".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    wait_exit(port, &executable)?;
    println!("VERIFIED: automatic application replacement, GUI restart and preserved user files");
    Ok(())
}

/// 等更新后重启的程序真正退出：控制端口不再接受连接，Windows 上程序文件能以写方式打开（运行中的程序打不开）。
/// `app.quit` 只是发出退出请求；不等它退完，调用方马上删临时安装目录时，Windows 会因为程序文件还被占用而失败。
fn wait_exit(port: u16, executable: &Path) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let closed = TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(200)).is_err();
        let unlocked = !cfg!(windows) || fs::OpenOptions::new().write(true).open(executable).is_ok();
        if closed && unlocked {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("Updated GUI did not exit after app.quit".into());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// 有界本地协议请求，仅访问本次临时演示程序的控制端口。
fn rpc(port: u16, method: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
    let mut connection = TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(500)).map_err(|e| e.to_string())?;
    connection.set_read_timeout(Some(Duration::from_secs(3))).map_err(|e| e.to_string())?;
    let message = serde_json::json!({"id":1,"method":method,"params":params});
    writeln!(connection, "{message}").map_err(|e| e.to_string())?;
    let mut response = String::new();
    BufReader::new(connection).take(1 << 20).read_line(&mut response).map_err(|e| e.to_string())?;
    let response: serde_json::Value = serde_json::from_str(&response).map_err(|e| e.to_string())?;
    if response["ok"] != true {
        return Err("QA control request failed".into());
    }
    Ok(response["result"].clone())
}
