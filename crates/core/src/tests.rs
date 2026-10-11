use super::*;
use crate::date::{DateBucket, bucket, local_ts, now};

#[test]
fn address_parse_forms() {
    assert_eq!(Address::parse("a@b.com"), Some(Address::new("a@b.com")));
    assert_eq!(Address::parse("张三 <z@qq.com>"), Some(Address::named("张三", "z@qq.com")));
    assert_eq!(Address::parse("\"Doe, John\" <j@x.org>"), Some(Address::named("Doe, John", "j@x.org")));
    assert_eq!(Address::parse("  "), None);
}

#[test]
fn address_list_respects_quotes() {
    let l = Address::parse_list("\"Doe, John\" <j@x.org>, b@y.com；c@z.cn");
    assert_eq!(l.len(), 3);
    assert_eq!(l[0].name.as_deref(), Some("Doe, John"));
    assert_eq!(l[2].email, "c@z.cn");
    let back = Address::format_list(&l);
    assert_eq!(Address::parse_list(&back), l);
}

#[test]
fn address_validity_and_display() {
    assert!(Address::new("a@b.cn").is_valid());
    assert!(!Address::new("a@b").is_valid());
    assert!(!Address::new("a b@c.cn").is_valid());
    assert_eq!(Address::new("bob@x.com").display_name(), "bob");
}

#[test]
fn presets_and_slug() {
    let (i, s) = detect_servers("me@QQ.com");
    assert_eq!(i.host, "imap.qq.com");
    assert_eq!(s.port, 465);
    let (_, s) = detect_servers("me@outlook.com");
    assert_eq!(s.security, Security::StartTls);
    let (i, _) = detect_servers("me@example.org");
    assert_eq!(i.host, "imap.example.org");
    // 自建邮箱常见只有 mail.<域名>：备选里要有
    let (imaps, smtps) = server_candidates("mike@talkwork.vip");
    let hosts = |v: &[ServerConfig]| v.iter().map(|c| c.host.clone()).collect::<Vec<_>>();
    assert_eq!(hosts(&imaps), ["imap.talkwork.vip", "mail.talkwork.vip", "talkwork.vip"]);
    assert_eq!(hosts(&smtps), ["smtp.talkwork.vip", "mail.talkwork.vip", "talkwork.vip"]);
    assert_eq!(server_candidates("me@qq.com").0.len(), 1, "presets are not guessed");
    // 邮箱类型：id 唯一，最后一个是"其他"，企业邮箱带固定服务器
    let ids: std::collections::HashSet<_> = PROVIDERS.iter().map(|p| p.id).collect();
    assert_eq!(ids.len(), PROVIDERS.len());
    assert_eq!(PROVIDERS.last().map(|p| p.id), Some("other"));
    assert_eq!(provider("exmail").and_then(Provider::server_configs).map(|(i, _)| i.host), Some("imap.exmail.qq.com".to_string()));
    assert!(provider("qq").and_then(Provider::server_configs).is_none());
    assert_eq!(AccountConfig::slug_for("Alice@QQ.com"), "alice-qq-com");
}

#[test]
fn folder_roles() {
    assert_eq!(FolderRole::detect("INBOX", &[]), FolderRole::Inbox);
    assert_eq!(FolderRole::detect("Sent Messages", &[]), FolderRole::Sent);
    assert_eq!(FolderRole::detect("[Gmail]/All Mail", &["\\All".into()]), FolderRole::Archive);
    assert_eq!(FolderRole::detect("其他文件夹/垃圾邮件", &[]), FolderRole::Junk);
    assert_eq!(FolderRole::detect("whatever", &["\\Trash".into()]), FolderRole::Trash);
    let f = Folder::new("a", "Work/2024", Some('/'));
    assert_eq!((f.name(), f.depth()), ("2024", 1));
}

#[test]
fn msgkey_roundtrip() {
    let k = MsgKey::new("me-qq-com", "Work/项目", 42);
    assert_eq!(MsgKey::parse(&k.to_string()), Some(k));
    assert_eq!(MsgKey::parse("a/x/INBOX"), None);
}

#[test]
fn flags_roundtrip() {
    let f = Flags::from_imap(&["\\Seen", "\\FLAGGED", "$Junk"]);
    assert!(f.seen && f.flagged && !f.answered);
    assert_eq!(Flags::from_imap(&f.to_imap()), f);
}

#[test]
fn outgoing_recipients_dedup() {
    let o = Outgoing {
        to: vec![Address::new("a@x.com")],
        cc: vec![Address::new("A@x.com"), Address::new("b@x.com")],
        bcc: vec![Address::new("c@x.com")],
        ..Default::default()
    };
    assert_eq!(o.recipients(), vec!["a@x.com", "b@x.com", "c@x.com"]);
}

#[test]
fn date_buckets() {
    let n = now();
    assert_eq!(bucket(n, n), DateBucket::Today);
    assert_eq!(bucket(local_ts(1, 9, 0), n), DateBucket::Yesterday);
    assert_eq!(bucket(local_ts(400, 9, 0), n), DateBucket::Earlier);
}

#[test]
fn memory_secrets() {
    let s = MemorySecrets::new();
    assert_eq!(s.get("a"), None);
    s.set("a", "pw").ok();
    assert_eq!(s.get("a").as_deref(), Some("pw"));
    s.delete("a").ok();
    assert_eq!(s.get("a"), None);
}
