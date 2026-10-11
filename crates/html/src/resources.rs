//! 跨预览复用远程资源的有界内存缓存；不写磁盘、不保存 Cookie、不缓存失败结果。

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use blitz_traits::net::Bytes;

/// 最多保留 32 MiB、128 个资源，五分钟后重新下载。
const MAX_BYTES: usize = 32 << 20;
const MAX_ENTRIES: usize = 128;
const TTL: Duration = Duration::from_secs(300);

/// 按最近使用顺序保存资源；每次查询和插入都清理过期项。
#[derive(Default)]
pub(crate) struct ResourceCache {
    entries: VecDeque<(String, Bytes, Instant)>,
    bytes: usize,
}

impl ResourceCache {
    /// 删除过期条目，更新时间不随查询延长，避免陈旧内容永久驻留。
    fn expire(&mut self, now: Instant) {
        self.entries.retain(|(_, data, at)| {
            if now.saturating_duration_since(*at) >= TTL {
                self.bytes = self.bytes.saturating_sub(data.len());
                false
            } else {
                true
            }
        });
    }

    /// 命中后移动到末尾，不复制资源数据。
    pub(crate) fn get(&mut self, url: &str, now: Instant) -> Option<Bytes> {
        self.expire(now);
        let index = self.entries.iter().position(|(key, _, _)| key == url)?;
        let item = self.entries.remove(index)?;
        let data = item.1.clone();
        self.entries.push_back(item);
        Some(data)
    }

    /// 仅保存非空且大小在限额内的资源，超额时淘汰最久未用的条目。
    pub(crate) fn insert(&mut self, url: String, data: Bytes, now: Instant) {
        self.expire(now);
        if data.is_empty() || data.len() > MAX_BYTES {
            return;
        }
        if let Some(index) = self.entries.iter().position(|(key, _, _)| key == &url)
            && let Some((_, old, _)) = self.entries.remove(index)
        {
            self.bytes = self.bytes.saturating_sub(old.len());
        }
        while self.bytes + data.len() > MAX_BYTES || self.entries.len() >= MAX_ENTRIES {
            let Some((_, old, _)) = self.entries.pop_front() else { break };
            self.bytes = self.bytes.saturating_sub(old.len());
        }
        self.bytes += data.len();
        self.entries.push_back((url, data, now));
    }
}

/// 缓存不会保留失败，受大小/数量约束，最近使用的资源优先保留且会按原始下载时间过期。
#[cfg(test)]
mod tests {
    use super::*;

    /// 通过可控时间验证 TTL、LRU 和资源数上限。
    #[test]
    fn bounded_cache_expires_and_keeps_recent_resources() {
        let now = Instant::now();
        let mut cache = ResourceCache::default();
        cache.insert("empty".into(), Bytes::new(), now);
        assert!(cache.get("empty", now).is_none());
        for i in 0..MAX_ENTRIES {
            cache.insert(i.to_string(), Bytes::from_static(b"logo"), now);
        }
        assert!(cache.get("0", now).is_some());
        cache.insert("next".into(), Bytes::from_static(b"logo"), now);
        assert!(cache.get("1", now).is_none());
        assert!(cache.get("0", now).is_some());
        assert!(cache.get("0", now + TTL).is_none());
        assert_eq!(cache.bytes, 0);
        cache.insert("large".into(), Bytes::from(vec![0; MAX_BYTES]), now);
        cache.insert("small".into(), Bytes::from_static(b"logo"), now);
        assert!(cache.get("large", now).is_none());
        assert_eq!(cache.bytes, 4);
    }
}
