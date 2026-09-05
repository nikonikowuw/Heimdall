//! 分级存储护盾状态机与写盘断路器 (Multi-tier Storage Guard & Circuit Breaker)
//!
//! 根据物理剩余字节、Inode 水位与只读标志动态计算当前系统的存储健康等级与写盘放行决策：
//! 1. **Normal**: 空间充足，放行所有抓拍与告警；
//! 2. **Evicting**: 触发淘汰水线，回滞加速淘汰中；
//! 3. **Emergency**: 空间紧急紧缺，降级截断普通抓拍，仅放行高危告警，自适应提速巡检；
//! 4. **Critical**: 物理剩余 < 5% 或 Inode < 2% 或只读，全域硬断路熔断，保全核心系统 WAL。

use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::fs_stat::FsStorageStat;

/// 存储健康等级
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum StorageHealthLevel {
    /// 正常健康 (>= 15% 空间且 >= 10% Inode)
    Normal = 0,
    /// 回滞淘汰中 (< 15% 空间或 < 10% Inode，但在安全线内)
    Evicting = 1,
    /// 紧急水位 (< 8% 空间或 < 5% Inode)
    Emergency = 2,
    /// 临界熔断 (< 5% 空间或 < 2% Inode 或只读)
    Critical = 3,
}

/// 存储守卫决策结果
#[derive(Debug, Clone, PartialEq)]
pub struct StorageDecision {
    /// 当前评估的健康等级
    pub level: StorageHealthLevel,
    /// 是否允许写入普通抓拍 (Captures)
    pub allow_normal_capture: bool,
    /// 是否允许写入关键告警大图 (Critical Alarms)
    pub allow_critical_alarm: bool,
    /// 建议的下次巡检时间间隔 (自适应加速)
    pub suggested_poll_interval: Duration,
    /// 是否需要继续触发淘汰
    pub should_evict: bool,
    /// 是否发生硬断路熔断
    pub is_circuit_broken: bool,
    /// 阻断或降级的原因说明
    pub reason: Option<String>,
}

/// 存储水位与断路保护阈值配置
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StorageWatermarkThresholds {
    /// 触发淘汰的高水位阈值（默认 0.15 即 15%）
    pub trigger_free_ratio: f64,
    /// 紧急严重水位阈值（默认 0.08 即 8%）
    pub emergency_free_ratio: f64,
    /// 临界写保护硬熔断阈值（默认 0.05 即 5%）
    pub critical_free_ratio: f64,
    /// Inode 触发淘汰阈值（默认 0.10 即 10%）
    pub min_inode_free_ratio: f64,
    /// Inode 临界硬熔断阈值（默认 0.02 即 2%）
    pub critical_inode_free_ratio: f64,
}

impl Default for StorageWatermarkThresholds {
    fn default() -> Self {
        Self {
            trigger_free_ratio: 0.15,
            emergency_free_ratio: 0.08,
            critical_free_ratio: 0.05,
            min_inode_free_ratio: 0.10,
            critical_inode_free_ratio: 0.02,
        }
    }
}

/// 存储断路与决策器
#[derive(Debug, Clone)]
pub struct StorageCircuitBreaker;

impl StorageCircuitBreaker {
    /// 综合评估当前文件系统状态并输出控制决策
    pub fn evaluate(
        stat: &FsStorageStat,
        thresholds: &StorageWatermarkThresholds,
    ) -> StorageDecision {
        // 1. 致命判定：若底层文件系统已被内核置为只读挂载 (硬件坏块或文件系统损坏)
        if stat.is_read_only {
            return StorageDecision {
                level: StorageHealthLevel::Critical,
                allow_normal_capture: false,
                allow_critical_alarm: false,
                suggested_poll_interval: Duration::from_secs(5),
                should_evict: false, // 只读下无法执行 unlink/rename
                is_circuit_broken: true,
                reason: Some("底层存储处于只读挂载状态 (Read-Only FS)，已硬熔断所有写盘".into()),
            };
        }

        // 2. 致命判定：剩余空间低于临界红线 (5%) 或 Inode 几乎耗尽 (< 2%)
        if stat.free_ratio < thresholds.critical_free_ratio
            || stat.inode_free_ratio < thresholds.critical_inode_free_ratio
        {
            let reason = if stat.free_ratio < thresholds.critical_free_ratio {
                format!(
                    "磁盘物理剩余空间低于临界红线 ({:.2}% < {:.2}%)",
                    stat.free_ratio * 100.0,
                    thresholds.critical_free_ratio * 100.0
                )
            } else {
                format!(
                    "Inode 可用节点严重耗尽 ({:.2}% < {:.2}%)",
                    stat.inode_free_ratio * 100.0,
                    thresholds.critical_inode_free_ratio * 100.0
                )
            };
            return StorageDecision {
                level: StorageHealthLevel::Critical,
                allow_normal_capture: false,
                allow_critical_alarm: false,
                suggested_poll_interval: Duration::from_secs(5),
                should_evict: true,
                is_circuit_broken: true,
                reason: Some(reason),
            };
        }

        // 3. 紧急判定：剩余空间低于紧急水位 (8%) 或 Inode 低于 5%
        if stat.free_ratio < thresholds.emergency_free_ratio
            || stat.inode_free_ratio < (thresholds.critical_inode_free_ratio * 2.0)
        {
            return StorageDecision {
                level: StorageHealthLevel::Emergency,
                allow_normal_capture: false, // 冻结普通抓拍！仅放行违规告警
                allow_critical_alarm: true,
                suggested_poll_interval: Duration::from_secs(10), // 自适应加速至 10 秒巡检
                should_evict: true,
                is_circuit_broken: false,
                reason: Some(format!(
                    "存储处于紧急水位 ({:.2}%)，已自动降级熔断普通抓拍，仅保全告警",
                    stat.free_ratio * 100.0
                )),
            };
        }

        // 4. 回滞淘汰中：低于触发水位 (15%) 或 Inode 低于 10%
        if stat.free_ratio < thresholds.trigger_free_ratio
            || stat.inode_free_ratio < thresholds.min_inode_free_ratio
        {
            return StorageDecision {
                level: StorageHealthLevel::Evicting,
                allow_normal_capture: true,
                allow_critical_alarm: true,
                suggested_poll_interval: Duration::from_secs(30), // 加速至 30 秒巡检
                should_evict: true,
                is_circuit_broken: false,
                reason: None,
            };
        }

        // 5. 空间充足健康状态
        StorageDecision {
            level: StorageHealthLevel::Normal,
            allow_normal_capture: true,
            allow_critical_alarm: true,
            suggested_poll_interval: Duration::from_secs(300), // 默认 5 分钟
            should_evict: false,
            is_circuit_broken: false,
            reason: None,
        }
    }

    /// 使用默认水位阈值评估当前文件系统状态
    pub fn evaluate_with_defaults(stat: &FsStorageStat) -> StorageDecision {
        Self::evaluate(stat, &StorageWatermarkThresholds::default())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn make_stat(free_ratio: f64, inode_free_ratio: f64, is_read_only: bool) -> FsStorageStat {
        FsStorageStat {
            fragment_size: 4096,
            total_bytes: 100_000_000_000,
            available_bytes: (100_000_000_000.0 * free_ratio) as u64,
            free_ratio,
            total_inodes: 1_000_000,
            available_inodes: (1_000_000.0 * inode_free_ratio) as u64,
            inode_free_ratio,
            is_read_only,
        }
    }

    #[test]
    fn test_storage_circuit_breaker_decisions() {
        let thresholds = StorageWatermarkThresholds::default();

        // 1. 只读测试 -> 必须硬熔断
        let ro_stat = make_stat(0.50, 0.50, true);
        let dec_ro = StorageCircuitBreaker::evaluate(&ro_stat, &thresholds);
        assert_eq!(dec_ro.level, StorageHealthLevel::Critical);
        assert!(!dec_ro.allow_normal_capture);
        assert!(!dec_ro.allow_critical_alarm);
        assert!(dec_ro.is_circuit_broken);

        // 2. 空间 < 5% -> 必须硬熔断
        let crit_stat = make_stat(0.04, 0.50, false);
        let dec_crit = StorageCircuitBreaker::evaluate(&crit_stat, &thresholds);
        assert_eq!(dec_crit.level, StorageHealthLevel::Critical);
        assert!(dec_crit.is_circuit_broken);

        // 3. Inode < 2% -> 必须硬熔断
        let inode_crit_stat = make_stat(0.50, 0.01, false);
        let dec_inode = StorageCircuitBreaker::evaluate(&inode_crit_stat, &thresholds);
        assert_eq!(dec_inode.level, StorageHealthLevel::Critical);
        assert!(dec_inode.is_circuit_broken);

        // 4. 紧急水位 (< 8%) -> 冻结抓拍，放行告警
        let emerg_stat = make_stat(0.07, 0.20, false);
        let dec_emerg = StorageCircuitBreaker::evaluate(&emerg_stat, &thresholds);
        assert_eq!(dec_emerg.level, StorageHealthLevel::Emergency);
        assert!(!dec_emerg.allow_normal_capture);
        assert!(dec_emerg.allow_critical_alarm);
        assert!(!dec_emerg.is_circuit_broken);

        // 5. 触发淘汰水位 (< 15%) -> 放行，建议加速排空
        let evict_stat = make_stat(0.12, 0.20, false);
        let dec_evict = StorageCircuitBreaker::evaluate(&evict_stat, &thresholds);
        assert_eq!(dec_evict.level, StorageHealthLevel::Evicting);
        assert!(dec_evict.should_evict);
        assert!(dec_evict.allow_normal_capture);

        // 6. 正常水位 (> 15% 且 > 10% Inode) -> 正常巡检
        let normal_stat = make_stat(0.30, 0.30, false);
        let dec_norm = StorageCircuitBreaker::evaluate(&normal_stat, &thresholds);
        assert_eq!(dec_norm.level, StorageHealthLevel::Normal);
        assert!(!dec_norm.should_evict);
    }
}
