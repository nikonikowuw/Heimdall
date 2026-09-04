use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::detection::BoundingBox;

/// 告警规则类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlarmType {
    /// 绊线越界
    LineCrossing,
    /// 区域入侵
    RegionIntrusion,
}

/// 告警记录模型 (1 Target = 1 Record)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlarmRecord {
    pub event_id: String,
    pub camera_id: String,
    pub alarm_type: AlarmType,
    pub occurred_at: DateTime<Utc>,
    pub target_label: String,
    pub confidence: f32,
    pub track_id: u64,
    pub bbox: BoundingBox,
    pub image_id: String,
    pub image_rel_path: String,
    pub created_at: DateTime<Utc>,
}
