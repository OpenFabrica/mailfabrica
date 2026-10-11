//! 同步期间让用户打开的正文插队，保留每账户单线程和同一条 IMAP 连接。

use std::collections::VecDeque;

use std::sync::mpsc::{Receiver, Sender};

use crate::{Event, Job, JobId, JobKind, MailError, MsgKey, Remote, Result, transient};

/// 工作线程拥有的任务队列；修改任务保持顺序，正文请求在同步检查点处理。
pub(crate) struct JobQueue {
    rx: Receiver<(JobId, Job)>,
    deferred: VecDeque<(JobId, Job)>,
}

impl JobQueue {
    /// 创建队列，不启动额外线程或连接。
    pub(crate) fn new(rx: Receiver<(JobId, Job)>) -> Self {
        Self { rx, deferred: VecDeque::new() }
    }

    /// 先执行同步期间留下的任务，再阻塞等新任务。
    pub(crate) fn next(&mut self) -> Option<(JobId, Job)> {
        self.deferred.pop_front().or_else(|| self.rx.recv().ok())
    }

    /// 检查点最多服务 16 个正文请求；不会越过移动、删除或配置验证等有顺序要求的任务。
    pub(crate) fn service_bodies(&mut self, remote: &mut dyn Remote, sync: &Job, events: &Sender<Event>) -> Result<Vec<MsgKey>> {
        self.deferred.extend(self.rx.try_iter());
        let mut downloaded = Vec::new();
        let mut serviced = 0;
        let mut index = 0;
        while index < self.deferred.len() && serviced < 16 {
            let Some((_, job)) = self.deferred.get(index) else { break };
            if job.account != sync.account || job.password != sync.password {
                break;
            }
            match &job.kind {
                // 标记是本地先更新的；读取正文可以越过它，修改操作本身仍按原顺序发送。
                JobKind::SetFlag { .. } => {
                    index += 1;
                    continue;
                }
                JobKind::FetchBody { .. } => {}
                _ => break,
            }
            let Some((id, job)) = self.deferred.remove(index) else { break };
            let JobKind::FetchBody { folder, uid } = &job.kind else { continue };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| remote.fetch_raw(folder, *uid)))
                .unwrap_or_else(|_| Err(MailError::Net("body download worker failed".into())));
            if let Some(e) = result.as_ref().err().filter(|e| transient(e)).cloned() {
                // 交给外层连接恢复逻辑重连；保留正文任务，不能把一次断线直接变成阅读失败。
                self.deferred.insert(index, (id, job));
                return Err(e);
            }
            let result = result.map(|raw| {
                let key = MsgKey::new(&job.account.id, folder, *uid);
                downloaded.push(key.clone());
                let _ = events.send(Event::Body { key, raw });
                String::new()
            });
            let _ = events.send(Event::Done { job: id, account: job.account.id, kind: job.kind.name(), result });
            serviced += 1;
        }
        Ok(downloaded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AccountConfig, Connector, FakeConnector};
    use std::collections::HashMap;
    use std::sync::mpsc;

    /// 构造同一账户的同步和正文任务，不触碰真实服务器。
    fn job(kind: JobKind) -> Job {
        Job { account: AccountConfig::new("me@example.com", "Me"), password: "pw".into(), kind }
    }

    /// 断线时正文任务保留，恢复后只完成一次；缺失 UID 则给明确失败而不阻断同步。
    #[test]
    fn failed_body_is_retried_after_reconnect_or_completed_as_missing() {
        let fake = FakeConnector::empty("pw");
        let uid = fake.deliver("INBOX", b"Subject: hello\r\n\r\nbody".to_vec());
        let sync = job(JobKind::Sync { known: HashMap::new(), limit: 500, prefetch: 0 });
        let mut remote = fake.connect(&sync.account, "pw").expect("remote");
        let (tx, rx) = mpsc::channel();
        let (events, received) = mpsc::channel();
        let mut queue = JobQueue::new(rx);
        tx.send((1, job(JobKind::FetchBody { folder: "INBOX".into(), uid }))).expect("queue body");
        fake.set_offline(true);
        assert!(queue.service_bodies(remote.as_mut(), &sync, &events).is_err());
        assert!(received.try_recv().is_err(), "transient failure must keep the body task pending");
        fake.set_offline(false);
        assert_eq!(queue.service_bodies(remote.as_mut(), &sync, &events).expect("recovered"), vec![MsgKey::new(&sync.account.id, "INBOX", uid)]);
        let done: Vec<Event> = received.try_iter().collect();
        assert_eq!(done.iter().filter(|e| matches!(e, Event::Body { .. })).count(), 1);
        assert_eq!(done.iter().filter(|e| matches!(e, Event::Done { job: 1, result: Ok(_), .. })).count(), 1);
        tx.send((2, job(JobKind::FetchBody { folder: "INBOX".into(), uid: 999 }))).expect("queue missing body");
        assert!(queue.service_bodies(remote.as_mut(), &sync, &events).expect("missing is nonfatal").is_empty());
        assert!(matches!(received.try_recv(), Ok(Event::Done { job: 2, result: Err(MailError::NotFound(_)), .. })));
        drop(tx);
        assert!(queue.next().is_none());
    }

    /// 删除任务构成顺序屏障，后面的正文不能提前读取一封待删除的邮件。
    #[test]
    fn body_does_not_overtake_delete() {
        let fake = FakeConnector::empty("pw");
        let sync = job(JobKind::Sync { known: HashMap::new(), limit: 500, prefetch: 0 });
        let mut remote = fake.connect(&sync.account, "pw").expect("remote");
        let (tx, rx) = mpsc::channel();
        let (events, received) = mpsc::channel();
        let mut queue = JobQueue::new(rx);
        tx.send((1, job(JobKind::Delete { folder: "INBOX".into(), uids: vec![1] }))).expect("delete");
        tx.send((2, job(JobKind::FetchBody { folder: "INBOX".into(), uid: 1 }))).expect("body");
        assert!(queue.service_bodies(remote.as_mut(), &sync, &events).expect("checkpoint").is_empty());
        assert!(received.try_recv().is_err());
        assert!(matches!(queue.next(), Some((1, Job { kind: JobKind::Delete { .. }, .. }))));
        assert!(matches!(queue.next(), Some((2, Job { kind: JobKind::FetchBody { .. }, .. }))));
    }
}
