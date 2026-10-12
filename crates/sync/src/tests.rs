use super::*;

const WAIT: Duration = Duration::from_secs(10);

/// 测试用账户。
fn account() -> AccountConfig {
    AccountConfig::new("me@example.com", "Me")
}

/// 用正确密码构造任务。
fn job(kind: JobKind) -> Job {
    Job { account: account(), password: "pw".into(), kind }
}

/// 一封最小的原文。
fn raw(subject: &str) -> Vec<u8> {
    format!("From: a@b.com\r\nTo: me@example.com\r\nSubject: {subject}\r\nMessage-ID: <{subject}@b.com>\r\n\r\nbody of {subject}\r\n").into_bytes()
}

/// 全部 `Done` 事件的结果。
fn dones(events: &[Event]) -> Vec<(&'static str, std::result::Result<String, MailError>)> {
    events.iter().filter_map(|e| if let Event::Done { kind, result, .. } = e { Some((*kind, result.clone())) } else { None }).collect()
}

/// 同步任务（没有本地数据）。
fn sync_kind(known: HashMap<String, Known>) -> JobKind {
    JobKind::Sync { known, limit: 500, prefetch: 5 }
}

#[test]
fn sync_reports_folders_messages_and_bodies() {
    let fake = FakeConnector::empty("pw");
    fake.deliver("INBOX", raw("one"));
    fake.deliver("INBOX", raw("two"));
    let w = Worker::new(Arc::new(fake.clone()));
    w.submit(job(sync_kind(HashMap::new())));
    let evs = w.wait_idle(WAIT);
    assert!(w.busy_accounts().is_empty());
    let Some(Event::Folders { folders, .. }) = evs.iter().find(|e| matches!(e, Event::Folders { .. })) else { panic!("no folders") };
    assert_eq!(folders.len(), 5);
    assert!(folders.iter().any(|f| f.role == FolderRole::Trash && f.account == "me-example-com"));
    let metas: Vec<&MessageMeta> = evs
        .iter()
        .filter_map(|e| if let Event::Messages { folder, metas, .. } = e { (folder == "INBOX").then_some(metas) } else { None })
        .flatten()
        .collect();
    assert_eq!(metas.len(), 2);
    assert!(metas.iter().any(|m| m.subject == "two"));
    assert_eq!(evs.iter().filter(|e| matches!(e, Event::Body { .. })).count(), 2);
    assert_eq!(dones(&evs), vec![("sync", Ok("2 new".to_string()))]);
    assert!(evs.iter().any(|e| matches!(e, Event::Progress { progress: Progress::Done { new: 2 }, .. })));
    assert!(evs.iter().any(|e| matches!(e, Event::Progress { progress: Progress::Folder { index: 1, count: 5, .. }, .. })));
}

#[test]
fn incremental_sync_flags_vanished_and_reset() {
    let fake = FakeConnector::empty("pw");
    let u1 = fake.deliver("INBOX", raw("one"));
    let u2 = fake.deliver("INBOX", raw("two"));
    let w = Worker::new(Arc::new(fake.clone()));
    // 本地已知 u1、u2 和一封服务器上已删除的 99
    let mut known = HashMap::new();
    known.insert("INBOX".to_string(), Known { uidvalidity: 1001, uids: vec![u1, u2, 99] });
    let u3 = fake.deliver("INBOX", raw("three"));
    lock(&fake.state).folders[0].messages[0].1.seen = true;
    w.submit(job(sync_kind(known.clone())));
    let evs = w.wait_idle(WAIT);
    let inbox: Vec<&Event> = evs.iter().filter(|e| matches!(e, Event::Messages { folder, .. } if folder == "INBOX")).collect();
    let Some(Event::Messages { metas, flags, vanished, reset, .. }) = inbox.first() else { panic!("no inbox event") };
    assert_eq!(metas.iter().map(|m| m.key.uid).collect::<Vec<_>>(), vec![u3]);
    assert_eq!(vanished, &vec![99]);
    assert!(!reset);
    assert!(flags.iter().any(|(u, f)| *u == u1 && f.seen));
    // uidvalidity 变了：重置并重新取全部
    known.insert("INBOX".to_string(), Known { uidvalidity: 7, uids: vec![u1] });
    w.submit(job(sync_kind(known)));
    let evs = w.wait_idle(WAIT);
    let Some(Event::Messages { metas, reset, .. }) = evs.iter().find(|e| matches!(e, Event::Messages { folder, .. } if folder == "INBOX")) else {
        panic!("no inbox event")
    };
    assert!(reset);
    assert_eq!(metas.len(), 3);
}

#[test]
fn sync_limit_takes_newest() {
    let fake = FakeConnector::empty("pw");
    for i in 0..5 {
        fake.deliver("INBOX", raw(&format!("m{i}")));
    }
    let w = Worker::new(Arc::new(fake));
    w.submit(job(JobKind::Sync { known: HashMap::new(), limit: 2, prefetch: 0 }));
    let evs = w.wait_idle(WAIT);
    let mut uids: Vec<u32> = evs
        .iter()
        .filter_map(|e| if let Event::Messages { folder, metas, .. } = e { (folder == "INBOX").then_some(metas) } else { None })
        .flatten()
        .map(|m| m.key.uid)
        .collect();
    uids.sort_unstable();
    assert_eq!(uids, vec![4, 5]);
    assert!(!evs.iter().any(|e| matches!(e, Event::Body { .. })));
}

/// 服务器按 UID 升序返回邮件头时，有限的预取预算仍必须留给最新邮件。
#[test]
fn prefetch_uses_newest_messages_first() {
    let fake = FakeConnector::empty("pw");
    for i in 0..12 {
        fake.deliver("INBOX", raw(&format!("m{i}")));
    }
    let worker = Worker::new(Arc::new(fake));
    worker.submit(job(JobKind::Sync { known: HashMap::new(), limit: 500, prefetch: 3 }));
    let events = worker.wait_idle(WAIT);
    let bodies: Vec<u32> = events.iter().filter_map(|e| if let Event::Body { key, .. } = e { Some(key.uid) } else { None }).collect();
    assert_eq!(bodies, vec![12, 11, 10]);
}

#[test]
fn auth_failure_and_offline() {
    let fake = FakeConnector::empty("pw");
    let w = Worker::new(Arc::new(fake.clone()));
    w.submit(Job { account: account(), password: "wrong".into(), kind: JobKind::Test });
    let evs = w.wait_idle(WAIT);
    assert!(matches!(dones(&evs).as_slice(), [("test", Err(MailError::Auth(_)))]));
    fake.set_offline(true);
    w.submit(job(JobKind::Test));
    assert!(matches!(dones(&w.wait_idle(WAIT)).as_slice(), [("test", Err(MailError::Net(_)))]));
    fake.set_offline(false);
    w.submit(job(JobKind::Test));
    assert!(matches!(dones(&w.wait_idle(WAIT)).as_slice(), [("test", Ok(_))]));
}

/// 自动找服务器：猜的 imap./smtp. 主机不存在时换 mail.<域名>，并把找到的服务器报回去；
/// 密码错时不再往下试；全都连不上时保留每个候选的失败原因。
#[test]
fn detect_falls_back_to_the_next_candidate() {
    let fake = FakeConnector::empty("pw");
    fake.set_unreachable(&["imap.example.com", "smtp.example.com"]);
    let w = Worker::new(Arc::new(fake.clone()));
    let (imaps, smtps) = mailfabrica_core::server_candidates("me@example.com");
    let detect = JobKind::Detect { imap: imaps.clone(), smtp: smtps.clone() };
    w.submit(job(detect.clone()));
    let evs = w.wait_idle(WAIT);
    assert!(matches!(dones(&evs).as_slice(), [("detect", Ok(_))]), "{evs:?}");
    let found = evs.iter().find_map(|e| if let Event::Servers { imap, smtp, .. } = e { Some((imap.host.clone(), smtp.host.clone())) } else { None });
    assert_eq!(found, Some(("mail.example.com".into(), "mail.example.com".into())));
    // 第一个就能用：不发 Servers
    fake.set_unreachable(&[]);
    let mut acc = job(detect.clone());
    acc.account.imap = imaps[0].clone();
    acc.account.smtp = smtps[0].clone();
    w.submit(acc);
    let evs = w.wait_idle(WAIT);
    assert!(!evs.iter().any(|e| matches!(e, Event::Servers { .. })), "{evs:?}");
    // 密码错：登录被拒就停，不再试别的主机
    w.submit(Job { account: account(), password: "wrong".into(), kind: detect.clone() });
    assert!(matches!(dones(&w.wait_idle(WAIT)).as_slice(), [("detect", Err(MailError::Auth(_)))]));
    // 全都连不上
    fake.set_offline(true);
    w.submit(job(detect));
    assert!(
        matches!(dones(&w.wait_idle(WAIT)).as_slice(), [("detect", Err(MailError::Net(m)))] if m.contains("also tried mail.example.com:993, example.com:993") && m.contains("attempts:") && m.contains("imap.example.com:993:") && m.contains("mail.example.com:993:") && m.contains("example.com:993:"))
    );
}

#[test]
fn reconnects_once_after_connection_drop() {
    let fake = FakeConnector::empty("pw");
    let uid = fake.deliver("INBOX", raw("one"));
    let w = Worker::new(Arc::new(fake.clone()));
    w.submit(job(JobKind::Test));
    w.wait_idle(WAIT);
    // 断网后恢复：旧连接第一次操作失败（模拟），重连后成功
    struct Flaky {
        inner: FakeConnector,
        fails: Mutex<u32>,
    }
    impl Connector for Flaky {
        fn connect(&self, a: &AccountConfig, p: &str) -> Result<Box<dyn Remote>> {
            let r = self.inner.connect(a, p)?;
            let mut n = lock(&self.fails);
            let fail = *n > 0;
            *n = n.saturating_sub(1);
            Ok(Box::new(FlakyRemote { inner: r, fail }))
        }
        fn probe_smtp(&self, server: &ServerConfig) -> Result<()> {
            self.inner.probe_smtp(server)
        }
    }
    struct FlakyRemote {
        inner: Box<dyn Remote>,
        fail: bool,
    }
    impl Remote for FlakyRemote {
        fn list_folders(&mut self) -> Result<Vec<Folder>> {
            self.inner.list_folders()
        }
        fn folder_uids(&mut self, f: &str) -> Result<(u32, Vec<u32>)> {
            self.inner.folder_uids(f)
        }
        fn fetch_headers(&mut self, f: &str, u: &[u32]) -> Result<Vec<(u32, Flags, u32, Vec<u8>)>> {
            self.inner.fetch_headers(f, u)
        }
        fn fetch_flags(&mut self, f: &str, u: &[u32]) -> Result<Vec<(u32, Flags)>> {
            self.inner.fetch_flags(f, u)
        }
        fn fetch_raw(&mut self, f: &str, u: u32) -> Result<Vec<u8>> {
            if self.fail {
                return Err(MailError::Net("broken pipe".into()));
            }
            self.inner.fetch_raw(f, u)
        }
        fn set_flag(&mut self, f: &str, u: &[u32], fl: Flag, on: bool) -> Result<()> {
            self.inner.set_flag(f, u, fl, on)
        }
        fn move_to(&mut self, f: &str, u: &[u32], d: &str) -> Result<()> {
            self.inner.move_to(f, u, d)
        }
        fn delete(&mut self, f: &str, u: &[u32]) -> Result<()> {
            self.inner.delete(f, u)
        }
        fn append(&mut self, f: &str, fl: Flags, r: &[u8]) -> Result<()> {
            self.inner.append(f, fl, r)
        }
        fn create_folder(&mut self, f: &str) -> Result<()> {
            self.inner.create_folder(f)
        }
        fn send(&mut self, f: &str, r: &[String], raw: &[u8]) -> Result<()> {
            self.inner.send(f, r, raw)
        }
    }
    let w = Worker::new(Arc::new(Flaky { inner: fake, fails: Mutex::new(1) }));
    // 第一个任务建立（会失败的）连接并成功；第二个任务复用它，失败后重连重试
    w.submit(job(JobKind::Test));
    w.submit(job(JobKind::FetchBody { folder: "INBOX".into(), uid }));
    let evs = w.wait_idle(WAIT);
    assert!(dones(&evs).iter().all(|(_, r)| r.is_ok()), "{evs:?}");
    assert!(evs.iter().any(|e| matches!(e, Event::Body { .. })));
}

#[test]
fn fake_ops_roundtrip() {
    let fake = FakeConnector::empty("pw");
    let uid = fake.deliver("INBOX", raw("one"));
    let w = Worker::new(Arc::new(fake.clone()));
    w.submit(job(JobKind::SetFlag { folder: "INBOX".into(), uids: vec![uid], flag: Flag::Flagged, on: true }));
    w.wait_idle(WAIT);
    assert!(fake.list("INBOX")[0].1.flagged);
    w.submit(job(JobKind::Move { folder: "INBOX".into(), uids: vec![uid], dest: "Trash".into() }));
    w.wait_idle(WAIT);
    assert!(fake.list("INBOX").is_empty());
    let trash = fake.list("Trash");
    assert_eq!(trash.len(), 1);
    assert!(trash[0].1.flagged);
    w.submit(job(JobKind::Delete { folder: "Trash".into(), uids: vec![trash[0].0] }));
    w.submit(job(JobKind::Append { folder: "Drafts".into(), flags: Flags { draft: true, ..Flags::default() }, raw: raw("draft") }));
    w.submit(job(JobKind::CreateFolder { folder: "Work".into() }));
    w.submit(job(JobKind::Send { from: "me@example.com".into(), rcpts: vec!["x@y.com".into()], raw: raw("sent"), sent_folder: Some("Sent".into()) }));
    let evs = w.wait_idle(WAIT);
    assert_eq!(dones(&evs).len(), 4);
    assert!(fake.list("Trash").is_empty());
    assert_eq!(fake.list("Drafts").len(), 1);
    assert!(fake.list("Sent")[0].1.seen);
    assert_eq!(fake.sent_count(), 1);
    assert!(lock(&fake.state).folders.iter().any(|f| f.path == "Work"));
    // 不存在的文件夹：服务器错误，不 panic
    w.submit(job(JobKind::FetchBody { folder: "Nope".into(), uid: 1 }));
    assert!(matches!(dones(&w.wait_idle(WAIT)).as_slice(), [("fetchBody", Err(_))]));
}

#[test]
fn demo_mailbox_syncs() {
    let fake = FakeConnector::demo("demo@mailfabrica.dev", "demo");
    let w = Worker::new(Arc::new(fake));
    w.submit(Job { account: AccountConfig::new("demo@mailfabrica.dev", "Demo"), password: "demo".into(), kind: sync_kind(HashMap::new()) });
    let evs = w.wait_idle(WAIT);
    let n: usize = evs.iter().filter_map(|e| if let Event::Messages { metas, .. } = e { Some(metas.len()) } else { None }).sum();
    assert!(n >= 15, "{n}");
    assert!(
        evs.iter()
            .filter_map(|e| if let Event::Messages { metas, .. } = e { Some(metas) } else { None })
            .flatten()
            .all(|m| !m.subject.is_empty() && m.date > 0)
    );
}

#[test]
fn wait_idle_times_out_and_poll_is_nonblocking() {
    let w = Worker::new(Arc::new(FakeConnector::empty("pw")));
    assert!(w.poll().is_empty());
    assert!(w.wait_idle(Duration::from_millis(10)).is_empty());
    w.submit(job(JobKind::Test));
    assert_eq!(w.busy_accounts(), vec!["me-example-com".to_string()]);
    w.wait_idle(WAIT);
    assert!(w.busy_accounts().is_empty());
}

/// 打开正文只等当前网络请求，不能等全部文件夹同步和预取结束。
#[test]
fn opened_body_is_serviced_during_sync() {
    use std::sync::mpsc;
    struct SlowConnector {
        inner: FakeConnector,
        started: Sender<()>,
        release: Mutex<Option<Receiver<()>>>,
        headers_release: Mutex<Option<Receiver<()>>>,
    }
    struct SlowRemote {
        inner: Box<dyn Remote>,
        started: Sender<()>,
        release: Receiver<()>,
        headers_release: Receiver<()>,
    }
    impl Connector for SlowConnector {
        fn connect(&self, a: &AccountConfig, p: &str) -> Result<Box<dyn Remote>> {
            Ok(Box::new(SlowRemote {
                inner: self.inner.connect(a, p)?,
                started: self.started.clone(),
                release: lock(&self.release).take().expect("one connection"),
                headers_release: lock(&self.headers_release).take().expect("one connection"),
            }))
        }
        fn probe_smtp(&self, server: &ServerConfig) -> Result<()> {
            self.inner.probe_smtp(server)
        }
    }
    impl Remote for SlowRemote {
        fn list_folders(&mut self) -> Result<Vec<Folder>> {
            let _ = self.started.send(());
            let _ = self.release.recv_timeout(WAIT);
            self.inner.list_folders()
        }
        fn folder_uids(&mut self, f: &str) -> Result<(u32, Vec<u32>)> {
            self.inner.folder_uids(f)
        }
        fn fetch_headers(&mut self, f: &str, u: &[u32]) -> Result<Vec<(u32, Flags, u32, Vec<u8>)>> {
            let _ = self.headers_release.recv_timeout(WAIT);
            self.inner.fetch_headers(f, u)
        }
        fn fetch_flags(&mut self, f: &str, u: &[u32]) -> Result<Vec<(u32, Flags)>> {
            self.inner.fetch_flags(f, u)
        }
        fn fetch_raw(&mut self, f: &str, u: u32) -> Result<Vec<u8>> {
            self.inner.fetch_raw(f, u)
        }
        fn set_flag(&mut self, f: &str, u: &[u32], fl: Flag, on: bool) -> Result<()> {
            self.inner.set_flag(f, u, fl, on)
        }
        fn move_to(&mut self, f: &str, u: &[u32], d: &str) -> Result<()> {
            self.inner.move_to(f, u, d)
        }
        fn delete(&mut self, f: &str, u: &[u32]) -> Result<()> {
            self.inner.delete(f, u)
        }
        fn append(&mut self, f: &str, fl: Flags, r: &[u8]) -> Result<()> {
            self.inner.append(f, fl, r)
        }
        fn create_folder(&mut self, f: &str) -> Result<()> {
            self.inner.create_folder(f)
        }
        fn send(&mut self, f: &str, r: &[String], raw: &[u8]) -> Result<()> {
            self.inner.send(f, r, raw)
        }
    }
    let fake = FakeConnector::empty("pw");
    let uid = fake.deliver("INBOX", raw("open-me"));
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let (headers_tx, headers_rx) = mpsc::channel();
    let w = Worker::new(Arc::new(SlowConnector {
        inner: fake,
        started: started_tx,
        release: Mutex::new(Some(release_rx)),
        headers_release: Mutex::new(Some(headers_rx)),
    }));
    w.submit(job(JobKind::Sync { known: HashMap::new(), limit: 500, prefetch: 1 }));
    started_rx.recv_timeout(WAIT).expect("sync started");
    w.submit(job(JobKind::FetchBody { folder: "INBOX".into(), uid }));
    release_tx.send(()).expect("release folder list");
    let start = Instant::now();
    let mut events = Vec::new();
    while start.elapsed() < Duration::from_millis(500) {
        events.extend(w.poll());
        if events.iter().any(|e| matches!(e, Event::Body { key, .. } if key.uid == uid)) {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    let ready = events.iter().any(|e| matches!(e, Event::Body { key, .. } if key.uid == uid));
    let _ = headers_tx.send(());
    events.extend(w.wait_idle(WAIT));
    assert!(ready, "opened body waited behind the entire sync job");
    assert_eq!(
        events.iter().filter(|e| matches!(e, Event::Body { key, .. } if key.uid == uid)).count(),
        1,
        "opened body must not be prefetched again"
    );
    assert_eq!(dones(&events).iter().filter(|(kind, _)| *kind == "fetchBody").count(), 1);
    assert!(dones(&events).iter().all(|(_, r)| r.is_ok()));
}
