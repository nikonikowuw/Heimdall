use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 摄像头探活状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProbeStatus {
    #[default]
    Never,
    Success,
    Failed,
}

/// 摄像头传输策略
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TransportPolicy {
    #[default]
    Auto,
    Tcp,
    Udp,
}

/// 摄像头视频源核心领域模型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Camera {
    pub camera_id: String,
    pub name: String,
    pub protocol: String,
    pub rtsp_url: String,
    pub sub_rtsp_url: String,
    pub remark: String,
    pub transport_policy: TransportPolicy,
    pub last_probe_status: ProbeStatus,
    pub last_probe_at: Option<DateTime<Utc>>,
    pub last_probe_error_code: String,
    pub last_success_at: Option<DateTime<Utc>>,
    pub last_codec: String,
    pub last_width: u32,
    pub last_height: u32,
    pub last_fps: f64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
