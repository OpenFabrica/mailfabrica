//! 处理后台工作线程发回的事件：写进存储、刷新视图、执行任务完成后的后续动作。

use std::collections::HashSet;

use mailfabrica_core::{AccountConfig, Address, Flag, MailError, MsgKey};
use mailfabrica_sync::{Event, Progress};

use crate::i18n::tr;
use crate::{BodyState, Session, UiRequest};

/// 任务完成后要做的事。
#[derive(Clone, Debug)]
pub(crate) enum Action {
    /// 只在失败时记错误。
    None,
    /// 收取：成功清掉账户错误，记录收取时间。
    Sync,
    /// 下载正文：失败时正文状态改为 `Failed`。
    Body(MsgKey),
    /// 添加账户的登录验证：成功后保存账户和密码并收取，失败时提示。
    AddAccount { account: AccountConfig, password: String },
    /// 发信：成功后关闭写信窗口、原邮件加已回复、记联系人、删旧草稿。
    Send { compose: u64, rcpts: Vec<Address>, answered: Option<MsgKey>, draft: Option<MsgKey> },
    /// 存草稿：成功后清掉"未保存"并收取（找回草稿的 UID）。
    Draft { compose: u64 },
    /// 改标记/移动/删除这些邮件：执行期间同步结果里这些邮件的旧数据要忽略；
    /// 失败时重新收取以恢复真实状态；`resync` 为 true 时成功后也收取（移动后目标文件夹要出现它）。
    Change { keys: Vec<MsgKey>, resync: bool },
    /// 新建文件夹：结束后收取文件夹列表。
    CreateFolder,
}

/// 添加账户失败时给用户的提示（按界面语言）：国内邮箱大多要用授权码登录。
pub(crate) fn add_account_hint(lang: &str, account: &AccountConfig, err: &MailError) -> String {
    let domain = account.address().domain();
    match err {
        MailError::Auth(_) => {
            let host = account.imap.host.trim().trim_end_matches('.').to_ascii_lowercase();
            if host == "qiye.163.com" || host.ends_with(".qiye.163.com") {
                return tr(lang, "hintNetEaseEnterprise", &[&account.email, err]);
            }
            let auth_code = ["qq.com", "foxmail.com", "vip.qq.com", "163.com", "126.com", "yeah.net", "sina.com", "sina.cn", "139.com", "aliyun.com"];
            if auth_code.contains(&domain.as_str()) {
                tr(lang, "hintAuthCode", &[&account.email, err, &domain])
            } else if domain == "gmail.com" || domain == "googlemail.com" {
                tr(lang, "hintGmail", &[&account.email, err])
            } else {
                tr(lang, "hintAuth", &[&account.email, err])
            }
        }
        MailError::Net(_) | MailError::Tls(_) | MailError::Io(_) => {
            tr(lang, "hintNet", &[&format!("{}:{}", account.imap.host, account.imap.port), err])
        }
        _ => tr(lang, "addFailed", &[&account.email, err]),
    }
}

impl Session {
    /// 处理一批事件。有任何变化时返回 true。
    pub(crate) fn apply_events(&mut self, events: Vec<Event>) -> bool {
        if events.is_empty() {
            return false;
        }
        let mut view_dirty = false;
        for ev in events {
            view_dirty |= self.apply_event(ev);
        }
        if view_dirty {
            self.refresh_view();
        }
        self.status.busy = !self.worker.busy_accounts().is_empty();
        self.recount();
        self.revision += 1;
        true
    }

    /// 处理一个事件，返回列表是否需要刷新。
    fn apply_event(&mut self, ev: Event) -> bool {
        match ev {
            Event::Folders { account, folders } => {
                if self.account(&account).is_none() {
                    return false;
                }
                if let Err(e) = self.store.set_folders(&account, folders) {
                    self.set_error(&account, e.to_string());
                }
                self.counts_dirty = true;
                // 当前文件夹在服务器上没了：退回到全部收件箱
                if let crate::Scope::Folder { account: a, folder } = &self.view.scope
                    && a == &account
                    && !self.store.folders(a).iter().any(|f| &f.path == folder)
                {
                    self.view.scope = crate::Scope::AllInboxes;
                    self.view.query.clear();
                }
                true
            }
            Event::Messages { account, folder, uidvalidity, mut metas, mut flags, vanished, reset } => {
                if self.account(&account).is_none() {
                    return false;
                }
                // 本地刚改过、服务器任务还没执行完的邮件：同步结果可能是旧的，先忽略
                let busy: HashSet<u32> = self
                    .pending
                    .values()
                    .filter_map(|(_, a)| if let Action::Change { keys, .. } = a { Some(keys) } else { None })
                    .flatten()
                    .filter(|k| k.account == account && k.folder == folder)
                    .map(|k| k.uid)
                    .collect();
                if !busy.is_empty() && !reset {
                    metas.retain(|m| !busy.contains(&m.key.uid));
                    flags.retain(|(u, _)| !busy.contains(u));
                }
                self.match_drafts(&account, &metas);
                if let Err(e) = self.store.apply_messages(&account, &folder, uidvalidity, metas, flags, vanished.clone(), reset) {
                    self.set_error(&account, e.to_string());
                }
                if reset {
                    self.bodies.retain(|k, _| !(k.account == account && k.folder == folder));
                } else {
                    for uid in vanished {
                        self.bodies.remove(&MsgKey::new(account.clone(), folder.clone(), uid));
                    }
                }
                self.counts_dirty = true;
                self.view_covers(&account, &folder)
            }
            Event::Body { key, raw } => {
                if self.account(&key.account).is_none() {
                    return false;
                }
                if let Err(e) = self.store.put_raw(&key, &raw) {
                    log::warn!("save body {key}: {e}");
                }
                let wanted = matches!(self.bodies.get(&key), Some(BodyState::Loading)) || self.view.focused.as_ref() == Some(&key);
                let waiting = self.awaiting.values().any(|a| a.key == key);
                if wanted || waiting {
                    self.cache_body(&key, &raw);
                }
                if waiting {
                    self.fill_waiting(&key);
                }
                // 只更新这一行的预览，不必重建整个列表
                if let Some(meta) = self.store.meta(&key)
                    && let Some(m) = self.view.messages.iter_mut().find(|m| m.key == key)
                {
                    *m = meta;
                }
                false
            }
            // 添加账户时自动找到的服务器：改掉还在验证中的账户配置，验证成功时按它保存
            Event::Servers { account, imap, smtp } => {
                for (_, action) in self.pending.values_mut() {
                    if let Action::AddAccount { account: acc, .. } = action
                        && acc.id == account
                    {
                        acc.imap = imap.clone();
                        acc.smtp = smtp.clone();
                    }
                }
                false
            }
            Event::Progress { account, progress } => {
                let text = match progress {
                    Progress::Folder { name, index, count } => self.tr("syncFolder", &[&name, &index, &count]),
                    Progress::Done { new } => self.tr("syncDone", &[&new]),
                };
                self.status.text = if self.store.accounts().len() > 1 { format!("{account}: {text}") } else { text };
                false
            }
            Event::Done { job, account, kind, result } => self.on_done(job, &account, kind, result),
        }
    }

    /// 草稿同步回来后，按 Message-ID 找回写信窗口对应的草稿 UID。
    fn match_drafts(&mut self, account: &str, metas: &[mailfabrica_core::MessageMeta]) {
        if self.draft_ids.is_empty() {
            return;
        }
        let mut found = Vec::new();
        for (cid, (acc, msgid)) in &self.draft_ids {
            if acc == account
                && let Some(m) = metas.iter().find(|m| &m.message_id == msgid)
            {
                found.push((*cid, m.key.clone()));
            }
        }
        for (cid, key) in found {
            self.draft_ids.remove(&cid);
            if let Some(c) = self.composes.iter_mut().find(|c| c.id == cid) {
                c.draft = Some(key);
            }
        }
    }

    /// 原邮件正文到了：给还没被用户改过的回复/转发/草稿窗口补上引用和附件。
    fn fill_waiting(&mut self, key: &MsgKey) {
        let ids: Vec<u64> = self.awaiting.iter().filter(|(_, a)| &a.key == key).map(|(id, _)| *id).collect();
        for id in ids {
            if let Some(a) = self.awaiting.remove(&id) {
                self.refill_compose(id, &a.to, &a.body, &a.doc);
            }
        }
    }

    /// 任务结束：按登记的后续动作处理。返回列表是否需要刷新。
    fn on_done(&mut self, job: u64, account: &str, kind: &str, result: std::result::Result<String, MailError>) -> bool {
        let action = self.pending.remove(&job).map(|(_, a)| a).unwrap_or(Action::None);
        match (action, result) {
            (Action::Sync, Ok(_)) => {
                self.status.errors.retain(|(a, _)| a != account);
                if !self.pending.values().any(|(_, a)| matches!(a, Action::Sync)) {
                    self.status.last_sync = mailfabrica_core::date::now();
                }
                self.resync_if_requested(account);
                false
            }
            (Action::Body(key), Err(e)) => {
                self.bodies.insert(key.clone(), BodyState::Failed(e.to_string()));
                // 正文拿不到了，等它的写信窗口就保持现状
                self.awaiting.retain(|_, a| a.key != key);
                false
            }
            (Action::Body(key), Ok(_)) => {
                // 下载成功但没收到原文（服务器上已没有这封）：不要一直显示"加载中"
                if matches!(self.bodies.get(&key), Some(BodyState::Loading)) {
                    self.bodies.insert(key, BodyState::Failed("message not found on the server".into()));
                }
                false
            }
            (Action::AddAccount { account: acc, password }, Ok(_)) => {
                self.finish_add_account(acc, password);
                true
            }
            (Action::AddAccount { account: acc, .. }, Err(e)) => {
                self.set_error(&acc.id, e.to_string());
                let servers = matches!(e, MailError::Net(_) | MailError::Tls(_) | MailError::Io(_));
                let message = add_account_hint(&self.prefs.language, &acc, &e);
                self.ui_requests.push(UiRequest::AccountAddFailed { id: acc.id.clone(), message, servers });
                false
            }
            (Action::Send { compose, rcpts, answered, draft }, Ok(summary)) => {
                self.composes.retain(|c| c.id != compose);
                self.awaiting.remove(&compose);
                self.draft_ids.remove(&compose);
                self.ui_requests.push(UiRequest::CloseCompose(compose));
                let sent = self.tr("sent", &[]);
                self.ui_requests.push(UiRequest::Toast(sent.clone()));
                // 工作线程的摘要是固定英文：`sent; saving a copy to … failed: <原因>`
                self.status.text = match summary.split_once("failed: ") {
                    Some((_, why)) => self.tr("sentNoCopy", &[&why]),
                    None => sent,
                };
                if let Some(src) = answered
                    && let Err(e) = self.set_flag_local(std::slice::from_ref(&src), Flag::Answered, true)
                {
                    log::warn!("mark answered: {e}");
                }
                if let Err(e) = self.store.touch_contacts(&rcpts, mailfabrica_core::date::now()) {
                    log::warn!("contacts: {e}");
                }
                if let Some(d) = draft
                    && let Err(e) = self.move_local(std::slice::from_ref(&d), |_| None)
                {
                    log::warn!("delete draft: {e}");
                }
                // 收取一次，让已发送里出现这封信
                self.sync_or_note(account);
                true
            }
            (Action::Send { compose, .. }, Err(e)) => {
                if let Some(c) = self.composes.iter_mut().find(|c| c.id == compose) {
                    c.sending = false;
                    c.error = Some(e.to_string());
                }
                self.status.text = self.tr("sendFailed", &[&e]);
                self.ui_requests.push(UiRequest::Toast(self.status.text.clone()));
                false
            }
            (Action::Draft { compose }, Ok(_)) => {
                if let Some(c) = self.composes.iter_mut().find(|c| c.id == compose) {
                    c.error = None;
                }
                self.ui_requests.push(UiRequest::Toast(self.tr("draftSaved", &[])));
                self.sync_or_note(account);
                false
            }
            (Action::Draft { compose }, Err(e)) => {
                if let Some(c) = self.composes.iter_mut().find(|c| c.id == compose) {
                    c.error = Some(e.to_string());
                    c.dirty = true;
                }
                self.draft_ids.remove(&compose);
                self.set_error(account, self.tr("draftFailed", &[&e]));
                false
            }
            (Action::Change { resync, .. }, Ok(_)) => {
                if resync {
                    self.sync_or_note(account);
                }
                false
            }
            (Action::Change { .. }, Err(e)) | (Action::CreateFolder, Err(e)) => {
                self.set_error(account, self.job_failed(kind, &e));
                // 本地已经乐观地改了，失败后收取一次，让本地回到服务器的真实状态
                self.sync_or_note(account);
                false
            }
            (Action::CreateFolder, Ok(_)) => {
                self.sync_or_note(account);
                false
            }
            (Action::Sync, Err(e)) => {
                self.set_error(account, e.to_string());
                self.resync_if_requested(account);
                false
            }
            (Action::None, Err(e)) => {
                self.set_error(account, self.job_failed(kind, &e));
                false
            }
            (Action::None, Ok(_)) => false,
        }
    }

    /// 同步结束后，如果期间又有人要求收取，再收一次。
    fn resync_if_requested(&mut self, account: &str) {
        if self.resync.remove(account) {
            self.sync_or_note(account);
        }
    }

    /// 收取账户；失败（例如账户已删除）只记录不报错。
    fn sync_or_note(&mut self, account: &str) {
        if self.account(account).is_some()
            && let Err(e) = self.request_sync(account)
        {
            self.set_error(account, e.to_string());
        }
    }

    /// 添加账户：保存配置和密码，开始收取。
    pub(crate) fn finish_add_account(&mut self, account: AccountConfig, password: String) {
        let id = account.id.clone();
        let email = account.email.clone();
        if let Err(e) = self.store.upsert_account(account) {
            self.set_error(&id, e.to_string());
            let message = self.tr("accountSaveFailed", &[&email, &e]);
            self.ui_requests.push(UiRequest::AccountAddFailed { id, message, servers: false });
            return;
        }
        if let Err(e) = self.secrets.set(&id, &password) {
            self.set_error(&id, self.tr("passwordSaveFailed", &[&e]));
            return;
        }
        self.status.errors.retain(|(a, _)| a != &id);
        self.ui_requests.push(UiRequest::AccountAdded(id.clone()));
        self.ui_requests.push(UiRequest::Toast(self.tr("accountAdded", &[&email])));
        self.sync_or_note(&id);
    }

    /// 按界面语言取一条文字。
    pub(crate) fn tr(&self, key: &str, args: &[&dyn std::fmt::Display]) -> String {
        tr(&self.prefs.language, key, args)
    }

    /// "某任务失败：原因"，任务名按界面语言。
    fn job_failed(&self, kind: &str, e: &MailError) -> String {
        let what = self.tr(&format!("job.{kind}"), &[]);
        self.tr("jobFailed", &[&what, e])
    }
}
