//! 系统密码存储：macOS 钥匙串 / Windows 凭据管理器（keyring crate）；其他系统或设置了
//! `MAILFABRICA_SECRETS=file` 时，退回到数据目录下权限 0600 的 `secrets.json`。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use mailfabrica_core::{MailError, Result, SecretStore};

/// 钥匙串里的服务名。
#[cfg(any(target_os = "macos", target_os = "windows"))]
const SERVICE: &str = "MailFabrica";

/// 桌面程序和 CLI 共用的密码存储（服务名 `MailFabrica`，用户名为账户 id）。
pub struct SystemSecrets {
    root: PathBuf,
    /// 是否用文件而不是系统钥匙串。
    file_mode: bool,
    /// 文件模式下的读写锁，防止同一进程里并发写坏文件。
    lock: Mutex<()>,
}

impl SystemSecrets {
    /// `root` 是数据目录（文件模式时 `secrets.json` 放在这里）。
    pub fn new(root: PathBuf) -> SystemSecrets {
        let forced = std::env::var("MAILFABRICA_SECRETS").is_ok_and(|v| v.eq_ignore_ascii_case("file"));
        let file_mode = forced || !cfg!(any(target_os = "macos", target_os = "windows"));
        SystemSecrets { root, file_mode, lock: Mutex::new(()) }
    }

    /// 强制用文件模式（测试用，不碰系统钥匙串）。
    #[cfg(test)]
    pub(crate) fn file_only(root: PathBuf) -> SystemSecrets {
        SystemSecrets { root, file_mode: true, lock: Mutex::new(()) }
    }

    /// `secrets.json` 的路径。
    fn path(&self) -> PathBuf {
        self.root.join("secrets.json")
    }

    /// 读文件里的密码表；文件不存在或损坏时为空。
    fn read_file(&self) -> BTreeMap<String, String> {
        std::fs::read(self.path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    /// 写密码表：先写权限 0600 的临时文件再改名，避免密码短暂地对其他用户可读。
    fn write_file(&self, map: &BTreeMap<String, String>) -> Result<()> {
        std::fs::create_dir_all(&self.root)?;
        let data = serde_json::to_vec_pretty(map).map_err(|e| MailError::Io(e.to_string()))?;
        let tmp = self.root.join("secrets.json.tmp");
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        {
            use std::io::Write;
            let mut f = opts.open(&tmp)?;
            f.write_all(&data)?;
            f.sync_all()?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&tmp, self.path())?;
        Ok(())
    }

    /// 文件模式下修改密码表。
    fn update_file(&self, f: impl FnOnce(&mut BTreeMap<String, String>)) -> Result<()> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut map = self.read_file();
        f(&mut map);
        self.write_file(&map)
    }

    /// 钥匙串条目。
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    fn entry(account: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(SERVICE, account).map_err(|e| MailError::Io(format!("keychain: {e}")))
    }
}

impl SecretStore for SystemSecrets {
    fn get(&self, account: &str) -> Option<String> {
        if self.file_mode {
            let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
            return self.read_file().get(account).cloned();
        }
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            Self::entry(account).ok()?.get_password().ok()
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        None
    }

    fn set(&self, account: &str, secret: &str) -> Result<()> {
        if self.file_mode {
            return self.update_file(|m| {
                m.insert(account.to_string(), secret.to_string());
            });
        }
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            Self::entry(account)?.set_password(secret).map_err(|e| MailError::Io(format!("keychain: {e}")))
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        Err(MailError::Unsupported("system keychain".into()))
    }

    fn delete(&self, account: &str) -> Result<()> {
        if self.file_mode {
            return self.update_file(|m| {
                m.remove(account);
            });
        }
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            match Self::entry(account)?.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(e) => Err(MailError::Io(format!("keychain: {e}"))),
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        Err(MailError::Unsupported("system keychain".into()))
    }
}
