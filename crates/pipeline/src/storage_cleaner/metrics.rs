//! 存储淘汰与孤儿对账运行指标看板 (Eviction Telemetry & Metrics)

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// 存储清理与自愈对账原子度量指标
#[derive(Debug, Default)]
pub struct EvictionMetrics {
    /// 触发清理巡检总轮次
    pub clean_cycles_total: AtomicU64,
    /// 成功淘汰抓拍记录总数
    pub captures_evicted_total: AtomicU64,
    /// 成功淘汰告警记录总数
    pub alarms_evicted_total: AtomicU64,
    /// 从 tombstone 彻底 unlink 释放的文件总数
    pub tombstone_reclaimed_total: AtomicU64,
    /// 扫描发现并隔离清理的孤儿物理文件总数
    pub orphan_files_detected_total: AtomicU64,
    /// 淘汰或对账时发现物理缺失的记录总数
    pub missing_files_detected_total: AtomicU64,
    /// 异步 unlink 触发的重试总次数
    pub unlink_retries_total: AtomicU64,
    /// 异步 unlink 重试超限失败总次数
    pub unlink_failures_total: AtomicU64,
    /// 当前重试队列堆积项数量
    pub retry_queue_size: AtomicUsize,
    /// 连续回滞排空循环执行的总子轮次
    pub drain_iterations_total: AtomicU64,
    /// 触发紧急严重水位淘汰的次数
    pub emergency_evictions_total: AtomicU64,
    /// 写盘断路器触发绝对阻断的次数
    pub circuit_breaker_tripped_total: AtomicU64,
    /// 普通抓拍被自适应降级抑制的次数
    pub normal_captures_throttled_total: AtomicU64,
}

impl EvictionMetrics {
    pub fn new() -> Self {
        Self::default()
    }

    /// 获取当前原子指标的不可变只读快照
    pub fn snapshot(&self) -> EvictionMetricsSnapshot {
        EvictionMetricsSnapshot {
            clean_cycles_total: self.clean_cycles_total.load(Ordering::Relaxed),
            captures_evicted_total: self.captures_evicted_total.load(Ordering::Relaxed),
            alarms_evicted_total: self.alarms_evicted_total.load(Ordering::Relaxed),
            tombstone_reclaimed_total: self.tombstone_reclaimed_total.load(Ordering::Relaxed),
            orphan_files_detected_total: self.orphan_files_detected_total.load(Ordering::Relaxed),
            missing_files_detected_total: self.missing_files_detected_total.load(Ordering::Relaxed),
            unlink_retries_total: self.unlink_retries_total.load(Ordering::Relaxed),
            unlink_failures_total: self.unlink_failures_total.load(Ordering::Relaxed),
            retry_queue_size: self.retry_queue_size.load(Ordering::Relaxed),
            drain_iterations_total: self.drain_iterations_total.load(Ordering::Relaxed),
            emergency_evictions_total: self.emergency_evictions_total.load(Ordering::Relaxed),
            circuit_breaker_tripped_total: self
                .circuit_breaker_tripped_total
                .load(Ordering::Relaxed),
            normal_captures_throttled_total: self
                .normal_captures_throttled_total
                .load(Ordering::Relaxed),
        }
    }
}

/// 存储淘汰指标只读快照 (可序列化用于 API 导出或系统监控)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvictionMetricsSnapshot {
    pub clean_cycles_total: u64,
    pub captures_evicted_total: u64,
    pub alarms_evicted_total: u64,
    pub tombstone_reclaimed_total: u64,
    pub orphan_files_detected_total: u64,
    pub missing_files_detected_total: u64,
    pub unlink_retries_total: u64,
    pub unlink_failures_total: u64,
    pub retry_queue_size: usize,
    pub drain_iterations_total: u64,
    pub emergency_evictions_total: u64,
    pub circuit_breaker_tripped_total: u64,
    pub normal_captures_throttled_total: u64,
}
