//! Developer ID 签名、公证与票据校验。只使用已有钥匙串 profile，不读取或输出密码。

use std::fs;
use std::path::Path;
use std::process::Command;

use crate::run;

/// 正式分发所需的签名身份和钥匙串 profile；仅在成对提供且版本、工作区合规时启用。
pub(crate) struct Config {
    identity: String,
    profile: String,
}

impl Config {
    /// 读取非秘密的配置名称；不设置时保持原有本地临时签名流程。
    pub(crate) fn load(version: &str, dirty: bool) -> Result<Option<Self>, String> {
        let value = |key| std::env::var(key).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        Self::validate(value("APPLE_SIGN_IDENTITY"), value("APPLE_NOTARY_PROFILE"), version, dirty)
    }

    /// 公证门禁：拒绝开发版本、脏工作区和非 Developer ID Application 身份。
    fn validate(identity: Option<String>, profile: Option<String>, version: &str, dirty: bool) -> Result<Option<Self>, String> {
        match (identity, profile) {
            (None, None) => Ok(None),
            (Some(identity), Some(profile)) => {
                if version.contains("-dev") {
                    return Err("notarized distributions must not use a -dev version".into());
                }
                if dirty {
                    return Err("commit all changes before building a notarized distribution".into());
                }
                if !identity.starts_with("Developer ID Application:") {
                    return Err("APPLE_SIGN_IDENTITY must be a Developer ID Application identity".into());
                }
                Ok(Some(Self { identity, profile }))
            }
            _ => Err("set both APPLE_SIGN_IDENTITY and APPLE_NOTARY_PROFILE, or leave both unset for local packaging".into()),
        }
    }

    /// 对 Mach-O 和应用启用 Hardened Runtime，使用 Developer ID 和安全时间戳；DMG 只签名。
    pub(crate) fn sign(&self, path: &Path, runtime: bool) -> Result<(), String> {
        let mut command = Command::new("/usr/bin/codesign");
        command.args(["--force", "--sign", &self.identity, "--timestamp"]);
        if runtime {
            command.args(["--options", "runtime"]);
        }
        command.arg(path);
        run(command, "Developer ID codesign")
    }

    /// 提交并等待 Accepted；保留独立审计文件，失败或超时也能用 UUID 查回 Apple 状态。
    pub(crate) fn submit(&self, path: &Path, audit: &Path) -> Result<String, String> {
        fs::create_dir_all(audit).map_err(|e| e.to_string())?;
        let reply = json(Command::new("xcrun").args(["notarytool", "submit"]).arg(path).args([
            "--keychain-profile",
            &self.profile,
            "--output-format",
            "json",
            "--no-progress",
        ]))?;
        let id = reply
            .get("id")
            .and_then(|s| s.as_str())
            .filter(|s| s.len() == 36 && s.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'))
            .ok_or("notary service did not return a submission UUID")?;
        fs::write(audit.join(format!("{id}-submission.json")), serde_json::to_vec_pretty(&reply).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        println!("Apple notarization submitted: {id}");
        let status = json(Command::new("xcrun").args([
            "notarytool",
            "wait",
            id,
            "--keychain-profile",
            &self.profile,
            "--timeout",
            "15m",
            "--output-format",
            "json",
            "--no-progress",
        ]))
        .map_err(|e| format!("notarization {id} did not complete: {e}; submission receipt: {}", audit.display()))?;
        fs::write(audit.join(format!("{id}-status.json")), serde_json::to_vec_pretty(&status).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        let mut log = Command::new("xcrun");
        log.args(["notarytool", "log", id, "--keychain-profile", &self.profile]).arg(audit.join(format!("{id}-log.json")));
        run(log, "download Apple notarization log")?;
        if status.get("status").and_then(|s| s.as_str()) != Some("Accepted") {
            return Err(format!("Apple did not accept submission {id}; inspect {}", audit.display()));
        }
        println!("Apple notarization Accepted: {id}");
        Ok(id.to_string())
    }

    /// 将公证票据附到应用或 DMG，并检查离线票据和 Gatekeeper 判定。
    pub(crate) fn staple(&self, path: &Path, app: bool) -> Result<(), String> {
        for action in ["staple", "validate"] {
            let mut command = Command::new("xcrun");
            command.args(["stapler", action]).arg(path);
            run(command, "stapler ticket verification")?;
        }
        let mut command = Command::new("/usr/sbin/spctl");
        command.args(["--assess", "--verbose=2", "--type", if app { "execute" } else { "open" }]);
        if !app {
            command.args(["--context", "context:primary-signature"]);
        }
        command.arg(path);
        run(command, "Gatekeeper assessment")
    }
}

/// 捕获 notarytool 的机器可读响应，只在明确成功时解析 JSON。
fn json(command: &mut Command) -> Result<serde_json::Value, String> {
    let output = command.output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }
    serde_json::from_slice(&output.stdout).map_err(|e| format!("invalid notary JSON: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 本地模式不需要证书；公证模式必须在不可发布输入上失败。
    #[test]
    fn notarization_requires_complete_release_configuration() {
        let identity = || Some("Developer ID Application: Example (TEAMID)".into());
        let profile = || Some("existing-profile".into());
        assert!(matches!(Config::validate(None, None, "0.1.0-dev", true), Ok(None)));
        assert!(Config::validate(identity(), None, "0.1.0", false).is_err());
        assert!(Config::validate(identity(), profile(), "0.1.0-dev", false).is_err());
        assert!(Config::validate(identity(), profile(), "0.1.0", true).is_err());
        assert!(Config::validate(Some("-".into()), profile(), "0.1.0", false).is_err());
        assert!(matches!(Config::validate(identity(), profile(), "0.1.0", false), Ok(Some(_))));
    }
}
