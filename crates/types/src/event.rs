//! WebSocket 广播事件与 Topic 常量契约
//!
//! 保证前后端、各内部子系统消费的 Topic 标识单一来源 (Single Source of Truth)

/// 摄像头探活与流媒体基础指标更新广播事件
pub const TOPIC_CAMERA_PROBE_UPDATED: &str = "camera.probe_updated";

/// 新增违规告警触发与实时抓拍广播事件
pub const TOPIC_ALARM_TRIGGERED: &str = "alarm.triggered";

/// 告警处理状态变更广播事件
pub const TOPIC_ALARM_STATUS_CHANGED: &str = "alarm.status_changed";

/// 摄像头实时 AI 航迹与目标框更新广播事件 (节流下发)
pub const TOPIC_CAMERA_TRACKS: &str = "camera.tracks";
