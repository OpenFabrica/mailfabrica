//! 密码存储接口。桌面程序用系统钥匙串实现，测试和 demo 用内存实现。

use std::collections::HashMap;
use std::sync::Mutex;

use crate::Result;

/// 按账户 id 存取密码（或授权码）。实现必须线程安全。
pub trait SecretStore: Send + Sync {
    fn get(&self, account: &str) -> Option<String>;
    fn set(&self, account: &str, secret: &str) -> Result<()>;
    fn delete(&self, account: &str) -> Result<()>;
}

/// 只存在内存里的密码表。
#[derive(Default)]
pub struct MemorySecrets(Mutex<HashMap<String, String>>);

impl MemorySecrets {
    pub fn new() -> MemorySecrets {
        MemorySecrets::default()
    }
}

impl SecretStore for MemorySecrets {
    fn get(&self, account: &str) -> Option<String> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).get(account).cloned()
    }
    fn set(&self, account: &str, secret: &str) -> Result<()> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).insert(account.to_string(), secret.to_string());
        Ok(())
    }
    fn delete(&self, account: &str) -> Result<()> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).remove(account);
        Ok(())
    }
}
