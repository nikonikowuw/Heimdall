//! 门控遥测：绕过计数、按类首次告警与限速日志
//!
//! 门控最危险的失效方式是**静默失效**——门控没有参与判定，却没有任何人知道。
//! 因此所有「未参与判定」的帧都必须满足三件事：
//! 1. 计入 [`GateTelemetry::bypassed_frames`]（管线按增量汇总到 `PumpMetrics::frames_gate_bypassed`）；
//! 2. 每类原因**首次**出现时 `WARN` / `ERROR` 一次（位图去重：既不刷屏，也不丢新原因）；
//! 3. 后续同类抑制为限速 `DEBUG`，与决策日志共用同一个限速时钟。
//!
//! 硬件链路不可用后的热路径必须零动态分配，因此错误原因串只在确实要写日志时才物化。

use tracing::{debug, warn};

use super::MotionGateDecision;

/// 限速日志间隔（毫秒）：现场标定阈值与观测绕过计数用
const GATE_LOG_INTERVAL_MS: i64 = 2000;

/// 门控绕过原因（按类各告警一次：既不刷屏，也不丢掉新原因）
///
/// 平台差异只体现在枚举取值上：硬件缩略图链路仅存在于 Linux 构建，
/// 非 Linux 平台只保留“载体未接入”一种原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BypassKind {
    /// 该帧载体尚未接入门控缩略图链路（如 Ascend DeviceMemory）
    CarrierUnsupported,
    /// 本机构建未启用 `rga` feature，硬件缩略图链路不存在
    #[cfg(all(target_os = "linux", not(feature = "rga")))]
    RgaDisabled,
    /// 源帧尺寸过小，无法生成门控缩略图
    #[cfg(all(target_os = "linux", feature = "rga"))]
    SourceTooSmall,
    /// 缩略图链路单帧失败（尚未达到熔断阈值）
    #[cfg(all(target_os = "linux", feature = "rga"))]
    ThumbnailError,
    /// 缩略图链路已判定不可用（初始化失败或连续失败熔断）
    #[cfg(all(target_os = "linux", feature = "rga"))]
    ThumbnailDown,
}

impl BypassKind {
    /// 告警去重位（[`GateTelemetry::warned`] 的位图位）
    #[inline]
    const fn bit(self) -> u8 {
        1 << (self as u8)
    }

    /// 固定中文短消息（变量一律走结构化字段）
    #[inline]
    const fn message(self) -> &'static str {
        match self {
            Self::CarrierUnsupported => "该帧载体尚未接入门控缩略图链路 (如 Ascend DeviceMemory)",
            #[cfg(all(target_os = "linux", not(feature = "rga")))]
            Self::RgaDisabled => "本机构建未启用 RGA 缩略图链路 (feature \"rga\")",
            #[cfg(all(target_os = "linux", feature = "rga"))]
            Self::SourceTooSmall => "源帧尺寸过小，无法生成门控缩略图",
            #[cfg(all(target_os = "linux", feature = "rga"))]
            Self::ThumbnailError => "门控缩略图单帧降采样失败",
            #[cfg(all(target_os = "linux", feature = "rga"))]
            Self::ThumbnailDown => "门控缩略图链路不可用（见首次错误告警）",
        }
    }
}

/// 决策限速日志载荷（收敛多参数，避免数据泥团）
#[derive(Debug, Clone, Copy)]
pub(super) struct DecisionLog {
    /// 评估栅格宽度（硬件缩略图载体上为 320，非源分辨率）
    pub grid_width: usize,
    /// 评估栅格高度（硬件缩略图载体上为 180，非源分辨率）
    pub grid_height: usize,
    /// 真实活跃宏块像素数（热度已被 clamp 到 1.0，反算会饱和失真）
    pub active_pixels: u32,
    /// 全图差分像素数（含被 Mask / 防区剔除的部分，用于区分“没变化”与“变化被屏蔽”）
    pub total_diff_pixels: u32,
    pub decision: MotionGateDecision,
}

/// 门控遥测状态：绕过计数 + 告警去重位图 + 限速日志时钟
///
/// 每路 `MotionGate` 一份，由门控工作线程独占，无需加锁。
#[derive(Debug, Default)]
pub(super) struct GateTelemetry {
    /// 相机标识：仅用于日志归属，便于按路标定阈值
    camera_id: String,
    /// 未参与门控判定而被放行的帧数（载体不支持或缩略图链路失败）：绝不静默失效
    bypassed_frames: u64,
    /// 已告警过的绕过原因位图（按 [`BypassKind`] 去重，每路每类原因只告警一次）
    warned: u8,
    /// 最近一次限速日志时标（毫秒）
    last_log_ms: i64,
}

impl GateTelemetry {
    /// 绑定相机标识（影响日志归属，不参与判定）
    pub(super) fn with_camera_id(camera_id: impl Into<String>) -> Self {
        Self {
            camera_id: camera_id.into(),
            ..Self::default()
        }
    }

    /// 相机标识（仅用于日志字段）
    #[inline]
    pub(super) fn camera_id(&self) -> &str {
        &self.camera_id
    }

    /// 决策限速日志：保活心跳是“门控仍然存活并定期送帧”的唯一正向证据，不受限速约束；
    /// 其余帧每 [`GATE_LOG_INTERVAL_MS`] 最多一条，现场无需额外探针即可标定 `threshold` 与 `contour_area`。
    pub(super) fn log_decision(&mut self, timestamp_ms: i64, entry: DecisionLog) {
        if !tracing::enabled!(tracing::Level::DEBUG) {
            return;
        }
        if !self.decision_log_due(timestamp_ms, entry.decision.is_keepalive) {
            return;
        }
        debug!(
            camera = %self.camera_id,
            width = entry.grid_width,
            height = entry.grid_height,
            active_pixels = entry.active_pixels,
            total_diff_pixels = entry.total_diff_pixels,
            motion_score = entry.decision.motion_score,
            should_skip = entry.decision.should_skip,
            is_keepalive = entry.decision.is_keepalive,
            bypassed_frames = self.bypassed_frames,
            "运动门控决策"
        );
    }

    /// 本条决策日志是否应当写出：保活心跳不受限速约束（并推进限速时钟，避免心跳后紧跟一条重复日志），
    /// 其余帧每 [`GATE_LOG_INTERVAL_MS`] 最多一条。
    ///
    /// 与 `tracing` 无关的纯状态逻辑，便于单测：日志开关只影响是否真的写记录。
    fn decision_log_due(&mut self, timestamp_ms: i64, is_keepalive: bool) -> bool {
        if is_keepalive {
            self.last_log_ms = timestamp_ms;
            return true;
        }
        self.log_due(timestamp_ms)
    }

    /// 限速判定：距上次日志达到 [`GATE_LOG_INTERVAL_MS`] 才允许再写一条
    fn log_due(&mut self, timestamp_ms: i64) -> bool {
        if timestamp_ms.saturating_sub(self.last_log_ms) < GATE_LOG_INTERVAL_MS {
            return false;
        }
        self.last_log_ms = timestamp_ms;
        true
    }

    /// 累计一次绕过；返回 true 表示该类原因首次出现（调用方负责发出详细告警）
    pub(super) fn count_bypass(&mut self, kind: BypassKind) -> bool {
        self.bypassed_frames = self.bypassed_frames.saturating_add(1);
        let bit = kind.bit();
        if self.warned & bit != 0 {
            return false;
        }
        self.warned |= bit;
        true
    }

    /// 绕过帧的限速调试日志（与决策日志共用限速时钟）
    pub(super) fn log_bypass(&mut self, kind: BypassKind, timestamp_ms: i64) {
        if tracing::enabled!(tracing::Level::DEBUG) && self.log_due(timestamp_ms) {
            debug!(
                camera = %self.camera_id,
                reason = kind.message(),
                bypassed_frames = self.bypassed_frames,
                "运动门控未参与判定（保守放行）"
            );
        }
    }

    /// 静态原因绕过：计数 + 按类首次告警 + 限速调试日志。绝不静默。
    pub(super) fn note_bypass(&mut self, kind: BypassKind, timestamp_ms: i64) {
        if self.count_bypass(kind) {
            warn!(
                camera = %self.camera_id,
                reason = kind.message(),
                "运动门控无法判定该帧载体，本帧保守放行；同类原因不再重复告警，仅累计 bypassed_frames"
            );
        }
        self.log_bypass(kind, timestamp_ms);
    }

    /// 带错误原因的绕过：原因串仅在确实要写日志时物化，
    /// 避免缩略图链路持续失败时逐帧 `to_string()` / `format!`（热路径零动态分配）。
    #[cfg(all(target_os = "linux", feature = "rga"))]
    pub(super) fn note_bypass_with_error(
        &mut self,
        kind: BypassKind,
        timestamp_ms: i64,
        streak: u32,
        error: &media::error::MediaError,
    ) {
        let first = self.count_bypass(kind);
        let debug_due = tracing::enabled!(tracing::Level::DEBUG) && self.log_due(timestamp_ms);
        if !first && !debug_due {
            return;
        }
        let reason = error.to_string();
        if first {
            warn!(
                camera = %self.camera_id,
                error = %reason,
                streak,
                "门控缩略图降采样失败，本帧保守放行；连续失败达到阈值将熔断该路门控"
            );
        }
        if debug_due {
            debug!(
                camera = %self.camera_id,
                error = %reason,
                bypassed_frames = self.bypassed_frames,
                "运动门控未参与判定（保守放行）"
            );
        }
    }

    /// 已累计的“未参与门控判定”帧数（载体不支持或缩略图链路失败）
    #[inline]
    pub(super) fn bypassed_frames(&self) -> u64 {
        self.bypassed_frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bypass_counts_once_per_kind() {
        let mut telemetry = GateTelemetry::with_camera_id("cam_bypass");
        assert_eq!(telemetry.camera_id(), "cam_bypass");
        assert!(
            telemetry.count_bypass(BypassKind::CarrierUnsupported),
            "每类原因首次出现必须允许调用方告警"
        );
        assert_eq!(telemetry.bypassed_frames(), 1);
        assert!(
            !telemetry.count_bypass(BypassKind::CarrierUnsupported),
            "同类原因重复出现不得再次告警（否则逐帧刷屏）"
        );
        assert_eq!(
            telemetry.bypassed_frames(),
            2,
            "重复出现仍然必须逐帧计数，绝不静默"
        );
    }

    #[test]
    fn test_log_clock_is_rate_limited() {
        let mut telemetry = GateTelemetry::default();
        assert!(telemetry.log_due(10_000), "首个日志点必须放行");
        assert!(!telemetry.log_due(10_500), "限速间隔内的日志必须被抑制");
        assert!(telemetry.log_due(12_000), "达到限速间隔后必须恢复");
    }

    #[test]
    fn test_keepalive_log_advances_rate_limit_clock() {
        let mut telemetry = GateTelemetry::with_camera_id("cam_ka");
        assert!(
            telemetry.decision_log_due(10_000, true),
            "保活心跳不受限速约束，必须始终写记录"
        );
        assert_eq!(
            telemetry.last_log_ms, 10_000,
            "心跳必须推进限速时钟，避免心跳后紧跟一条重复决策日志"
        );
        assert!(!telemetry.log_due(10_500), "心跳后限速窗口内必须抑制");
        assert!(telemetry.log_due(12_000), "限速窗口结束后必须恢复");
        assert!(
            !telemetry.decision_log_due(12_100, false),
            "非心跳帧超过限速后仍需受限速约束（刚写过一条）"
        );
    }
}
