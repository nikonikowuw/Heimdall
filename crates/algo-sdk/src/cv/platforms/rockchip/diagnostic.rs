//! RGA 预处理连续失败追踪机制
//!
//! 提供连续失败计数与阈值检测，供健康检查接口或监控体系读取。
//! 不主动推送告警——错误日志由 `engine.rs` 的 `tracing::error!` 负责。

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

/// 连续失败追踪配置
#[derive(Debug, Clone)]
pub struct DiagnosticConfig {
    /// 连续失败多少帧后视为异常状态
    pub failure_threshold: u64,
}

impl Default for DiagnosticConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 30, // 连续 30 帧失败（约 1 秒 @30fps）
        }
    }
}

/// 连续失败计数器
///
/// 纯计数器，不负责告警推送。调用方可通过 `status()` 读取状态，
/// 在健康检查接口或 Prometheus metrics 中暴露。
#[derive(Debug)]
pub struct FailureTracker {
    failure_threshold: u64,
    consecutive_failures: AtomicU64,
    total_failures: AtomicU64,
}

impl FailureTracker {
    /// 创建新的失败跟踪器
    pub fn new(config: DiagnosticConfig) -> Self {
        Self {
            failure_threshold: config.failure_threshold,
            consecutive_failures: AtomicU64::new(0),
            total_failures: AtomicU64::new(0),
        }
    }

    /// 记录处理失败
    pub fn record_failure(&self) {
        self.consecutive_failures.fetch_add(1, Ordering::Relaxed);
        self.total_failures.fetch_add(1, Ordering::Relaxed);
    }

    /// 记录处理成功（重置连续失败计数）
    pub fn record_success(&self) {
        let prev = self.consecutive_failures.swap(0, Ordering::Relaxed);
        if prev > 0 {
            tracing::info!(consecutive_failures = prev, "RGA 预处理恢复正常");
        }
    }

    /// 重置连续失败计数（不打日志）
    ///
    /// 用于 `flush()` 等非成功语义的重置场景，避免误打"恢复正常"日志。
    pub fn reset(&self) {
        self.consecutive_failures.store(0, Ordering::Relaxed);
    }

    /// 获取当前状态
    pub fn status(&self) -> FailureTrackerStatus {
        FailureTrackerStatus {
            consecutive_failures: self.consecutive_failures.load(Ordering::Relaxed),
            total_failures: self.total_failures.load(Ordering::Relaxed),
            last_failure_at: None, // 由调用方在需要时填充
        }
    }

    /// 当前连续失败次数是否达到阈值
    pub fn is_threshold_exceeded(&self) -> bool {
        self.consecutive_failures.load(Ordering::Relaxed) >= self.failure_threshold
    }

    /// 获取连续失败次数
    pub fn consecutive_failures(&self) -> u64 {
        self.consecutive_failures.load(Ordering::Relaxed)
    }

    /// 获取总失败次数
    pub fn total_failures(&self) -> u64 {
        self.total_failures.load(Ordering::Relaxed)
    }

    /// 获取失败阈值
    pub fn failure_threshold(&self) -> u64 {
        self.failure_threshold
    }
}

/// 失败跟踪器状态快照
#[derive(Debug, Clone)]
pub struct FailureTrackerStatus {
    pub consecutive_failures: u64,
    pub total_failures: u64,
    /// 最近一次失败的时间（由调用方按需填充）
    pub last_failure_at: Option<SystemTime>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_counting() {
        let tracker = FailureTracker::new(DiagnosticConfig {
            failure_threshold: 5,
        });

        for _ in 0..4 {
            tracker.record_failure();
        }
        assert_eq!(tracker.consecutive_failures(), 4);
        assert!(!tracker.is_threshold_exceeded());

        tracker.record_failure();
        assert_eq!(tracker.consecutive_failures(), 5);
        assert!(tracker.is_threshold_exceeded());
        assert_eq!(tracker.total_failures(), 5);
    }

    #[test]
    fn test_success_resets_consecutive() {
        let tracker = FailureTracker::new(DiagnosticConfig::default());

        for _ in 0..10 {
            tracker.record_failure();
        }
        assert_eq!(tracker.consecutive_failures(), 10);

        tracker.record_success();
        assert_eq!(tracker.consecutive_failures(), 0);
        assert_eq!(tracker.total_failures(), 10);
    }

    #[test]
    fn test_reset_without_log() {
        let tracker = FailureTracker::new(DiagnosticConfig::default());

        for _ in 0..5 {
            tracker.record_failure();
        }
        assert_eq!(tracker.consecutive_failures(), 5);

        // reset 只清零连续计数，不打日志
        tracker.reset();
        assert_eq!(tracker.consecutive_failures(), 0);
        assert_eq!(tracker.total_failures(), 5);
    }

    #[test]
    fn test_status_snapshot() {
        let tracker = FailureTracker::new(DiagnosticConfig::default());

        tracker.record_failure();
        tracker.record_failure();
        tracker.record_success();
        tracker.record_failure();

        let status = tracker.status();
        assert_eq!(status.consecutive_failures, 1);
        assert_eq!(status.total_failures, 3);
    }
}
