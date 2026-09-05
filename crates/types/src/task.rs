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
pub enum TaskStatus {
    #[default]
    Stopped,
    Starting,
    Running,
    Degraded,
    Reconnecting,
    Error,
}

/// 视频分析任务模型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisTask {
    pub camera_id: String,
    pub name: String,
    pub desired_enabled: bool,
    pub actual_status: TaskStatus,
    pub status_message: String,
    pub rules: Vec<DetectionRule>,
    pub motion_gate: MotionGateConfig,
    pub last_frame_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
