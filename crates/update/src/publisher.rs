//! macOS 系统信任与 Apple Team 验证；哈希匹配不等于发布者可信。
use std::path::Path;

/// macOS 验证 Team 和公证；portable 平台的来源信任由固定 GitHub URL 与必需 digest 提供。
pub(crate) fn verify(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        run(Command::new("/usr/bin/codesign").args(["--verify", "--strict"]).arg(path))?;
        let signature = run(Command::new("/usr/bin/codesign").args(["-d", "--verbose=4"]).arg(path))?;
        require_team(&signature)?;
        run(Command::new("/usr/sbin/spctl").args(["--assess", "--type", "open", "--context", "context:primary-signature"]).arg(path))?;
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        Ok(())
    }
}

/// 必须匹配实际证书的 TeamIdentifier，不能用可由其他证书仿冒的显示名称。
#[cfg(any(target_os = "macos", test))]
pub(crate) fn require_team(signature: &str) -> Result<(), String> {
    if !signature.lines().any(|line| line.trim() == "TeamIdentifier=HQ537XMLJY") {
        return Err("Installer is not signed by the MailFabrica publisher".into());
    }
    Ok(())
}

/// 不经 shell 执行固定系统工具；限时并收割子进程，避免后台检查永久挂起。
#[cfg(target_os = "macos")]
pub(crate) fn run(command: &mut std::process::Command) -> Result<String, String> {
    use std::process::Stdio;
    use std::time::{Duration, Instant};
    let mut child = command.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|_| "Could not start Apple signature verification")?;
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Apple signature verification timed out or failed".into());
            }
        }
    }
    let output = child.wait_with_output().map_err(|_| "Could not read Apple signature verification")?;
    if !output.status.success() {
        return Err("Apple signature or Gatekeeper verification failed".into());
    }
    Ok(format!("{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr)))
}

#[cfg(test)]
mod tests {
    use super::*;
    /// 同名证书、缺少 TeamIdentifier 或另一个 Apple Team 都不能通过。
    #[test]
    fn publisher_team_must_match_exactly() {
        assert!(require_team("Authority=Developer ID Application: Jiangwei Lan\nTeamIdentifier=HQ537XMLJY\n").is_ok());
        assert!(require_team("Authority=Developer ID Application: Jiangwei Lan\nTeamIdentifier=OTHERTEAM\n").is_err());
        assert!(require_team("Authority=HQ537XMLJY").is_err());
        assert!(require_team("TeamIdentifier=HQ537XMLJY-extra").is_err());
    }
}
