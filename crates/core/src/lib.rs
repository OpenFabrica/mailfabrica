//! MailFabrica 的基础类型（L0 层）：地址、账户、文件夹、邮件元数据、日期格式化、错误、密码存储接口。
//!
//! 这一层没有网络、没有磁盘、没有 UI。上层 crate 都只通过这些类型交流。
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod account;
pub mod address;
pub mod date;
pub mod error;
pub mod folder;
pub mod message;
pub mod secrets;

pub use account::{AccountConfig, Security, ServerConfig, detect_servers};
pub use address::Address;
pub use error::{MailError, Result};
pub use folder::{Folder, FolderRole};
pub use message::{AttachmentInfo, Contact, Flag, Flags, MessageMeta, MessageView, MsgKey, OutAttachment, Outgoing};
pub use secrets::{MemorySecrets, SecretStore};

#[cfg(test)]
mod tests;
