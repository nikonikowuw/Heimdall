//! 跨流 PTS 轴对齐锚点
//!
//! ## 问题
//!
//! `types::EncodedPacket::pts_ms` 的语义是「**该路物理流自己的**媒体时间轴上的毫秒值」，
//! 其原点由该 RTSP 会话的 `PLAY` 应答（`RTP-Info`）决定。因此主码流与子码流各自一条
//! 时标轴，同一个真实捕获时刻在两路上的数值天然相差一个常量偏移：
//!
//! ```text
//! pts_main(W) = W + lag_main
//! pts_sub(W)  = W + lag_sub
//! 偏移 offset = lag_sub - lag_main
//! ```
//!
//! 任何形如 `main_frame.timestamp == sub_frame.timestamp`（或 `abs_diff <= 容忍度`）的
//! 跨流比较都在量一条**已经错位**的轴，恒为「近似成立」而永远无法发现错配——这正是子码流
//! 推理模式下，主码流证据帧与推理帧对不上的根因。
//!
//! ## 修复依据
//!
//! PTS 是媒体时间，加上实测的接入时延后才是墙上时间：
//!
//! ```text
//! capture_wall ≈ pts_ms + lag，其中 lag = wall_arrival - pts_ms
//! ```
//!
//! `lag` 可在接入层直接观测。于是跨流可比的量是 `pts_ms + lag`，把子码流轴上的目标时标
//! 换算到主码流轴即为：
//!
//! ```text
//! main_axis_pts = detection_pts + (lag_analysis - lag_main)
//! ```
//!
//! ## 为什么取低分位数而非均值/EMA
//!
//! 接入时延的噪声是**单边**的：网络拥塞与任务调度只会让包到达得更晚，不会更早。均值与 EMA
//! 会被系统性抬高，从而低估偏移量。固定窗口的低分位数（p10）能够稳定估计到时延地板，
//! 且对偶发调度停顿（<10% 的离群样本）不敏感。
//!
//! 本锚点只做估计，**不替调用方决定容错策略**：未收敛时 `lag_ms()` 返回 `None`，调用方必须
//! 走安全回退（使用时间必然正确的推理帧），严禁假定偏移为 0，否则等于把缺陷静默还原。

use std::collections::VecDeque;

use parking_lot::Mutex;

/// 滞后采样窗口容量（约 8 秒 @25fps）。窗口越长，分位数越稳。
const LAG_WINDOW_SAMPLES: usize = 200;
/// 低分位数的分子/分母，即 p10。
const LAG_PERCENTILE_NUM: usize = 10;
const LAG_PERCENTILE_DEN: usize = 100;
/// 达成标定所需的最少采样数（约 2 秒 @25fps）。
const LAG_MIN_SAMPLES: usize = 48;
/// 单个样本的绝对值上限。超出视为时钟跳变或会话切换瞬间的脏样本，直接丢弃。
const LAG_MAX_ABS_MS: i64 = 10_000;

#[derive(Debug, Default)]
struct LagEstimator {
    samples: VecDeque<i64>,
    /// 已标定的接入时延；`None` 表示样本不足，不得用于跨流换算。
    lag_ms: Option<i64>,
}

/// 单路物理流的接入时延锚点，提供跨流 PTS 轴换算。
///
/// 采样窗口与估计值同锁承载，避免两个原子之间出现「标定已可见、估计值尚未可见」的
/// 撕裂读取（那会把偏移读成 0，正好退回本模块要修掉的缺陷）。临界区只做一趟
/// `VecDeque` 推入与一次 200 元素排序，不涉及 IO、FFI 或 `await`。
#[derive(Debug)]
pub struct StreamClockAnchor {
    inner: Mutex<LagEstimator>,
}

impl Default for StreamClockAnchor {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamClockAnchor {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(LagEstimator::default()),
        }
    }

    /// 观测一个接入时延样本 `wall_arrival_ms - pts_ms`。
    pub fn observe(&self, lag_sample_ms: i64) {
        // 用区间判断而非 abs()：i64::MIN 取绝对值会溢出。
        if !(-LAG_MAX_ABS_MS..=LAG_MAX_ABS_MS).contains(&lag_sample_ms) {
            return;
        }

        let mut estimator = self.inner.lock();
        if estimator.samples.len() == LAG_WINDOW_SAMPLES {
            estimator.samples.pop_front();
        }
        estimator.samples.push_back(lag_sample_ms);

        if estimator.samples.len() < LAG_MIN_SAMPLES {
            return;
        }

        let mut sorted: Vec<i64> = estimator.samples.iter().copied().collect();
        sorted.sort_unstable();
        estimator.lag_ms = Some(sorted[sorted.len() * LAG_PERCENTILE_NUM / LAG_PERCENTILE_DEN]);
    }

    /// 会话重连时作废历史样本。
    ///
    /// 每次 `PLAY` 都会重新协商 `RTP-Info`，PTS 原点随之改变，旧连接的滞后样本
    /// 与新轴不可混用。清空后 `lag_ms()` 立即回到 `None`，调用方自动退回安全路径。
    pub fn reset(&self) {
        *self.inner.lock() = LagEstimator::default();
    }

    /// 已标定的接入时延；`None` 表示样本不足，**不得**用于跨流换算。
    pub fn lag_ms(&self) -> Option<i64> {
        self.inner.lock().lag_ms
    }

    /// 当前窗口内的有效样本数，用于诊断与可观测性。
    pub fn sample_count(&self) -> usize {
        self.inner.lock().samples.len()
    }

    /// 把 `self` 轴上的时标换算到 `other` 轴上。
    ///
    /// `self` 为源轴（如分析流），`other` 为目标轴（如主码流证据流）。
    /// 返回 `None` 表示任一侧未标定，调用方必须走安全回退。
    ///
    /// 同一物理流（两份 `Arc` 指向同一锚点）时偏移恒为 0，无需标定即可成立。
    pub fn convert_pts_to(&self, other: &StreamClockAnchor, pts_ms: i64) -> Option<i64> {
        if std::ptr::eq(self, other) {
            return Some(pts_ms);
        }
        Some(pts_ms + self.lag_ms()? - other.lag_ms()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calibrated_anchor(samples: impl IntoIterator<Item = i64>) -> StreamClockAnchor {
        let anchor = StreamClockAnchor::new();
        for sample in samples {
            anchor.observe(sample);
        }
        anchor
    }

    #[test]
    fn test_uncalibrated_returns_none() {
        let anchor = StreamClockAnchor::new();
        assert_eq!(anchor.lag_ms(), None);

        let other = StreamClockAnchor::new();
        assert_eq!(anchor.convert_pts_to(&other, 1_000), None);
    }

    #[test]
    fn test_calibrates_after_min_samples() {
        let anchor = StreamClockAnchor::new();
        for i in 0..(LAG_MIN_SAMPLES - 1) {
            anchor.observe(100 + i as i64);
        }
        assert_eq!(anchor.lag_ms(), None, "样本未达阈值不得提前标定");

        anchor.observe(180);
        assert!(anchor.lag_ms().is_some());
    }

    #[test]
    fn test_low_percentile_rejects_one_sided_spikes() {
        // 时延地板 120ms，偶发调度停顿把样本抬高到 400ms。
        let samples: Vec<i64> = (0..LAG_WINDOW_SAMPLES)
            .map(|i| {
                if i % 10 == 0 {
                    400
                } else {
                    120 + (i % 3) as i64
                }
            })
            .collect();
        let mean = samples.iter().sum::<i64>() / samples.len() as i64;
        let lag = calibrated_anchor(samples).lag_ms().expect("应已标定");

        assert!(
            (118..=129).contains(&lag),
            "p10 应贴近地板 120ms，实际 {lag}ms"
        );
        assert!(
            lag < mean,
            "单边噪声下低分位必须低于均值（lag={lag}, mean={mean}）"
        );
    }

    #[test]
    fn test_cross_stream_offset_conversion() {
        // 主码流滞后 150ms，子码流滞后 90ms → 偏移 = 90 - 150 = -60ms
        let main = calibrated_anchor([150; LAG_WINDOW_SAMPLES]);
        let sub = calibrated_anchor([90; LAG_WINDOW_SAMPLES]);

        assert_eq!(main.lag_ms(), Some(150));
        assert_eq!(sub.lag_ms(), Some(90));
        assert_eq!(sub.convert_pts_to(&main, 10_000), Some(9_940));
        // 反向换算必须自洽
        assert_eq!(main.convert_pts_to(&sub, 9_940), Some(10_000));
    }

    #[test]
    fn test_same_anchor_is_identity_without_calibration() {
        let anchor = StreamClockAnchor::new();
        assert_eq!(anchor.convert_pts_to(&anchor, 12_345), Some(12_345));
    }

    #[test]
    fn test_conversion_requires_both_sides_calibrated() {
        let main = calibrated_anchor([150; LAG_WINDOW_SAMPLES]);
        let uncalibrated = StreamClockAnchor::new();

        assert_eq!(uncalibrated.convert_pts_to(&main, 1_000), None);
        assert_eq!(main.convert_pts_to(&uncalibrated, 1_000), None);
    }

    #[test]
    fn test_reset_clears_calibration() {
        let anchor = calibrated_anchor([130; LAG_WINDOW_SAMPLES]);
        assert!(anchor.lag_ms().is_some());

        anchor.reset();

        assert_eq!(anchor.lag_ms(), None);
        assert_eq!(anchor.sample_count(), 0);
    }

    #[test]
    fn test_absurd_samples_are_discarded() {
        let anchor = StreamClockAnchor::new();
        anchor.observe(i64::MIN);
        anchor.observe(i64::MAX);
        anchor.observe(LAG_MAX_ABS_MS + 1);
        assert_eq!(anchor.sample_count(), 0, "脏样本必须被丢弃");

        // 采样数不足时不得标定
        anchor.observe(120);
        assert_eq!(anchor.lag_ms(), None);
    }

    #[test]
    fn test_window_is_bounded() {
        let anchor = StreamClockAnchor::new();
        for i in 0..(LAG_WINDOW_SAMPLES * 5) {
            anchor.observe(100 + (i % 7) as i64);
        }
        assert_eq!(anchor.sample_count(), LAG_WINDOW_SAMPLES);
    }

    #[test]
    fn test_reconnect_shifts_axis_after_reset() {
        let main = calibrated_anchor([150; LAG_WINDOW_SAMPLES]);
        let sub = calibrated_anchor([90; LAG_WINDOW_SAMPLES]);
        assert_eq!(sub.convert_pts_to(&main, 10_000), Some(9_940));

        // 主码流重连：RTP-Info 重新协商，旧样本必须作废
        main.reset();
        assert_eq!(sub.convert_pts_to(&main, 10_000), None);

        // 新连接收敛到新的滞后值后偏移随之更新
        for _ in 0..LAG_WINDOW_SAMPLES {
            main.observe(30);
        }
        assert_eq!(sub.convert_pts_to(&main, 10_000), Some(10_060));
    }
}
