use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::detection::BoundingBox;

/// 告警处理状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AlarmStatus {
    /// 待处理 / 未核验 (系统初始状态)
    #[default]
    Unprocessed,
    /// 已处理 / 已核验
    Processed,
}

impl AlarmStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unprocessed => "unprocessed",
            Self::Processed => "processed",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "processed" | "handled" | "verified" => Self::Processed,
            _ => Self::Unprocessed,
        }
    }
}

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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_alarm_status_serialization_and_loose_parsing() {
        assert_eq!(AlarmStatus::Unprocessed.as_str(), "unprocessed");
        assert_eq!(AlarmStatus::Processed.as_str(), "processed");

        assert_eq!(
            AlarmStatus::from_str_loose("processed"),
            AlarmStatus::Processed
        );
        assert_eq!(
            AlarmStatus::from_str_loose("HANDLED"),
            AlarmStatus::Processed
        );
        assert_eq!(
            AlarmStatus::from_str_loose("unprocessed"),
            AlarmStatus::Unprocessed
        );
        assert_eq!(
            AlarmStatus::from_str_loose("unknown"),
            AlarmStatus::Unprocessed
        );

        let json = serde_json::to_string(&AlarmStatus::Processed).unwrap();
        assert_eq!(json, "\"processed\"");
        let de: AlarmStatus = serde_json::from_str("\"unprocessed\"").unwrap();
        assert_eq!(de, AlarmStatus::Unprocessed);
    }
}
