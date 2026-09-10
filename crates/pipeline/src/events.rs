use types::TrackedObject;

use crate::rules::TriggeredAlarm;
use crate::snapshot::SnapshotResult;

/// 默认管线分析事件广播通道容量
pub const DEFAULT_ANALYSIS_EVENT_CHANNEL_CAPACITY: usize = 1024;

/// 告警证据生成状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceStatus {
    /// 全景图与特写均已落地。
    Ready,
    /// 告警事实已成立，但证据生成失败。
    Failed,
}

/// 管线告警触发事件载荷
#[derive(Debug, Clone)]
pub struct PipelineAlarmEvent {
    /// 告警唯一事件 ID (UUID v4)，在证据生成前分配并贯穿整个事件。
    pub event_id: String,
    /// 触发告警的摄像头 ID
    pub camera_id: String,
    /// 规则引擎判定的告警上下文
    pub alarm: TriggeredAlarm,
    /// 靶向抓拍结果；证据失败时为 None，但告警事实仍然保留。
    pub snapshot: Option<SnapshotResult>,
    /// 证据生成的最终状态
    pub evidence_status: EvidenceStatus,
    /// 证据失败原因，仅供领域层消费者记录，不作为客户端稳定错误码。
    pub evidence_error: Option<String>,
    /// 告警触发时刻的 13 位 UTC Unix 毫秒时间戳
    pub timestamp: i64,
}

/// 管线实时航迹跟踪更新事件载荷
#[derive(Debug, Clone)]
pub struct PipelineTrackEvent {
    /// 摄像头 ID
    pub camera_id: String,
    /// 产生航迹的算法实例标识 (如 "builtin-yolo26")
    pub algorithm_id: String,
    /// 帧时间戳 (13 位 UTC Unix 毫秒)
    pub timestamp: i64,
    /// 当前帧的所有活跃航迹对象
    pub tracks: Vec<TrackedObject>,
}

/// 管线分析事件统一出口枚举
#[derive(Debug, Clone)]
pub enum PipelineAnalysisEvent {
    /// 规则告警事件 (稀疏触发)
    Alarm(Box<PipelineAlarmEvent>),
    /// 航迹跟踪元数据更新事件 (高频流式)
    Tracks(PipelineTrackEvent),
}
