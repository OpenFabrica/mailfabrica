use mailfabrica_core::{AccountConfig, Address, Contact, Flags, Folder, FolderRole, MessageMeta, MsgKey};

use super::*;

const ACC: &str = "me-qq-com";

/// 测试用的邮件元数据。
fn meta(uid: u32, date: i64, subject: &str, seen: bool) -> MessageMeta {
    MessageMeta {
        key: MsgKey::new(ACC, "INBOX", uid),
        subject: subject.into(),
        from: Address::named("张三", "zhang@qq.com"),
        to: vec![Address::new("me@qq.com")],
        date,
        flags: Flags { seen, ..Default::default() },
        ..Default::default()
    }
}

/// 打开一个有账户和 INBOX/Sent/Work 文件夹的存储。
fn setup(root: &Path) -> Store {
    let mut s = Store::open(root).expect("open");
    s.upsert_account(AccountConfig::new("me@qq.com", "Me")).expect("account");
    let folders = vec![Folder::new(ACC, "Work/项目", Some('/')), Folder::new(ACC, "INBOX", Some('/')), Folder::new(ACC, "Sent Messages", Some('/'))];
    s.set_folders(ACC, folders).expect("folders");
    s
}

/// 某个文件夹的 (unread, total)。
fn counts(s: &Store, path: &str) -> (u32, u32) {
    s.folders(ACC).iter().find(|f| f.path == path).map(|f| (f.unread, f.total)).unwrap_or((99, 99))
}

#[test]
fn folder_dir_name_is_stable() {
    // FNV-1a 64 的已知值：空串是偏移基数
    assert_eq!(folder_dir_name(""), "f-cbf29ce484222325");
    assert_eq!(folder_dir_name("a"), "f-af63dc4c8601ec8c");
    assert_ne!(folder_dir_name("INBOX"), folder_dir_name("inbox"));
}

#[test]
fn accounts_and_folders_persist() {
    let dir = tempfile::tempdir().expect("tempdir");
    {
        let s = setup(dir.path());
        let roles: Vec<FolderRole> = s.folders(ACC).iter().map(|f| f.role).collect();
        assert_eq!(roles, vec![FolderRole::Inbox, FolderRole::Sent, FolderRole::Other]);
    }
    let s = Store::open(dir.path()).expect("reopen");
    assert_eq!(s.accounts().len(), 1);
    assert_eq!(s.accounts()[0].email, "me@qq.com");
    assert_eq!(s.folders(ACC).len(), 3);
    let json = std::fs::read_to_string(dir.path().join("accounts.json")).expect("accounts.json");
    assert!(!json.to_lowercase().contains("password"));
}

#[test]
fn invalid_account_id_is_rejected() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut s = Store::open(dir.path()).expect("open");
    let mut a = AccountConfig::new("x@y.com", "X");
    a.id = "../evil".into();
    assert!(s.upsert_account(a).is_err());
    assert!(s.messages("../evil", "INBOX").is_err());
    assert!(s.folders("../evil").is_empty());
}

#[test]
fn apply_messages_merge_reset_vanished() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut s = setup(dir.path());
    assert_eq!(s.known(ACC, "INBOX").expect("known"), (0, vec![]));

    s.apply_messages(
        ACC,
        "INBOX",
        7,
        vec![meta(1, 100, "old", true), meta(2, 300, "new", false), meta(3, 300, "same time", false)],
        vec![],
        vec![],
        false,
    )
    .expect("apply");
    let uids: Vec<u32> = s.messages(ACC, "INBOX").expect("messages").iter().map(|m| m.key.uid).collect();
    assert_eq!(uids, vec![3, 2, 1], "日期倒序，同日期 UID 倒序");
    assert_eq!(counts(&s, "INBOX"), (2, 3));
    assert_eq!(s.folders(ACC).iter().find(|f| f.path == "INBOX").map(|f| f.uidvalidity), Some(7));

    // 补正文预览后，再次同步头部不应丢掉预览
    s.update_meta(&MsgKey::new(ACC, "INBOX", 2), |m| {
        m.preview = "正文预览".into();
        m.has_attachments = true;
    })
    .expect("update");
    s.apply_messages(
        ACC,
        "INBOX",
        7,
        vec![meta(2, 300, "new (edited)", false)],
        vec![(1, Flags::default()), (3, Flags { seen: true, ..Default::default() })],
        vec![],
        false,
    )
    .expect("apply");
    let m2 = s.meta(&MsgKey::new(ACC, "INBOX", 2)).expect("m2");
    assert_eq!((m2.subject.as_str(), m2.preview.as_str(), m2.has_attachments), ("new (edited)", "正文预览", true));
    assert_eq!(counts(&s, "INBOX"), (2, 3), "uid1 变未读、uid3 变已读");

    // vanished 同时删掉原文
    let k1 = MsgKey::new(ACC, "INBOX", 1);
    s.put_raw(&k1, b"Subject: x\r\n\r\nhello\r\n").expect("put_raw");
    assert!(s.raw(&k1).is_some());
    s.apply_messages(ACC, "INBOX", 7, vec![], vec![], vec![1], false).expect("apply");
    assert!(s.raw(&k1).is_none());
    assert!(s.meta(&k1).is_none());
    assert_eq!(counts(&s, "INBOX"), (1, 2));

    // reset 清空旧索引和原文
    let k2 = MsgKey::new(ACC, "INBOX", 2);
    s.put_raw(&k2, b"Subject: x\r\n\r\nhello\r\n").expect("put_raw");
    s.apply_messages(ACC, "INBOX", 8, vec![meta(10, 50, "fresh", true)], vec![], vec![], true).expect("reset");
    assert_eq!(s.known(ACC, "INBOX").expect("known"), (8, vec![10]));
    assert!(s.raw(&k2).is_none());
    assert_eq!(counts(&s, "INBOX"), (0, 1));

    // 重新打开后索引和计数都还在
    drop(s);
    let mut s = Store::open(dir.path()).expect("reopen");
    assert_eq!(s.known(ACC, "INBOX").expect("known"), (8, vec![10]));
    assert_eq!(counts(&s, "INBOX"), (0, 1));
    assert_eq!(s.messages(ACC, "INBOX").expect("messages")[0].subject, "fresh");
}

#[test]
fn update_meta_and_remove() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut s = setup(dir.path());
    s.apply_messages(ACC, "INBOX", 1, vec![meta(1, 100, "a", false), meta(2, 200, "b", false)], vec![], vec![], false).expect("apply");
    assert_eq!(counts(&s, "INBOX"), (2, 2));
    let k1 = MsgKey::new(ACC, "INBOX", 1);
    assert!(s.update_meta(&k1, |m| m.flags.seen = true).expect("update"));
    assert!(!s.update_meta(&MsgKey::new(ACC, "INBOX", 99), |m| m.flags.seen = true).expect("update"));
    assert_eq!(counts(&s, "INBOX"), (1, 2));
    s.put_raw(&k1, b"Subject: a\r\n\r\nbody\r\n").expect("put_raw");
    s.remove(std::slice::from_ref(&k1)).expect("remove");
    assert!(s.meta(&k1).is_none());
    assert!(s.raw(&k1).is_none());
    assert_eq!(counts(&s, "INBOX"), (1, 1));
}

#[test]
fn put_raw_fills_preview_and_attachments() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut s = setup(dir.path());
    let mut header_meta = meta(5, 100, "来自头部的主题", false);
    header_meta.size = 4321;
    s.apply_messages(ACC, "INBOX", 1, vec![header_meta], vec![], vec![], false).expect("apply");
    let raw = "From: other@x.com\r\nSubject: body subject\r\nContent-Type: multipart/mixed; boundary=\"b\"\r\n\r\n\
               --b\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n你好，这是正文。\r\n> quoted\r\n\
               --b\r\nContent-Type: application/pdf\r\nContent-Disposition: attachment; filename=\"a.pdf\"\r\n\r\n%PDF\r\n--b--\r\n";
    let key = MsgKey::new(ACC, "INBOX", 5);
    s.put_raw(&key, raw.as_bytes()).expect("put_raw");
    assert_eq!(s.raw(&key).as_deref(), Some(raw.as_bytes()));
    let m = s.meta(&key).expect("meta");
    assert_eq!(m.preview, "你好，这是正文。");
    assert!(m.has_attachments);
    assert_eq!(m.subject, "来自头部的主题", "头部字段不被原文覆盖");
    assert_eq!(m.from.email, "zhang@qq.com");
    assert_eq!(m.size, 4321);
    // 不在索引里的邮件只存原文
    let ghost = MsgKey::new(ACC, "INBOX", 77);
    s.put_raw(&ghost, raw.as_bytes()).expect("put_raw");
    assert!(s.meta(&ghost).is_none());
    assert!(s.raw(&ghost).is_some());
    // 不留下临时文件
    let fdir = dir.path().join(ACC).join(folder_dir_name("INBOX"));
    let leftovers: Vec<_> = std::fs::read_dir(&fdir).expect("dir").flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".tmp")).collect();
    assert!(leftovers.is_empty());
}

#[test]
fn search_terms() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut s = setup(dir.path());
    let mut a = meta(1, 100, "Quarterly Report 季度报告", false);
    a.cc = vec![Address::named("Carol", "carol@corp.com")];
    let mut b = meta(2, 200, "Lunch", false);
    b.preview = "Let's discuss the REPORT tomorrow".into();
    let mut c = meta(3, 300, "Other", false);
    c.from = Address::new("bob@example.com");
    s.apply_messages(ACC, "INBOX", 1, vec![a, b, c], vec![], vec![], false).expect("apply");
    let mut w = meta(9, 400, "report from work", false);
    w.key.folder = "Work/项目".into();
    s.apply_messages(ACC, "Work/项目", 1, vec![w], vec![], vec![], false).expect("apply");

    let inbox = vec![(ACC.to_string(), "INBOX".to_string())];
    let uids = |r: Vec<MessageMeta>| r.iter().map(|m| m.key.uid).collect::<Vec<_>>();
    assert_eq!(uids(s.search(&inbox, "report")), vec![2, 1]);
    assert_eq!(uids(s.search(&inbox, "REPORT carol")), vec![1]);
    assert_eq!(uids(s.search(&inbox, "季度")), vec![1]);
    assert_eq!(uids(s.search(&inbox, "张三")), vec![2, 1]);
    assert_eq!(uids(s.search(&inbox, "bob@example")), vec![3]);
    assert_eq!(uids(s.search(&inbox, "report missing")), Vec::<u32>::new());
    assert!(s.search(&inbox, "   ").is_empty());
    let both = vec![(ACC.to_string(), "INBOX".to_string()), (ACC.to_string(), "Work/项目".to_string())];
    let r = s.search(&both, "report");
    assert_eq!(uids(r.clone()), vec![9, 2, 1]);
    assert_eq!(r[0].key.folder, "Work/项目");
}

#[test]
fn contacts_touch_upsert_remove() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut s = Store::open(dir.path()).expect("open");
    s.touch_contacts(&[Address::new("bob@x.com"), Address::named("Alice", "alice@x.com"), Address::new("not-an-address")], 100).expect("touch");
    s.touch_contacts(&[Address::named("Bob", "BOB@x.com"), Address::new("bob@x.com")], 200).expect("touch");
    let c = s.contacts();
    assert_eq!(c.len(), 2, "无效地址被跳过");
    assert_eq!((c[0].email.as_str(), c[0].name.as_str(), c[0].count, c[0].last), ("bob@x.com", "Bob", 2, 200));
    assert_eq!((c[1].name.as_str(), c[1].count), ("Alice", 1));

    s.upsert_contact(Contact { name: "Zed".into(), email: "zed@x.com".into(), ..Default::default() }).expect("upsert");
    s.upsert_contact(Contact { name: "Alice Smith".into(), email: "ALICE@x.com".into(), ..Default::default() }).expect("upsert");
    assert!(s.upsert_contact(Contact { email: "bad".into(), ..Default::default() }).is_err());
    let names: Vec<&str> = s.contacts().iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["Bob", "Alice Smith", "Zed"]);
    assert!(s.contacts().iter().all(|c| c.manual || c.email == "bob@x.com"));

    s.remove_contact("Bob@X.com").expect("remove");
    drop(s);
    let s = Store::open(dir.path()).expect("reopen");
    let emails: Vec<&str> = s.contacts().iter().map(|c| c.email.as_str()).collect();
    assert_eq!(emails, vec!["ALICE@x.com", "zed@x.com"], "手动修改可以改邮箱大小写");
}

#[test]
fn corrupt_files_are_recovered() {
    let dir = tempfile::tempdir().expect("tempdir");
    {
        let mut s = setup(dir.path());
        s.apply_messages(ACC, "INBOX", 1, vec![meta(1, 100, "a", false)], vec![], vec![], false).expect("apply");
        s.touch_contacts(&[Address::new("a@x.com")], 1).expect("touch");
    }
    let index = dir.path().join(ACC).join(folder_dir_name("INBOX")).join("index.json");
    std::fs::write(dir.path().join("contacts.json"), b"[{\"email\": tru").expect("write");
    std::fs::write(&index, b"{not json").expect("write");
    let mut s = Store::open(dir.path()).expect("open must not fail on corrupt files");
    assert!(s.contacts().is_empty());
    assert_eq!(s.accounts().len(), 1);
    assert!(s.messages(ACC, "INBOX").expect("messages").is_empty());
    assert_eq!(std::fs::read(dir.path().join("contacts.json.bak")).expect("bak"), b"[{\"email\": tru");
    assert_eq!(std::fs::read(with_suffix(&index, ".bak")).expect("bak"), b"{not json");
    // 损坏后还能继续写
    s.touch_contacts(&[Address::new("b@x.com")], 2).expect("touch");
    assert_eq!(s.contacts().len(), 1);
}

#[test]
fn folder_removal_deletes_data_and_keeps_counts() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut s = setup(dir.path());
    s.apply_messages(ACC, "Work/项目", 3, vec![meta(1, 100, "w", false)], vec![], vec![], false).expect("apply");
    s.apply_messages(ACC, "INBOX", 5, vec![meta(1, 100, "i", false)], vec![], vec![], false).expect("apply");
    let work_dir = dir.path().join(ACC).join(folder_dir_name("Work/项目"));
    assert!(work_dir.join("index.json").exists());

    // 服务器列表里没有计数：保留本地的 unread/total/uidvalidity；Work 消失则删数据
    s.set_folders(ACC, vec![Folder::new(ACC, "INBOX", Some('/')), Folder::new(ACC, "Sent Messages", Some('/'))]).expect("set_folders");
    let inbox = s.folders(ACC).into_iter().find(|f| f.path == "INBOX").expect("inbox");
    assert_eq!((inbox.unread, inbox.total, inbox.uidvalidity), (1, 1, 5));
    assert!(!work_dir.exists());
    assert!(s.messages(ACC, "Work/项目").expect("messages").is_empty());

    // 删除账户删除全部数据
    s.remove_account(ACC).expect("remove account");
    assert!(s.accounts().is_empty());
    assert!(!dir.path().join(ACC).exists());
    assert!(s.folders(ACC).is_empty());
}

/// 安装器只能写在更新缓存中；重复保存原子覆盖，路径穿越和其他格式被拒绝。
#[test]
fn installer_cache_is_atomic_and_confined() {
    let tmp = tempfile::tempdir().expect("temp");
    let store = Store::open(tmp.path()).expect("store");
    let name = "MailFabrica-0.2.0-arm64.dmg";
    let path = store.save_installer(name, b"old").expect("save");
    assert_eq!(path.parent(), Some(tmp.path().join("updates").as_path()));
    store.save_installer(name, b"new").expect("replace");
    assert_eq!(std::fs::read(&path).expect("read"), b"new");
    assert!(!with_suffix(&path, ".tmp").exists());
    for name in ["../file.dmg", "MailFabrica-../file.dmg", "MailFabrica.exe", "/tmp/MailFabrica.dmg"] {
        assert!(store.save_installer(name, b"bad").is_err());
    }
}
