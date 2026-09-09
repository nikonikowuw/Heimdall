use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 检测规则角色
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetectionRuleRole {
    /// 感兴趣/入侵布防区域
    Roi,
    /// 屏蔽遮罩区域
    Mask,
    /// 绊线越界检测
    Line,
}

/// 分界线越界方向
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DetectionLineDirection {
    #[default]
    Both,
    AToB,
    BToA,
}

/// 归一化二维坐标点 (0.0 ~ 1.0)
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DetectionPoint {
    pub x: f64,
    pub y: f64,
}

impl DetectionPoint {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// 任务级空间几何布防规则
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectionRule {
    pub role: DetectionRuleRole,
    #[serde(default, alias = "line_direction")]
    pub line_direction: DetectionLineDirection,
    pub points: Vec<DetectionPoint>,
}

/// 运动门控配置参数
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MotionGateConfig {
    pub enabled: bool,
    #[serde(default = "default_threshold")]
    pub threshold: u32,
    #[serde(default = "default_contour_area")]
    pub contour_area: u32,
    #[serde(
        default = "default_keepalive_interval_ms",
        alias = "keepalive_interval_ms"
    )]
    pub keepalive_interval_ms: u64,
}

fn default_threshold() -> u32 {
    25
}
fn default_contour_area() -> u32 {
    100
}
fn default_keepalive_interval_ms() -> u64 {
    2000
}

impl Default for MotionGateConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            threshold: default_threshold(),
            contour_area: default_contour_area(),
            keepalive_interval_ms: default_keepalive_interval_ms(),
        }
    }
}

/// 分析任务状态码
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
#[repr(i32)]
pub enum TaskStatus {
    #[default]
    Stopped = 0,
    Starting = 1,
    Running = 2,
    Degraded = 3,
    Reconnecting = 4,
    Error = 5,
}

impl TaskStatus {
    pub const fn as_i32(self) -> i32 {
        self as i32
    }

    pub const fn from_i32(val: i32) -> Option<Self> {
        match val {
            0 => Some(Self::Stopped),
            1 => Some(Self::Starting),
            2 => Some(Self::Running),
            3 => Some(Self::Degraded),
            4 => Some(Self::Reconnecting),
            5 => Some(Self::Error),
            _ => None,
        }
    }
}

impl From<TaskStatus> for i32 {
    fn from(s: TaskStatus) -> Self {
        s.as_i32()
    }
}

impl TryFrom<i32> for TaskStatus {
    type Error = i32;
    fn try_from(val: i32) -> Result<Self, i32> {
        Self::from_i32(val).ok_or(val)
    }
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stopped => write!(f, "stopped"),
            Self::Starting => write!(f, "starting"),
            Self::Running => write!(f, "running"),
            Self::Degraded => write!(f, "degraded"),
            Self::Reconnecting => write!(f, "reconnecting"),
            Self::Error => write!(f, "error"),
        }
    }
}

pub fn default_algo_params() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
}

/// 视频分析任务模型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisTask {
    pub camera_id: String,
    pub name: String,
    pub desired_enabled: bool,
    pub actual_status: TaskStatus,
    pub status_message: String,
    #[serde(default)]
    pub algorithm_id: String,
    #[serde(default)]
    pub analysis_fps: u32,
    #[serde(default = "default_algo_params")]
    pub algo_params: serde_json::Value,
    pub rules: Vec<DetectionRule>,
    pub motion_gate: MotionGateConfig,
    pub last_frame_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_status_representation_and_conversions() {
        assert_eq!(TaskStatus::Stopped.as_i32(), 0);
        assert_eq!(TaskStatus::Starting.as_i32(), 1);
        assert_eq!(TaskStatus::Running.as_i32(), 2);
        assert_eq!(TaskStatus::Degraded.as_i32(), 3);
        assert_eq!(TaskStatus::Reconnecting.as_i32(), 4);
        assert_eq!(TaskStatus::Error.as_i32(), 5);

        assert_eq!(TaskStatus::from_i32(0), Some(TaskStatus::Stopped));
        assert_eq!(TaskStatus::from_i32(1), Some(TaskStatus::Starting));
        assert_eq!(TaskStatus::from_i32(2), Some(TaskStatus::Running));
        assert_eq!(TaskStatus::from_i32(3), Some(TaskStatus::Degraded));
        assert_eq!(TaskStatus::from_i32(4), Some(TaskStatus::Reconnecting));
        assert_eq!(TaskStatus::from_i32(5), Some(TaskStatus::Error));
        assert_eq!(TaskStatus::from_i32(6), None);
        assert_eq!(TaskStatus::from_i32(-1), None);

        assert_eq!(i32::from(TaskStatus::Running), 2);
        assert_eq!(TaskStatus::try_from(2), Ok(TaskStatus::Running));
        assert_eq!(TaskStatus::try_from(99), Err(99));

        assert_eq!(TaskStatus::Running.to_string(), "running");
        assert_eq!(TaskStatus::Stopped.to_string(), "stopped");
    }

    #[test]
    fn test_task_status_serde() {
        let serialized = serde_json::to_string(&TaskStatus::Running).expect("serialize");
        assert_eq!(serialized, "\"running\"");

        let deserialized: TaskStatus = serde_json::from_str("\"running\"").expect("deserialize");
        assert_eq!(deserialized, TaskStatus::Running);
    }

    #[test]
    fn test_analysis_task_backward_compatible_deserialization() {
        let json_data = r#"{
            "camera_id": "CAM-01",
            "name": "Test Task",
            "desired_enabled": true,
            "actual_status": "running",
            "status_message": "All good",
            "rules": [],
            "motion_gate": {
                "enabled": true,
                "threshold": 25,
                "contour_area": 100,
                "keepalive_interval_ms": 2000
            },
            "last_frame_at": null,
            "created_at": "2026-09-08T12:00:00Z",
            "updated_at": "2026-09-08T12:00:00Z"
        }"#;

        let task: AnalysisTask =
            serde_json::from_str(json_data).expect("deserialize without algo fields");
        assert_eq!(task.algorithm_id, "");
        assert_eq!(task.analysis_fps, 0);
        assert_eq!(task.algo_params, serde_json::json!({}));
        assert_eq!(task.actual_status, TaskStatus::Running);
    }
}
