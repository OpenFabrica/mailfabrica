//! 本地存储（L2 层）：账户、文件夹列表、文件夹邮件索引、邮件原文、联系人，全部存在一个目录下。
//!
//! 目录结构（学 Foxmail，每个账户一个目录，方便备份和迁移）：
//! ```text
//! <root>/accounts.json                    Vec<AccountConfig>
//! <root>/contacts.json                    Vec<Contact>
//! <root>/<account-id>/folders.json        Vec<Folder>
//! <root>/<account-id>/<folder-dir>/index.json   { uidvalidity, messages: Vec<MessageMeta> }
//! <root>/<account-id>/<folder-dir>/<uid>.eml    原文
//! ```
//! `folder-dir` 是文件夹路径的稳定哈希（`f-<16位十六进制>`），避免特殊字符。
//! 写文件先写临时文件再 rename，断电不会留下半个 JSON。索引按需加载进内存缓存。
//! 只有引擎线程持有 `Store`（单一所有者），后台同步线程不直接碰磁盘。
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use mailfabrica_core::{AccountConfig, Address, Contact, Flags, Folder, MailError, MessageMeta, MsgKey, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// 一个文件夹的邮件索引（`index.json` 的内容）。
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct FolderIndex {
    uidvalidity: u32,
    messages: Vec<MessageMeta>,
}

/// 本地存储。
pub struct Store {
    root: PathBuf,
    accounts: Vec<AccountConfig>,
    contacts: Vec<Contact>,
    /// 每个账户的文件夹列表（已排序）。打开时为已知账户加载，`set_folders` 时更新。
    folders: HashMap<String, Vec<Folder>>,
    /// 按 (账户, 文件夹路径) 懒加载的索引缓存。
    indexes: HashMap<(String, String), FolderIndex>,
}

impl Store {
    /// 打开（不存在就创建）存储目录，读取账户和联系人。
    pub fn open(root: impl Into<PathBuf>) -> Result<Store> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        let accounts: Vec<AccountConfig> = read_json(&root.join("accounts.json"));
        let mut contacts: Vec<Contact> = read_json(&root.join("contacts.json"));
        sort_contacts(&mut contacts);
        let mut folders = HashMap::new();
        for a in &accounts {
            if let Some(dir) = account_dir(&root, &a.id) {
                let mut list: Vec<Folder> = read_json(&dir.join("folders.json"));
                sort_folders(&mut list);
                folders.insert(a.id.clone(), list);
            }
        }
        Ok(Store { root, accounts, contacts, folders, indexes: HashMap::new() })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    // ---------------------------------------------------------------- 账户

    pub fn accounts(&self) -> &[AccountConfig] {
        &self.accounts
    }

    /// 新增或按 `id` 覆盖一个账户，立即写盘。
    pub fn upsert_account(&mut self, account: AccountConfig) -> Result<()> {
        let dir = self.account_dir(&account.id)?;
        match self.accounts.iter_mut().find(|a| a.id == account.id) {
            Some(a) => *a = account,
            None => {
                // 新账户：目录里可能还有上次留下的文件夹列表，一并读进来
                let mut list: Vec<Folder> = read_json(&dir.join("folders.json"));
                sort_folders(&mut list);
                self.folders.insert(account.id.clone(), list);
                self.accounts.push(account);
            }
        }
        self.save_accounts()
    }

    /// 删除账户及其全部本地数据。
    pub fn remove_account(&mut self, id: &str) -> Result<()> {
        let dir = self.account_dir(id)?;
        self.accounts.retain(|a| a.id != id);
        self.folders.remove(id);
        self.indexes.retain(|(acc, _), _| acc != id);
        self.save_accounts()?;
        remove_dir(&dir)
    }

    // ---------------------------------------------------------------- 文件夹

    /// 账户的文件夹，按 `FolderRole::order()` 再按路径排序。
    pub fn folders(&self, account: &str) -> Vec<Folder> {
        if let Some(list) = self.folders.get(account) {
            return list.clone();
        }
        // 不在缓存里（账户还没登记）：直接读盘，不缓存
        let Some(dir) = account_dir(&self.root, account) else { return Vec::new() };
        let mut list: Vec<Folder> = read_json(&dir.join("folders.json"));
        sort_folders(&mut list);
        list
    }

    /// 用服务器上的列表替换本地文件夹列表。保留本地已有的 `unread`/`total`/`uidvalidity`
    /// （新列表里对应值为 0 时）；服务器上已消失的文件夹连同本地数据一起删除。
    pub fn set_folders(&mut self, account: &str, folders: Vec<Folder>) -> Result<()> {
        let dir = self.account_dir(account)?;
        let old = self.folders(account);
        let mut list: Vec<Folder> = Vec::with_capacity(folders.len());
        for mut f in folders {
            // 同一路径只保留第一个，防止服务器返回重复项
            if list.iter().any(|x| x.path == f.path) {
                continue;
            }
            f.account = account.to_string();
            if let Some(o) = old.iter().find(|o| o.path == f.path) {
                if f.unread == 0 {
                    f.unread = o.unread;
                }
                if f.total == 0 {
                    f.total = o.total;
                }
                if f.uidvalidity == 0 {
                    f.uidvalidity = o.uidvalidity;
                }
            }
            list.push(f);
        }
        for gone in old.iter().filter(|o| !list.iter().any(|f| f.path == o.path)) {
            self.indexes.remove(&(account.to_string(), gone.path.clone()));
            remove_dir(&dir.join(folder_dir_name(&gone.path)))?;
        }
        sort_folders(&mut list);
        write_json(&dir.join("folders.json"), &list, true)?;
        self.folders.insert(account.to_string(), list);
        Ok(())
    }

    // ---------------------------------------------------------------- 邮件

    /// 文件夹里的全部邮件，按日期倒序（新的在前）。首次访问时从磁盘加载索引。
    pub fn messages(&mut self, account: &str, folder: &str) -> Result<Vec<MessageMeta>> {
        Ok(self.index(account, folder)?.messages.clone())
    }

    /// 本地已知的 uidvalidity 和 UID 列表（同步时告诉服务器端只取新的）。没有索引时为 (0, [])。
    pub fn known(&mut self, account: &str, folder: &str) -> Result<(u32, Vec<u32>)> {
        let idx = self.index(account, folder)?;
        Ok((idx.uidvalidity, idx.messages.iter().map(|m| m.key.uid).collect()))
    }

    /// 合并一次同步结果：`reset` 为 true（uidvalidity 变了）时先清空本地索引和原文；
    /// 加入/覆盖 `metas`（覆盖时保留已有的 `preview`，除非新的不为空）；按 `flags` 更新标记；
    /// 删除 `vanished` 里的 UID（连同原文）。之后重算文件夹的 `unread`/`total` 并写盘。
    pub fn apply_messages(
        &mut self,
        account: &str,
        folder: &str,
        uidvalidity: u32,
        metas: Vec<MessageMeta>,
        flags: Vec<(u32, Flags)>,
        vanished: Vec<u32>,
        reset: bool,
    ) -> Result<()> {
        let dir = self.folder_dir(account, folder)?;
        if reset {
            // UID 已经失效，旧原文对应不上新 UID，整个目录清掉
            remove_dir(&dir)?;
        }
        let idx = self.index(account, folder)?;
        if reset {
            idx.messages.clear();
        }
        idx.uidvalidity = uidvalidity;
        for mut m in metas {
            m.key = MsgKey::new(account, folder, m.key.uid);
            match idx.messages.iter_mut().find(|x| x.key.uid == m.key.uid) {
                Some(old) => {
                    // 头部同步拿不到正文；已下载过正文时保留预览和据正文算出的附件标记
                    if m.preview.is_empty() && !old.preview.is_empty() {
                        m.preview = std::mem::take(&mut old.preview);
                        m.has_attachments = old.has_attachments;
                    }
                    *old = m;
                }
                None => idx.messages.push(m),
            }
        }
        for (uid, f) in flags {
            if let Some(m) = idx.messages.iter_mut().find(|m| m.key.uid == uid) {
                m.flags = f;
            }
        }
        if !vanished.is_empty() {
            let gone: HashSet<u32> = vanished.into_iter().collect();
            idx.messages.retain(|m| !gone.contains(&m.key.uid));
            for uid in gone {
                remove_file(&dir.join(format!("{uid}.eml")))?;
            }
        }
        sort_messages(&mut idx.messages);
        self.save_index(account, folder)
    }

    /// 一封邮件的元数据。
    pub fn meta(&mut self, key: &MsgKey) -> Option<MessageMeta> {
        self.index(&key.account, &key.folder).ok()?.messages.iter().find(|m| m.key.uid == key.uid).cloned()
    }

    /// 修改一封邮件的元数据（例如标记已读、补预览），返回是否找到。会重算文件夹计数并写盘。
    pub fn update_meta(&mut self, key: &MsgKey, f: impl FnOnce(&mut MessageMeta)) -> Result<bool> {
        let idx = self.index(&key.account, &key.folder)?;
        let Some(m) = idx.messages.iter_mut().find(|m| m.key.uid == key.uid) else { return Ok(false) };
        f(m);
        // 键决定存放位置，不允许回调改掉
        m.key = key.clone();
        sort_messages(&mut idx.messages);
        self.save_index(&key.account, &key.folder)?;
        Ok(true)
    }

    /// 从本地删除这些邮件（元数据和原文）。
    pub fn remove(&mut self, keys: &[MsgKey]) -> Result<()> {
        let mut by_folder: HashMap<(&str, &str), HashSet<u32>> = HashMap::new();
        for k in keys {
            by_folder.entry((k.account.as_str(), k.folder.as_str())).or_default().insert(k.uid);
        }
        for ((account, folder), uids) in by_folder {
            let dir = self.folder_dir(account, folder)?;
            let idx = self.index(account, folder)?;
            idx.messages.retain(|m| !uids.contains(&m.key.uid));
            for uid in uids {
                remove_file(&dir.join(format!("{uid}.eml")))?;
            }
            self.save_index(account, folder)?;
        }
        Ok(())
    }

    /// 本地缓存的原文。
    pub fn raw(&self, key: &MsgKey) -> Option<Vec<u8>> {
        let dir = self.folder_dir(&key.account, &key.folder).ok()?;
        fs::read(dir.join(format!("{}.eml", key.uid))).ok()
    }

    /// 保存原文，并用 `mailfabrica_mime::meta_from_raw` 补全该邮件的 `preview` 和 `has_attachments`。
    pub fn put_raw(&mut self, key: &MsgKey, raw: &[u8]) -> Result<()> {
        let dir = self.folder_dir(&key.account, &key.folder)?;
        write_atomic(&dir.join(format!("{}.eml", key.uid)), raw)?;
        let idx = self.index(&key.account, &key.folder)?;
        // 索引里没有这封邮件（例如刚被删掉）时只存原文，不凭空造一条元数据
        let Some(m) = idx.messages.iter_mut().find(|m| m.key.uid == key.uid) else { return Ok(()) };
        let full = mailfabrica_mime::meta_from_raw(key.clone(), m.flags, m.size, raw);
        // 主题、发件人等以头部同步的结果为准，只补正文才算得出的字段
        m.preview = full.preview;
        m.has_attachments = full.has_attachments;
        if m.size == 0 {
            m.size = u32::try_from(raw.len()).unwrap_or(u32::MAX);
        }
        self.save_index(&key.account, &key.folder)
    }

    /// 在给定的 (账户, 文件夹) 范围内搜索：`query` 按空白拆词，每个词都要出现在
    /// 主题、发件人名/邮箱、收件人、抄送或预览里（大小写不敏感）。结果按日期倒序。
    /// 空查询返回空结果。
    pub fn search(&mut self, scope: &[(String, String)], query: &str) -> Vec<MessageMeta> {
        let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        if terms.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for (account, folder) in scope {
            let Ok(idx) = self.index(account, folder) else { continue };
            out.extend(
                idx.messages
                    .iter()
                    .filter(|m| {
                        let hay = haystack(m);
                        terms.iter().all(|t| hay.contains(t.as_str()))
                    })
                    .cloned(),
            );
        }
        out.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| b.key.cmp(&a.key)));
        out
    }

    // ---------------------------------------------------------------- 联系人

    /// 地址簿，按往来次数倒序、再按名字排序。
    pub fn contacts(&self) -> &[Contact] {
        &self.contacts
    }

    /// 记录一次往来：已有的 `count += 1` 并更新 `last` 和空名字；没有的新建。写盘。
    pub fn touch_contacts(&mut self, addrs: &[Address], now: i64) -> Result<()> {
        let mut seen: Vec<String> = Vec::new();
        for a in addrs.iter().filter(|a| a.is_valid()) {
            let email = a.email.trim();
            // 同一次往来里重复出现的地址只算一次
            if seen.iter().any(|s| s.eq_ignore_ascii_case(email)) {
                continue;
            }
            seen.push(email.to_string());
            let name = a.name.clone().unwrap_or_default();
            match self.contacts.iter_mut().find(|c| c.email.eq_ignore_ascii_case(email)) {
                Some(c) => {
                    c.count = c.count.saturating_add(1);
                    c.last = c.last.max(now);
                    if c.name.trim().is_empty() && !name.is_empty() {
                        c.name = name;
                    }
                }
                None => self.contacts.push(Contact { name, email: email.to_string(), count: 1, last: now, manual: false }),
            }
        }
        self.save_contacts()
    }

    /// 手动添加或修改联系人（`manual = true`）。
    pub fn upsert_contact(&mut self, contact: Contact) -> Result<()> {
        let email = contact.email.trim().to_string();
        if !Address::new(email.as_str()).is_valid() {
            return Err(MailError::Invalid(format!("contact email: {email}")));
        }
        let name = contact.name.trim().to_string();
        match self.contacts.iter_mut().find(|c| c.email.eq_ignore_ascii_case(&email)) {
            Some(c) => {
                c.name = name;
                c.email = email;
                c.manual = true;
                if contact.count > 0 {
                    c.count = contact.count;
                }
                if contact.last > 0 {
                    c.last = contact.last;
                }
            }
            None => self.contacts.push(Contact { name, email, manual: true, ..contact }),
        }
        self.save_contacts()
    }

    /// 删除联系人（邮箱大小写不敏感）。
    pub fn remove_contact(&mut self, email: &str) -> Result<()> {
        let email = email.trim();
        self.contacts.retain(|c| !c.email.eq_ignore_ascii_case(email));
        self.save_contacts()
    }

    // ---------------------------------------------------------------- 内部

    /// 账户目录；账户 id 不安全（可能跳出根目录）时报错。
    fn account_dir(&self, account: &str) -> Result<PathBuf> {
        account_dir(&self.root, account).ok_or_else(|| MailError::Invalid(format!("account id: {account:?}")))
    }

    /// 文件夹目录 `<root>/<account>/f-<hash>`。
    fn folder_dir(&self, account: &str, folder: &str) -> Result<PathBuf> {
        Ok(self.account_dir(account)?.join(folder_dir_name(folder)))
    }

    /// 取文件夹索引，首次访问时从磁盘加载并放进缓存。
    fn index(&mut self, account: &str, folder: &str) -> Result<&mut FolderIndex> {
        let path = self.folder_dir(account, folder)?.join("index.json");
        let idx = self.indexes.entry((account.to_string(), folder.to_string())).or_insert_with(|| {
            let mut idx: FolderIndex = read_json(&path);
            // 键以存放位置为准，防止目录被复制或改名后键对不上
            for m in &mut idx.messages {
                m.key.account = account.to_string();
                m.key.folder = folder.to_string();
            }
            sort_messages(&mut idx.messages);
            idx
        });
        Ok(idx)
    }

    /// 写索引，并把重算的 `unread`/`total`/`uidvalidity` 写回文件夹列表。
    fn save_index(&mut self, account: &str, folder: &str) -> Result<()> {
        let dir = self.folder_dir(account, folder)?;
        let Some(idx) = self.indexes.get(&(account.to_string(), folder.to_string())) else { return Ok(()) };
        write_json(&dir.join("index.json"), idx, false)?;
        let total = u32::try_from(idx.messages.len()).unwrap_or(u32::MAX);
        let unread = u32::try_from(idx.messages.iter().filter(|m| !m.flags.seen && !m.flags.deleted).count()).unwrap_or(u32::MAX);
        let uidvalidity = idx.uidvalidity;
        let mut list = self.folders(account);
        // 只更新已登记的文件夹，不凭索引造出文件夹列表里没有的项
        let Some(f) = list.iter_mut().find(|f| f.path == folder) else { return Ok(()) };
        if (f.unread, f.total, f.uidvalidity) == (unread, total, uidvalidity) {
            return Ok(());
        }
        (f.unread, f.total, f.uidvalidity) = (unread, total, uidvalidity);
        write_json(&self.account_dir(account)?.join("folders.json"), &list, true)?;
        self.folders.insert(account.to_string(), list);
        Ok(())
    }

    /// 写 `accounts.json`（AccountConfig 本身不含密码，密码在 SecretStore 里）。
    fn save_accounts(&self) -> Result<()> {
        write_json(&self.root.join("accounts.json"), &self.accounts, true)
    }

    /// 排序后写 `contacts.json`。
    fn save_contacts(&mut self) -> Result<()> {
        sort_contacts(&mut self.contacts);
        write_json(&self.root.join("contacts.json"), &self.contacts, true)
    }
}

/// 账户 id 只允许 `[A-Za-z0-9._-]` 且不以 `.` 开头，否则返回 None（防止 `../` 跳出根目录）。
fn account_dir(root: &Path, account: &str) -> Option<PathBuf> {
    let ok = !account.is_empty() && !account.starts_with('.') && account.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    ok.then(|| root.join(account))
}

/// 文件夹目录名：`f-` + 路径的 FNV-1a 64 位哈希。
/// 不用 std 的 DefaultHasher：它的算法不保证跨 Rust 版本稳定，换版本后会找不到旧数据。
fn folder_dir_name(path: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in path.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("f-{h:016x}")
}

/// 邮件排序：日期倒序，同一时间按 UID 倒序。
fn sort_messages(list: &mut [MessageMeta]) {
    list.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| b.key.uid.cmp(&a.key.uid)));
}

/// 文件夹排序：先按角色，再按路径。
fn sort_folders(list: &mut [Folder]) {
    list.sort_by(|a, b| a.role.order().cmp(&b.role.order()).then_with(|| a.path.cmp(&b.path)));
}

/// 联系人排序：往来次数倒序，再按名字（不分大小写）、邮箱。
fn sort_contacts(list: &mut [Contact]) {
    list.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())).then_with(|| a.email.cmp(&b.email)));
}

/// 搜索用的小写文本：主题、发件人、收件人、抄送、预览。
fn haystack(m: &MessageMeta) -> String {
    let mut s = String::with_capacity(m.subject.len() + m.preview.len() + 64);
    let addr = |s: &mut String, a: &Address| {
        s.push('\n');
        s.push_str(a.name.as_deref().unwrap_or_default());
        s.push(' ');
        s.push_str(&a.email);
    };
    s.push_str(&m.subject);
    addr(&mut s, &m.from);
    for a in m.to.iter().chain(&m.cc) {
        addr(&mut s, a);
    }
    s.push('\n');
    s.push_str(&m.preview);
    s.to_lowercase()
}

/// 读 JSON 文件。不存在时返回默认值；读失败或内容损坏时记警告、返回默认值，
/// 损坏的文件另存为 `*.bak`，避免下次写盘把它覆盖掉后无法找回。
fn read_json<T: DeserializeOwned + Default>(path: &Path) -> T {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return T::default(),
        Err(e) => {
            log::warn!("store: cannot read {}: {e}", path.display());
            return T::default();
        }
    };
    match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => {
            let bak = with_suffix(path, ".bak");
            log::warn!("store: corrupt {} ({e}); starting empty, copy kept at {}", path.display(), bak.display());
            if let Err(e) = fs::copy(path, &bak) {
                log::warn!("store: cannot back up {}: {e}", path.display());
            }
            T::default()
        }
    }
}

/// 把值写成 JSON（`pretty` 为 true 时带缩进，方便人看；索引文件用紧凑格式省空间）。
fn write_json<T: Serialize + ?Sized>(path: &Path, value: &T, pretty: bool) -> Result<()> {
    let bytes = if pretty { serde_json::to_vec_pretty(value) } else { serde_json::to_vec(value) };
    write_atomic(path, &bytes.map_err(|e| MailError::Io(e.to_string()))?)
}

/// 原子写：先写 `<name>.tmp` 并刷到磁盘，再 rename 覆盖目标。
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = with_suffix(path, ".tmp");
    let mut f = fs::File::create(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    fs::rename(&tmp, path)?;
    Ok(())
}

/// 在文件名后追加后缀，例如 `index.json` → `index.json.tmp`。
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// 删除文件，不存在不算错。
fn remove_file(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

/// 递归删除目录，不存在不算错。
fn remove_dir(path: &Path) -> Result<()> {
    match fs::remove_dir_all(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests;
