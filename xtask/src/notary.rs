//! Developer ID 签名、公证与票据校验。只使用已有钥匙串 profile，不读取或输出密码。

use std::fs;
use std::path::Path;
use std::process::Command;

use crate::run;

/// Developer ID 签名身份，加上可选的公证钥匙串 profile。只有身份时是"签名的本地包"：不公证、不查版本和工作区，
/// 但签名每次都一样，钥匙串里点过的"始终允许"对之后的包仍然有效（临时签名每次打包都变，会反复弹授权框）。
/// 有 profile 时是正式分发，版本、工作区必须合规。
pub(crate) struct Config {
    identity: String,
    profile: Option<String>,
}

impl Config {
    /// 读取非秘密的配置名称；不设置时保持原有本地临时签名流程。
    pub(crate) fn load(version: &str, dirty: bool) -> Result<Option<Self>, String> {
        let value = |key| std::env::var(key).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        Self::validate(value("APPLE_SIGN_IDENTITY"), value("APPLE_NOTARY_PROFILE"), version, dirty)
    }

    /// 门禁：身份必须是 Developer ID Application；要公证时拒绝开发版本和脏工作区；只给 profile 不给身份是配置错误。
    fn validate(identity: Option<String>, profile: Option<String>, version: &str, dirty: bool) -> Result<Option<Self>, String> {
        let Some(identity) = identity else {
            return match profile {
                None => Ok(None),
                Some(_) => Err("APPLE_NOTARY_PROFILE needs APPLE_SIGN_IDENTITY".into()),
            };
        };
        if !identity.starts_with("Developer ID Application:") {
            return Err("APPLE_SIGN_IDENTITY must be a Developer ID Application identity".into());
        }
        if profile.is_some() && version.contains("-dev") {
            return Err("notarized distributions must not use a -dev version".into());
        }
        if profile.is_some() && dirty {
            return Err("commit all changes before building a notarized distribution".into());
        }
        Ok(Some(Self { identity, profile }))
    }

    /// 是否要提交 Apple 公证（设置了 profile）。
    pub(crate) fn notarizes(&self) -> bool {
        self.profile.is_some()
    }

    /// 公证用的钥匙串 profile；只签名不公证时调用是流程错误。
    fn profile(&self) -> Result<&str, String> {
        self.profile.as_deref().ok_or_else(|| "APPLE_NOTARY_PROFILE is not set".to_string())
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
        let profile = self.profile()?;
        fs::create_dir_all(audit).map_err(|e| e.to_string())?;
        let reply = json(Command::new("xcrun").args(["notarytool", "submit"]).arg(path).args([
            "--keychain-profile",
            profile,
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
            profile,
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
        log.args(["notarytool", "log", id, "--keychain-profile", profile]).arg(audit.join(format!("{id}-log.json")));
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

    /// 本地模式不需要证书；只签名不公证的本地包不查版本和工作区；公证模式必须在不可发布输入上失败。
    #[test]
    fn notarization_requires_complete_release_configuration() {
        let identity = || Some("Developer ID Application: Example (TEAMID)".into());
        let profile = || Some("existing-profile".into());
        assert!(matches!(Config::validate(None, None, "0.1.0-dev", true), Ok(None)));
        assert!(Config::validate(None, profile(), "0.1.0", false).is_err());
        assert!(matches!(Config::validate(identity(), None, "0.1.0-dev", true), Ok(Some(c)) if !c.notarizes()));
        assert!(Config::validate(Some("Apple Development: x".into()), None, "0.1.0-dev", true).is_err());
        assert!(Config::validate(identity(), profile(), "0.1.0-dev", false).is_err());
        assert!(Config::validate(identity(), profile(), "0.1.0", true).is_err());
        assert!(Config::validate(Some("-".into()), profile(), "0.1.0", false).is_err());
        assert!(matches!(Config::validate(identity(), profile(), "0.1.0", false), Ok(Some(c)) if c.notarizes()));
    }
}
