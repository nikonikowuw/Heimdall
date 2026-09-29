//! 算法实例推理在途心跳与活性判定
//!
//! 常驻推理主路径的活性无法从宿主侧的结果面反推：一帧没有检测目标时，
//! 宿主看到的同样是「没有结果产出」，它既不能证明算法停摆，也不能证明算法在跑。
//! 本模块独立观测**同步调用序列是否推进**——主路径在同步阻塞点前取得 [`InflightGuard`]，
//! 由守卫的 `Drop` 在返回、报错或调用被取消时统一退出在途；全部为无锁原子操作，
//! 不触碰帧数据、不做任何 IO，对零拷贝主路径零干扰。
//!
//! ## 三态语义（这是本模块存在的核心理由）
//!
//! 单一「在途起始时间戳」会退化成二值语义：`0.0` 同时表示「健康空闲」与
//! 「从未进入推理」。上游开源实现（Frigate `detection_start`）正是踩了这个陷阱，
//! 导致推理进程在初始化阶段挂死时两路看门狗全部失明，静默故障持续数日。
//!
//! 因此这里额外维护一个单调递增、永不归零的 `entered_total`：
//!
//! | 状态 | 判据 |
//! | --- | --- |
//! | [`InflightVerdict::NeverEntered`] | `entered_total == 0` 且已超出启动宽限 |
//! | [`InflightVerdict::HealthyIdle`] | `entered_total > 0` 且当前空闲 |
//! | [`InflightVerdict::HealthyInFlight`] | 在途且未超时 |
//!
//! 「从未进入」只证明没有帧进入过推理循环：源流久未交付可用帧（掉线、等不到
//! 关键帧、门控尚未放行）与插件初始化挂死同判，消费方必须结合源帧到达状态判读。
//!
//! ## 时钟约定
//!
//! 所有时间戳均为**单调时钟毫秒**，禁止使用挂钟时间：NTP 回拨会把在途时长
//! 算成负数或巨值，直接产生误报。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

/// 进程内单调时钟毫秒（首个调用点为零点）
///
/// 与媒体层 `dispatcher::monotonic_ms` 同构，但本模块需独立持有，
/// 避免 pipeline 依赖 media 的内部工具函数。
pub fn monotonic_now_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START
        .get_or_init(Instant::now)
        .elapsed()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

/// 推理在途心跳标记
///
/// 由推理循环独占写入，任意线程可无锁读取。四个原子量分别回答：
/// - `in_flight_since_ms`：现在是否卡在推理里？
/// - `entered_total`：这个实例究竟有没有真正跑起来过？
/// - `entered - completed`：有没有死在返回路径上？
/// - `started_at_ms`：启动宽限期从何时起算？
#[derive(Debug)]
pub struct InflightMarker {
    /// 实例创建时刻，用于区分「仍在启动宽限」与「从未进入推理」
    started_at_ms: u64,
    /// 当前在途推理的起始时刻（0 表示空闲）
    in_flight_since_ms: AtomicU64,
    /// 累计进入推理次数（单调递增，永不归零）
    entered_total: AtomicU64,
    /// 累计退出推理次数（单调递增）
    completed_total: AtomicU64,
}

impl InflightMarker {
    /// 创建标记，`started_at_ms` 必须来自 [`monotonic_now_ms`]。
    pub fn new(started_at_ms: u64) -> Self {
        Self {
            started_at_ms,
            in_flight_since_ms: AtomicU64::new(0),
            entered_total: AtomicU64::new(0),
            completed_total: AtomicU64::new(0),
        }
    }

    /// 进入推理前调用，返回的 [`InflightGuard`] 在 `Drop` 时退出在途。
    ///
    /// 先递增 `entered_total` 再写入时刻：即使后续推理直接卡死，
    /// 「该实例确实进入过推理」这一事实也不会丢失。
    #[inline]
    #[must_use = "在途守卫必须存活到推理调用返回，否则在途心跳会被立即清空"]
    pub fn enter(&self, now_ms: u64) -> InflightGuard<'_> {
        self.entered_total.fetch_add(1, Ordering::Relaxed);
        self.in_flight_since_ms.store(now_ms, Ordering::Release);
        InflightGuard { marker: self }
    }

    /// 退出在途（仅由 [`InflightGuard`] 调用）。
    #[inline]
    fn leave(&self) {
        self.completed_total.fetch_add(1, Ordering::Relaxed);
        self.in_flight_since_ms.store(0, Ordering::Release);
    }

    /// 采集一次无锁快照。
    pub fn sample(&self) -> InflightSample {
        InflightSample {
            started_at_ms: self.started_at_ms,
            in_flight_since_ms: self.in_flight_since_ms.load(Ordering::Acquire),
            entered_total: self.entered_total.load(Ordering::Relaxed),
            completed_total: self.completed_total.load(Ordering::Relaxed),
        }
    }
}

/// 在途守卫：退出在途的唯一路径。
///
/// 覆盖三条退出路径——正常返回、返回错误、以及调用 future 在 await 点被取消或 abort。
/// 刻意用 `Drop` 而不是显式 `leave`：`tokio::select!` 取消分支胜出时分支 future 会在
/// await 点被整体丢弃，显式收尾永远不会执行，实例将永久停留在「在途」并被误判为卡死。
#[derive(Debug)]
pub struct InflightGuard<'a> {
    marker: &'a InflightMarker,
}

impl Drop for InflightGuard<'_> {
    fn drop(&mut self) {
        self.marker.leave();
    }
}

/// 在途心跳的一次采样快照
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InflightSample {
    /// 实例创建时刻（单调毫秒）
    pub started_at_ms: u64,
    /// 在途起始时刻（0 表示空闲）
    pub in_flight_since_ms: u64,
    /// 累计进入推理次数
    pub entered_total: u64,
    /// 累计退出推理次数
    pub completed_total: u64,
}

impl InflightSample {
    /// 当前是否处于在途状态
    pub fn is_in_flight(&self) -> bool {
        self.in_flight_since_ms > 0
    }

    /// 尚未退出的在途调用数（正常情况下为 0 或 1）
    pub fn outstanding(&self) -> u64 {
        self.entered_total.saturating_sub(self.completed_total)
    }
}

/// 在途活性判定结论
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InflightVerdict {
    /// 曾进入过推理，当前空闲——门控全屏蔽与低抽帧率下的合法状态
    HealthyIdle,
    /// 在途且未超时
    HealthyInFlight,
    /// 自实例启动以来从未有帧进入推理循环，且已超出启动宽限。
    ///
    /// 只说明「没有帧进入过推理」，不证明插件初始化挂死：源流未交付被放行的帧
    /// 也会走到这里，消费方必须结合源帧到达/解码状态判读。
    NeverEntered,
    /// 在途超时——同步推理调用卡死
    Stuck {
        /// 已卡住的时长（毫秒）
        in_flight_ms: u64,
    },
    /// 有入无出——退出路径未被记录（防御性判定，正常循环不会出现）
    EnteredButNeverLeaves {
        /// 未完成的在途调用数
        outstanding: u64,
    },
}

/// 在途活性判定阈值
///
/// 必须按平台配置：RKNN/ACL 首次加载模型与预热的耗时差异很大，
/// 且推理超时强杀会破坏设备 context，阈值过短比漏报更危险。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InflightThresholds {
    /// 启动宽限期：实例创建后多久仍未进入推理才判定为初始化挂死
    pub entry_grace_ms: u64,
    /// 单次推理卡死阈值
    pub stall_timeout_ms: u64,
    /// 允许的未完成在途调用数上限
    pub max_outstanding: u64,
}

impl Default for InflightThresholds {
    fn default() -> Self {
        Self {
            // 冷启动需为模型加载、NPU context 初始化与首帧预热留足余量
            entry_grace_ms: 120_000,
            // 显著高于常见推理延迟，避免把慢速首帧误判为卡死
            stall_timeout_ms: 30_000,
            max_outstanding: 2,
        }
    }
}

/// 判定在途活性。
///
/// 判定优先级：从未进入 > 在途超时 > 有入无出 > 健康。
/// 「从未进入」排在最前，因为它是唯一无法从其它计数器推断出的状态。
pub fn evaluate_inflight(
    sample: InflightSample,
    now_ms: u64,
    thresholds: &InflightThresholds,
) -> InflightVerdict {
    if sample.entered_total == 0 {
        let age_ms = now_ms.saturating_sub(sample.started_at_ms);
        if age_ms > thresholds.entry_grace_ms {
            return InflightVerdict::NeverEntered;
        }
        return InflightVerdict::HealthyIdle;
    }

    if sample.is_in_flight() {
        let in_flight_ms = now_ms.saturating_sub(sample.in_flight_since_ms);
        if in_flight_ms > thresholds.stall_timeout_ms {
            return InflightVerdict::Stuck { in_flight_ms };
        }
        return InflightVerdict::HealthyInFlight;
    }

    let outstanding = sample.outstanding();
    if outstanding > thresholds.max_outstanding {
        return InflightVerdict::EnteredButNeverLeaves { outstanding };
    }

    InflightVerdict::HealthyIdle
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const START: u64 = 1_000_000;

    fn thresholds() -> InflightThresholds {
        InflightThresholds {
            entry_grace_ms: 10_000,
            stall_timeout_ms: 5_000,
            max_outstanding: 2,
        }
    }

    fn idle_since_start() -> InflightSample {
        InflightMarker::new(START).sample()
    }

    #[test]
    fn fresh_instance_within_grace_period_is_healthy_idle() {
        assert_eq!(
            evaluate_inflight(idle_since_start(), START + 5_000, &thresholds()),
            InflightVerdict::HealthyIdle
        );
    }

    #[test]
    fn instance_that_never_entered_inference_is_reported_after_grace() {
        // Frigate detection_start 的经典盲区：进程存活但初始化挂死时，
        // 单一时间戳判据把它当成健康空闲；entered_total 锚点才能识别。
        assert_eq!(
            evaluate_inflight(idle_since_start(), START + 10_001, &thresholds()),
            InflightVerdict::NeverEntered
        );
    }

    #[test]
    fn in_flight_beyond_stall_timeout_is_stuck() {
        let mut sample = idle_since_start();
        sample.in_flight_since_ms = START + 1_000;
        sample.entered_total = 1;
        assert_eq!(
            evaluate_inflight(sample, START + 1_000 + 5_001, &thresholds()),
            InflightVerdict::Stuck {
                in_flight_ms: 5_001
            }
        );
    }

    #[test]
    fn in_flight_within_stall_timeout_is_healthy() {
        let mut sample = idle_since_start();
        sample.in_flight_since_ms = START + 1_000;
        sample.entered_total = 1;
        assert_eq!(
            evaluate_inflight(sample, START + 1_000 + 4_999, &thresholds()),
            InflightVerdict::HealthyInFlight
        );
    }

    #[test]
    fn entered_but_never_leaves_is_distinguished_from_idle() {
        let mut sample = idle_since_start();
        sample.entered_total = 10;
        sample.completed_total = 7;
        assert_eq!(
            evaluate_inflight(sample, START + 60_000, &thresholds()),
            InflightVerdict::EnteredButNeverLeaves { outstanding: 3 }
        );
    }

    #[test]
    fn balanced_enter_leave_cycles_stay_healthy() {
        let marker = InflightMarker::new(START);
        drop(marker.enter(START + 100));
        drop(marker.enter(START + 200));

        let sample = marker.sample();
        assert_eq!(sample.entered_total, 2);
        assert_eq!(sample.completed_total, 2);
        assert_eq!(sample.outstanding(), 0);
        assert!(!sample.is_in_flight());
        assert_eq!(
            evaluate_inflight(sample, START + 60_000, &thresholds()),
            InflightVerdict::HealthyIdle
        );
    }

    #[test]
    fn enter_records_entry_before_exposing_in_flight_position() {
        let marker = InflightMarker::new(START);
        let guard = marker.enter(START + 999);

        let sample = marker.sample();
        assert_eq!(sample.entered_total, 1, "进入事实必须先于在途时刻可见");
        assert_eq!(sample.in_flight_since_ms, START + 999);
        drop(guard);
    }

    #[test]
    fn dropping_guard_exits_in_flight_even_when_call_is_abandoned() {
        // 取消安全：调用 future 在 await 点被 drop（关机/替换）时守卫随之析构，
        // 实例不得被永久留在在途状态。
        let marker = InflightMarker::new(START);
        {
            let _guard = marker.enter(START + 500);
            assert!(marker.sample().is_in_flight());
        }

        let sample = marker.sample();
        assert!(!sample.is_in_flight());
        assert_eq!(sample.outstanding(), 0);
        assert_eq!(
            evaluate_inflight(sample, START + 60_000, &thresholds()),
            InflightVerdict::HealthyIdle
        );
    }

    #[test]
    fn clock_rewind_cannot_produce_negative_elapsed() {
        // 单调时钟保证 now 不回退；即便调用方传入较小值，也必须饱和为零而非下溢
        let mut sample = idle_since_start();
        sample.in_flight_since_ms = START;
        sample.entered_total = 1;
        assert_eq!(
            evaluate_inflight(sample, START - 500, &thresholds()),
            InflightVerdict::HealthyInFlight
        );
    }

    #[test]
    fn outstanding_boundary_is_inclusive_of_threshold() {
        let mut sample = idle_since_start();
        sample.entered_total = 5;
        sample.completed_total = 3;
        assert_eq!(
            evaluate_inflight(sample, START + 60_000, &thresholds()),
            InflightVerdict::HealthyIdle,
            "恰好等于阈值不应判为异常"
        );
    }
}
