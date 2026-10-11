//! 邮件列表视图和对邮件的本地操作：范围 → 文件夹列表、刷新、选择、正文加载、
//! 乐观更新（先改本地再提交服务器任务）。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use mailfabrica_core::{Flag, Folder, FolderRole, MessageMeta, MsgKey};
use mailfabrica_sync::JobKind;

use crate::events::Action;
use crate::{BODY_CACHE_MAX, BodyState, EngineError, ListRow, Result, Scope, Session};

/// 统一范围（未读、红旗）里算不算这个文件夹：可打开，且不是已删除和垃圾邮件。
pub(crate) fn counts_as_mail(f: &Folder) -> bool {
    f.selectable && !matches!(f.role, FolderRole::Trash | FolderRole::Junk)
}

/// 把邮件键按 (账户, 文件夹) 分组，组内 UID 去重，顺序稳定。
pub(crate) fn group(keys: &[MsgKey]) -> BTreeMap<(String, String), Vec<u32>> {
    let mut out: BTreeMap<(String, String), Vec<u32>> = BTreeMap::new();
    for k in keys {
        let v = out.entry((k.account.clone(), k.folder.clone())).or_default();
        if !v.contains(&k.uid) {
            v.push(k.uid);
        }
    }
    out
}

/// 把按日期倒序的邮件排成列表行。`by_subject` 时归一化主题相同的邮件合成一行（空主题不归类），
/// 行的顺序跟着每组最新一封走；只有一封的组还是普通行。
pub(crate) fn build_rows(messages: &[MessageMeta], by_subject: bool) -> Vec<ListRow> {
    if !by_subject {
        return (0..messages.len()).map(|i| ListRow { members: vec![i], group: None }).collect();
    }
    let mut rows: Vec<ListRow> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for (i, m) in messages.iter().enumerate() {
        let key = mailfabrica_mime::subject_key(&m.subject);
        if key.is_empty() {
            rows.push(ListRow { members: vec![i], group: None });
            continue;
        }
        match index.get(&key).and_then(|r| rows.get_mut(*r)) {
            Some(row) => row.members.push(i),
            None => {
                index.insert(key.clone(), rows.len());
                rows.push(ListRow { members: vec![i], group: Some(key) });
            }
        }
    }
    for r in &mut rows {
        if r.members.len() < 2 {
            r.group = None;
        }
    }
    rows
}

impl Session {
    /// 某个范围对应的 (账户, 文件夹) 列表。`search` 为 true 时文件夹范围扩大到该账户全部文件夹。
    pub(crate) fn scope_folders(&self, scope: &Scope, search: bool) -> Vec<(String, String)> {
        match scope {
            Scope::Folder { account, folder } if !search => vec![(account.clone(), folder.clone())],
            Scope::Folder { account, .. } => {
                self.store.folders(account).into_iter().filter(|f| f.selectable).map(|f| (account.clone(), f.path)).collect()
            }
            Scope::AllInboxes => self
                .account_ids()
                .into_iter()
                .map(|a| {
                    let inbox = self.folder_path(&a, FolderRole::Inbox).unwrap_or_else(|| "INBOX".into());
                    (a, inbox)
                })
                .collect(),
            Scope::Unread | Scope::Flagged => self
                .account_ids()
                .into_iter()
                .flat_map(|a| self.store.folders(&a).into_iter().filter(counts_as_mail).map(move |f| (a.clone(), f.path)))
                .collect(),
        }
    }

    /// 视图是否包含这个文件夹（决定事件到达后要不要刷新列表）。
    pub(crate) fn view_covers(&self, account: &str, folder: &str) -> bool {
        match &self.view.scope {
            Scope::Folder { account: a, folder: f } => a == account && (f == folder || !self.view.query.is_empty()),
            _ => true,
        }
    }

    /// 按存储重建列表，并让选择和聚焦保持有效。
    pub(crate) fn refresh_view(&mut self) {
        let scope = self.view.scope.clone();
        let query = self.view.query.trim().to_string();
        let pairs = self.scope_folders(&scope, !query.is_empty());
        let mut list = if query.is_empty() {
            let mut v = Vec::new();
            for (a, f) in &pairs {
                v.extend(self.store.messages(a, f).unwrap_or_default());
            }
            v
        } else {
            self.store.search(&pairs, &query)
        };
        // 未读/红旗范围里，刚打开（因此变成已读）的那封先留在列表里，不然一点就消失
        let sticky: HashSet<&MsgKey> = self.view.selected.iter().chain(self.view.focused.as_ref()).collect();
        match scope {
            Scope::Unread => list.retain(|m| !m.flags.seen || sticky.contains(&m.key)),
            Scope::Flagged => list.retain(|m| m.flags.flagged || sticky.contains(&m.key)),
            _ => {}
        }
        list.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| b.key.cmp(&a.key)));
        let present: HashSet<&MsgKey> = list.iter().map(|m| &m.key).collect();
        self.view.selected.retain(|k| present.contains(k));
        if self.view.focused.as_ref().is_some_and(|k| !present.contains(k)) {
            self.view.focused = self.view.selected.last().cloned();
        }
        self.view.rows = build_rows(&list, self.prefs.group_by_subject);
        self.view.messages = list;
        self.sync_open_group();
    }

    /// 列表重建后让打开的主题组保持一致：组还在就把选择更新成组里现在的全部邮件（新到的同主题邮件也算）；
    /// 组没了（删到只剩一封、关掉归类）就关掉组视图，改为显示剩下的那封。
    fn sync_open_group(&mut self) {
        let Some(g) = self.view.group.clone() else { return };
        let keys: Vec<MsgKey> = self.group_messages(&g).into_iter().map(|m| m.key.clone()).collect();
        if keys.is_empty() {
            self.view.group = None;
            if self.view.focused.is_none() {
                self.view.focused = self.view.selected.first().cloned();
                self.view.selected = self.view.focused.iter().cloned().collect();
            }
        } else {
            self.view.selected = keys;
            self.view.focused = None;
        }
    }

    /// 打开一个主题组：选中组里全部邮件（最新的在前），不聚焦单封，阅读窗格显示这一组的列表。不标已读。
    pub(crate) fn select_group(&mut self, group: &str) -> Result<Vec<MsgKey>> {
        let keys: Vec<MsgKey> = self.group_messages(group).into_iter().map(|m| m.key.clone()).collect();
        if keys.is_empty() {
            return Err(EngineError::Other(format!("no subject group `{group}` in the list")));
        }
        self.view.group = Some(group.to_string());
        self.view.selected = keys.clone();
        self.view.focused = None;
        Ok(keys)
    }

    /// 当前所在的行：打开的主题组那一行，或者聚焦邮件所在的行。
    pub fn current_row(&self) -> Option<usize> {
        let rows = &self.view.rows;
        if let Some(g) = &self.view.group {
            return rows.iter().position(|r| r.group.as_ref() == Some(g));
        }
        let fk = self.view.focused.as_ref()?;
        let mi = self.view.messages.iter().position(|m| &m.key == fk)?;
        rows.iter().position(|r| r.members.contains(&mi))
    }

    /// 选中第 `idx` 行：主题组就打开组，单封邮件就选中并打开它。返回这一行最新一封的键和（如有）归类键。
    pub(crate) fn select_row(&mut self, idx: usize) -> Result<(MsgKey, Option<String>)> {
        let row = self.view.rows.get(idx).cloned().ok_or_else(|| EngineError::Other("the list is empty".into()))?;
        let key = row.members.first().and_then(|i| self.view.messages.get(*i)).map(|m| m.key.clone());
        let key = key.ok_or_else(|| EngineError::Other("the list is empty".into()))?;
        match &row.group {
            Some(g) => {
                self.select_group(g)?;
            }
            None => self.select_keys(vec![key.clone()], false, true),
        }
        Ok((key, row.group))
    }

    /// 选中这封邮件所在的行（在主题组里就打开整组，否则打开这封）。删除后移动焦点用。
    fn select_row_of(&mut self, key: &MsgKey) {
        match self.group_of(key).map(str::to_string) {
            Some(g) => {
                if let Err(e) = self.select_group(&g) {
                    log::warn!("select group: {e}");
                }
            }
            None => self.select_keys(vec![key.clone()], false, true),
        }
    }

    /// 选择邮件：`extend` 时切换这些邮件的选中状态，否则替换选择。聚焦最后一封并打开它。
    pub(crate) fn select_keys(&mut self, keys: Vec<MsgKey>, extend: bool, mark_read: bool) {
        // 选单封（或多选）就离开主题组视图
        self.view.group = None;
        if extend {
            for k in &keys {
                if let Some(i) = self.view.selected.iter().position(|s| s == k) {
                    self.view.selected.remove(i);
                } else {
                    self.view.selected.push(k.clone());
                }
            }
            self.view.focused = match keys.last() {
                Some(k) if self.view.selected.contains(k) => Some(k.clone()),
                _ => self.view.selected.last().cloned(),
            };
        } else {
            self.view.focused = keys.last().cloned();
            self.view.selected = keys;
        }
        if let Some(k) = self.view.focused.clone() {
            self.open_message(&k, mark_read);
        }
    }

    /// 打开一封邮件：确保正文在加载，按偏好标为已读。
    pub(crate) fn open_message(&mut self, key: &MsgKey, mark_read: bool) {
        self.ensure_body(key);
        if mark_read
            && self.prefs.mark_read_on_open
            && self.store.meta(key).is_some_and(|m| !m.flags.seen)
            && let Err(e) = self.set_flag_local(std::slice::from_ref(key), Flag::Seen, true)
        {
            self.status.text = e.to_string();
        }
    }

    /// 正文状态；本地有原文就解析进缓存，否则提交下载任务。
    pub(crate) fn ensure_body(&mut self, key: &MsgKey) -> BodyState {
        if let Some(BodyState::Ready(_) | BodyState::Loading) = self.bodies.get(key) {
            return self.body(key);
        }
        if let Some(raw) = self.store.raw(key) {
            self.cache_body(key, &raw);
            return self.body(key);
        }
        if self.store.meta(key).is_none() {
            return BodyState::Missing;
        }
        let kind = JobKind::FetchBody { folder: key.folder.clone(), uid: key.uid };
        match self.submit(&key.account, kind, Action::Body(key.clone())) {
            Ok(_) => {
                self.bodies.insert(key.clone(), BodyState::Loading);
            }
            Err(e) => {
                self.bodies.insert(key.clone(), BodyState::Failed(e.to_string()));
            }
        }
        self.body(key)
    }

    /// 解析原文放进正文缓存。缓存太大时只留下正在用的条目。
    pub(crate) fn cache_body(&mut self, key: &MsgKey, raw: &[u8]) {
        if self.bodies.len() >= BODY_CACHE_MAX {
            let focused = self.view.focused.clone();
            let waiting: HashSet<MsgKey> = self.awaiting.values().map(|a| a.key.clone()).collect();
            self.bodies.retain(|k, v| matches!(v, BodyState::Loading) || focused.as_ref() == Some(k) || waiting.contains(k));
        }
        self.bodies.insert(key.clone(), BodyState::Ready(Arc::new(mailfabrica_mime::parse(raw))));
    }

    /// 本地改标记（只改确实不同的），刷新视图，再提交服务器任务。返回改动的封数。
    pub(crate) fn set_flag_local(&mut self, keys: &[MsgKey], flag: Flag, on: bool) -> Result<usize> {
        let mut changed = Vec::new();
        for k in keys {
            if self.store.meta(k).is_some_and(|m| m.flags.get(flag) != on) {
                self.store.update_meta(k, |m| m.flags.set(flag, on))?;
                changed.push(k.clone());
            }
        }
        if changed.is_empty() {
            return Ok(0);
        }
        for m in self.view.messages.iter_mut().filter(|m| changed.contains(&m.key)) {
            m.flags.set(flag, on);
        }
        self.counts_dirty = true;
        let mut first_err = None;
        for ((account, folder), uids) in group(&changed) {
            let keys: Vec<MsgKey> = uids.iter().map(|u| MsgKey::new(account.clone(), folder.clone(), *u)).collect();
            let kind = JobKind::SetFlag { folder, uids, flag, on };
            if let Err(e) = self.submit(&account, kind, Action::Change { keys, resync: false }) {
                first_err.get_or_insert(e);
            }
        }
        match first_err {
            Some(e) => Err(e),
            None => Ok(changed.len()),
        }
    }

    /// 把邮件移到同账户的 `dest`（`None` = 彻底删除）。先从本地拿掉，焦点移到下一封，再提交服务器任务。
    pub(crate) fn move_local(&mut self, keys: &[MsgKey], dest: impl Fn(&str) -> Option<String>) -> Result<usize> {
        let mut plan = Vec::new();
        for ((account, folder), uids) in group(keys) {
            let target = dest(&account);
            if target.as_deref() == Some(folder.as_str()) {
                continue;
            }
            plan.push((account, folder, uids, target));
        }
        if plan.is_empty() {
            return Ok(0);
        }
        let moving: Vec<MsgKey> = plan.iter().flat_map(|(a, f, uids, _)| uids.iter().map(|u| MsgKey::new(a.clone(), f.clone(), *u))).collect();
        let next = self.next_after(&moving);
        // 打开的主题组整组被删/移走，也算丢了焦点
        let lost_focus = self.view.focused.as_ref().is_some_and(|k| moving.contains(k)) || (self.view.group.is_some() && self.view.focused.is_none());
        self.store.remove(&moving)?;
        for k in &moving {
            self.bodies.remove(k);
        }
        self.counts_dirty = true;
        let mut first_err = None;
        for (account, folder, uids, target) in plan {
            let keys: Vec<MsgKey> = uids.iter().map(|u| MsgKey::new(account.clone(), folder.clone(), *u)).collect();
            let (kind, resync) = match target {
                Some(dest) => (JobKind::Move { folder, uids, dest }, true),
                None => (JobKind::Delete { folder, uids }, false),
            };
            if let Err(e) = self.submit(&account, kind, Action::Change { keys, resync }) {
                first_err.get_or_insert(e);
            }
        }
        self.view.selected.retain(|k| !moving.contains(k));
        self.refresh_view();
        // 像 Foxmail 一样：删掉正在看的邮件后接着显示下一封
        if lost_focus
            && self.view.selected.is_empty()
            && let Some(n) = next
        {
            self.select_row_of(&n);
        }
        match first_err {
            Some(e) => Err(e),
            None => Ok(moving.len()),
        }
    }

    /// 列表里排在这些邮件所在行之后的第一行（没有就取之前最近的一行）里还留着的最新一封，用于删除后的焦点。
    /// 按行算：主题组算一行，删掉组里一封后先跳到下一行，最后才回到这个组剩下的邮件。
    fn next_after(&self, gone: &[MsgKey]) -> Option<MsgKey> {
        let gone: HashSet<&MsgKey> = gone.iter().collect();
        let msgs = &self.view.messages;
        let rows = &self.view.rows;
        let touched = |r: &ListRow| r.members.iter().any(|i| msgs.get(*i).is_some_and(|m| gone.contains(&m.key)));
        let alive = |r: &ListRow| r.members.iter().filter_map(|i| msgs.get(*i)).find(|m| !gone.contains(&m.key)).map(|m| m.key.clone());
        let last = rows.iter().rposition(touched)?;
        rows.iter().skip(last + 1).find_map(alive).or_else(|| rows.iter().take(last + 1).rev().find_map(alive))
    }

    /// 校验并定位删除/移动目标：同账户里某个角色的文件夹。
    pub(crate) fn role_folder(&self, account: &str, role: FolderRole) -> Result<String> {
        self.folder_path(account, role).ok_or_else(|| EngineError::Other(format!("account {account} has no {role:?} folder")))
    }
}
