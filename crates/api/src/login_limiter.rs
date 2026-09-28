//! 登录失败限流：账号维度与来源 IP 维度双桶渐进退避。
//!
//! 为什么是双桶而不是单一维度：
//! - 只按 IP 限流会误伤：边缘设备常部署在 NAT 之后，整栋楼房的终端共用同一出口 IP，
//!   一个客户端输错几次密码就会把同网段其他终端一起锁在门外。
//! - 只按账号限流挡不住扫用户名：攻击者遍历用户名词典时每个账号只错一次，
//!   任何单账号阈值都不会触发。
//!
//! 两个桶各自独立判定，任一命中即拒绝，阈值刻意不对称：账号桶收紧（默认 5 次），
//! IP 桶放宽（默认 20 次）以容忍共享出口 IP 的正常误输。
//!
//! 内存有界是硬要求：键完全由攻击者可控（用户名 + IP），无上限的 HashMap 本身就是
//! 内存耗尽入口。超过 [`MAX_TRACKED_KEYS`] 时先按时间窗口清理，仍超限则淘汰最久未活动条目。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// 账号维度：连续失败达到该次数后开始退避。
const USERNAME_FAILURE_THRESHOLD: u32 = 5;
/// IP 维度：阈值放宽以容忍 NAT 后共用出口 IP 的多终端。
const IP_FAILURE_THRESHOLD: u32 = 20;
/// 首次触发退避时的锁定时长，此后按 2 的幂递增。
const BACKOFF_BASE: Duration = Duration::from_secs(1);
/// 单次锁定时长上限。
const MAX_BACKOFF: Duration = Duration::from_secs(300);
/// 失败计数窗口：距上次失败超过该时长则计数归零（也用于判定条目是否可清理）。
const FAILURE_WINDOW: Duration = Duration::from_secs(900);
/// 每个桶的追踪条目上限。
const MAX_TRACKED_KEYS: usize = 4096;
/// 指数退避的最大移位量，防止 `2u32.pow` 溢出与无意义的巨大数值。
const MAX_BACKOFF_SHIFT: u32 = 20;

/// 一次失败登记的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureOutcome {
    /// 已记录但尚未触发退避，附带累计失败次数。
    Counted { failures: u32 },
    /// 已进入退避，附带需等待的时长。
    Locked {
        failures: u32,
        retry_after: Duration,
    },
}

#[derive(Debug, Clone, Copy)]
struct Attempts {
    failures: u32,
    last_failure: Instant,
    locked_until: Option<Instant>,
}

/// 双桶登录失败限流器。
///
/// 所有判定方法都接收显式的 `now`，便于在测试中推进时钟而不依赖真实睡眠；
/// 面向调用方的 `check` / `record_failure` / `record_success` 内部取 [`Instant::now`]。
#[derive(Debug)]
pub struct LoginLimiter {
    usernames: Mutex<HashMap<String, Attempts>>,
    ips: Mutex<HashMap<String, Attempts>>,
    username_threshold: u32,
    ip_threshold: u32,
}

impl Default for LoginLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl LoginLimiter {
    pub fn new() -> Self {
        Self::with_thresholds(USERNAME_FAILURE_THRESHOLD, IP_FAILURE_THRESHOLD)
    }

    /// 以自定义阈值构造，供测试缩小或放大触发条件。
    pub fn with_thresholds(username_threshold: u32, ip_threshold: u32) -> Self {
        Self {
            usernames: Mutex::new(HashMap::new()),
            ips: Mutex::new(HashMap::new()),
            username_threshold: username_threshold.max(1),
            ip_threshold: ip_threshold.max(1),
        }
    }

    /// 判定当前账号与 IP 是否处于退避期；返回需等待的剩余时长。
    pub fn check(&self, username: &str, ip: &str) -> Option<Duration> {
        self.check_at(username, ip, Instant::now())
    }

    /// 登记一次失败；返回累计次数或新进入的退避时长。
    pub fn record_failure(&self, username: &str, ip: &str) -> FailureOutcome {
        self.record_failure_at(username, ip, Instant::now())
    }

    /// 登记一次成功：清除该账号的退避与失败计数。
    ///
    /// 刻意不清理 IP 桶：同一出口 IP 上的正常成功登录不应替攻击者的扫描行为洗白计数，
    /// IP 桶只能靠时间窗口自然衰减。
    pub fn record_success(&self, username: &str) {
        self.usernames
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(username);
    }

    /// 清空全部追踪状态，供测试隔离使用。
    pub fn clear(&self) {
        self.usernames
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        self.ips
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }

    pub fn check_at(&self, username: &str, ip: &str, now: Instant) -> Option<Duration> {
        let user_wait = {
            let mut guard = self
                .usernames
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard
                .get_mut(username)
                .and_then(|entry| take_remaining_lock(entry, now))
        };
        let ip_wait = {
            let mut guard = self
                .ips
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard
                .get_mut(ip)
                .and_then(|entry| take_remaining_lock(entry, now))
        };
        // 两桶可能同时锁定，取更长的一方，避免先解除的那个桶被攻击者用来试探。
        match (user_wait, ip_wait) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        }
    }

    pub fn record_failure_at(&self, username: &str, ip: &str, now: Instant) -> FailureOutcome {
        let username_outcome =
            self.register(&self.usernames, username, self.username_threshold, now);
        let ip_outcome = self.register(&self.ips, ip, self.ip_threshold, now);

        match (username_outcome, ip_outcome) {
            (
                FailureOutcome::Locked {
                    failures,
                    retry_after: user_wait,
                },
                FailureOutcome::Locked {
                    retry_after: ip_wait,
                    ..
                },
            ) => FailureOutcome::Locked {
                failures,
                retry_after: user_wait.max(ip_wait),
            },
            (
                FailureOutcome::Locked {
                    failures,
                    retry_after,
                },
                FailureOutcome::Counted { .. },
            )
            | (
                FailureOutcome::Counted { .. },
                FailureOutcome::Locked {
                    failures,
                    retry_after,
                },
            ) => FailureOutcome::Locked {
                failures,
                retry_after,
            },
            (FailureOutcome::Counted { failures }, FailureOutcome::Counted { .. }) => {
                FailureOutcome::Counted { failures }
            }
        }
    }

    fn register(
        &self,
        map: &Mutex<HashMap<String, Attempts>>,
        key: &str,
        threshold: u32,
        now: Instant,
    ) -> FailureOutcome {
        let mut guard = map.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

        if guard.len() >= MAX_TRACKED_KEYS && !guard.contains_key(key) {
            prune_expired(&mut guard, now);
            if guard.len() >= MAX_TRACKED_KEYS {
                evict_least_recent(&mut guard);
            }
        }

        let entry = guard.entry(key.to_string()).or_insert(Attempts {
            failures: 0,
            last_failure: now,
            locked_until: None,
        });

        // 窗口外视为新一轮：攻击者必须持续制造失败才能维持退避。
        if now.saturating_duration_since(entry.last_failure) > FAILURE_WINDOW {
            entry.failures = 0;
            entry.locked_until = None;
        }

        entry.failures = entry.failures.saturating_add(1);
        entry.last_failure = now;

        if entry.failures >= threshold {
            let retry_after = backoff_for(entry.failures - threshold);
            entry.locked_until = Some(now + retry_after);
            FailureOutcome::Locked {
                failures: entry.failures,
                retry_after,
            }
        } else {
            FailureOutcome::Counted {
                failures: entry.failures,
            }
        }
    }
}

/// 取出剩余锁定时长；已过期则顺手清除锁定标记。
fn take_remaining_lock(entry: &mut Attempts, now: Instant) -> Option<Duration> {
    match entry.locked_until {
        Some(until) if until > now => Some(until - now),
        Some(_) => {
            entry.locked_until = None;
            None
        }
        None => None,
    }
}

/// 指数退避：第 `step` 次超阈失败对应 `BACKOFF_BASE * 2^step`，封顶 [`MAX_BACKOFF`]。
fn backoff_for(step: u32) -> Duration {
    let shift = step.min(MAX_BACKOFF_SHIFT);
    (BACKOFF_BASE * 2u32.pow(shift)).min(MAX_BACKOFF)
}

/// 清理既超出失败窗口、又未处于退避期的条目。
fn prune_expired(map: &mut HashMap<String, Attempts>, now: Instant) {
    map.retain(|_, entry| {
        let within_window = now.saturating_duration_since(entry.last_failure) <= FAILURE_WINDOW;
        let still_locked = entry.locked_until.is_some_and(|until| until > now);
        within_window || still_locked
    });
}

/// 淘汰最久未活动的一个条目，保证插入后总量不超上限。
fn evict_least_recent(map: &mut HashMap<String, Attempts>) {
    if let Some(key) = map
        .iter()
        .min_by_key(|(_, entry)| entry.last_failure)
        .map(|(key, _)| key.clone())
    {
        map.remove(&key);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn below_threshold_counts_without_locking() {
        let limiter = LoginLimiter::with_thresholds(5, 20);
        let now = Instant::now();
        for expected in 1..5 {
            match limiter.record_failure_at("admin", "10.0.0.1", now) {
                FailureOutcome::Counted { failures } => assert_eq!(failures, expected),
                FailureOutcome::Locked { .. } => panic!("阈值前不应锁定"),
            }
            assert!(limiter.check_at("admin", "10.0.0.1", now).is_none());
        }
    }

    #[test]
    fn username_bucket_locks_at_threshold_with_exponential_backoff() {
        let limiter = LoginLimiter::with_thresholds(3, 100);
        let now = Instant::now();

        let mut waits = Vec::new();
        for _ in 0..5 {
            if let FailureOutcome::Locked { retry_after, .. } =
                limiter.record_failure_at("admin", "10.0.0.1", now)
            {
                waits.push(retry_after);
            }
        }

        // 第 3、4、5 次失败分别对应 1s / 2s / 4s
        assert_eq!(
            waits,
            vec![
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(4)
            ]
        );
    }

    #[test]
    fn backoff_is_capped() {
        assert_eq!(backoff_for(0), Duration::from_secs(1), "首次触发为基数");
        assert_eq!(backoff_for(30), MAX_BACKOFF, "高位应封顶而非溢出");
        assert_eq!(backoff_for(u32::MAX), MAX_BACKOFF);
    }

    #[test]
    fn lock_expires_and_allows_retry() {
        let limiter = LoginLimiter::with_thresholds(1, 100);
        let start = Instant::now();
        limiter.record_failure_at("admin", "10.0.0.1", start);

        assert!(limiter.check_at("admin", "10.0.0.1", start).is_some());
        let after = start + Duration::from_secs(2);
        assert!(
            limiter.check_at("admin", "10.0.0.1", after).is_none(),
            "退避结束后应放行"
        );
    }

    #[test]
    fn ip_bucket_threshold_is_independent_from_username() {
        let limiter = LoginLimiter::with_thresholds(5, 3);
        let now = Instant::now();

        // 每个账号只错一次，账号桶不触发；同一 IP 累计到阈值后由 IP 桶拦下
        for index in 0..2 {
            let outcome = limiter.record_failure_at(&format!("user{index}"), "10.0.0.1", now);
            assert!(matches!(outcome, FailureOutcome::Counted { .. }));
        }
        let outcome = limiter.record_failure_at("user2", "10.0.0.1", now);
        assert!(
            matches!(outcome, FailureOutcome::Locked { .. }),
            "扫用户名的行为必须由 IP 桶拦下"
        );
    }

    #[test]
    fn check_reports_longer_of_two_locks() {
        let limiter = LoginLimiter::with_thresholds(1, 1);
        let now = Instant::now();
        // 账号桶连续失败推高退避，IP 桶仅一次
        for _ in 0..4 {
            limiter.record_failure_at("admin", "10.0.0.1", now);
        }
        let wait = limiter.check_at("admin", "10.0.0.1", now).unwrap();
        assert!(
            wait > Duration::from_secs(1),
            "应取两桶中更长的退避，实际 {wait:?}"
        );
    }

    #[test]
    fn success_clears_username_bucket_only() {
        let limiter = LoginLimiter::with_thresholds(5, 5);
        let now = Instant::now();
        for _ in 0..4 {
            limiter.record_failure_at("admin", "10.0.0.1", now);
        }
        limiter.record_success("admin");

        assert!(
            limiter.check_at("admin", "10.0.0.1", now).is_none(),
            "成功登录后账号桶应清零"
        );

        // IP 桶保留：此时再错一次即达 IP 阈值
        let outcome = limiter.record_failure_at("other", "10.0.0.1", now);
        assert!(
            matches!(outcome, FailureOutcome::Locked { .. }),
            "IP 桶计数不应被成功登录洗白"
        );
    }

    #[test]
    fn failure_window_resets_stale_counts() {
        let limiter = LoginLimiter::with_thresholds(3, 100);
        let start = Instant::now();
        limiter.record_failure_at("admin", "10.0.0.1", start);
        limiter.record_failure_at("admin", "10.0.0.1", start);

        let later = start + FAILURE_WINDOW + Duration::from_secs(1);
        match limiter.record_failure_at("admin", "10.0.0.1", later) {
            FailureOutcome::Counted { failures } => assert_eq!(failures, 1, "窗口外应重新计数"),
            FailureOutcome::Locked { .. } => panic!("窗口外不应继承旧计数"),
        }
    }

    #[test]
    fn tracked_keys_stay_bounded_and_evict_oldest() {
        let limiter = LoginLimiter::with_thresholds(5, 1000);
        let start = Instant::now();

        for index in 0..(MAX_TRACKED_KEYS + 64) {
            limiter.record_failure_at(
                &format!("attacker{index}"),
                "10.0.0.1",
                start + Duration::from_millis(index as u64),
            );
        }

        let tracked = limiter.usernames.lock().unwrap().len();
        assert!(
            tracked <= MAX_TRACKED_KEYS,
            "随机用户名不得撑爆内存，实际 {tracked}"
        );
    }

    #[test]
    fn prune_drops_expired_entries_before_eviction() {
        let mut map: HashMap<String, Attempts> = HashMap::new();
        let now = Instant::now();
        map.insert(
            "stale".to_string(),
            Attempts {
                failures: 1,
                last_failure: now - FAILURE_WINDOW - Duration::from_secs(1),
                locked_until: None,
            },
        );
        map.insert(
            "locked".to_string(),
            Attempts {
                failures: 9,
                last_failure: now - FAILURE_WINDOW - Duration::from_secs(1),
                locked_until: Some(now + Duration::from_secs(30)),
            },
        );

        prune_expired(&mut map, now);

        assert!(!map.contains_key("stale"), "过期且未锁定的条目应被清理");
        assert!(
            map.contains_key("locked"),
            "仍在退避期的条目不得被清理，否则等于提前放行"
        );
    }
}
