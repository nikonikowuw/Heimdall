use bytes::Bytes;
use serde::{Deserialize, Serialize};

/// 摄像头编解码类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CodecType {
    #[default]
    H264,
    H265,
}

impl CodecType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::H264 => "h264",
            Self::H265 => "h265",
        }
    }
}

/// 摄像头接入协议
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CameraProtocol {
    #[default]
    Rtsp,
    Gb28181,
}

impl CameraProtocol {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Rtsp => "rtsp",
            Self::Gb28181 => "gb28181",
        }
    }
}

/// 摄像头三态健康度状态机（具备防抖容错能力）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProbeStatus {
    #[default]
    Never,
    #[serde(alias = "success")]
    Healthy,
    Degraded,
    Failed,
}

impl ProbeStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Never => "never",
            Self::Healthy => "healthy",
            Self::Degraded => "degraded",
            Self::Failed => "failed",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "healthy" | "success" => Self::Healthy,
            "degraded" | "reconnecting" => Self::Degraded,
            "failed" | "error" | "offline" => Self::Failed,
            _ => Self::Never,
        }
    }
}

/// 摄像头网络传输策略
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TransportPolicy {
    #[default]
    Auto,
    Tcp,
    Udp,
}

impl TransportPolicy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Tcp => "tcp",
            Self::Udp => "udp",
        }
    }
}

/// 跨组件流转的压缩视频数据包
#[derive(Debug, Clone)]
pub struct EncodedPacket {
    /// 13 位 UTC Unix 毫秒时间戳
    pub pts_ms: i64,
    /// 是否为关键帧 (IDR/I-Frame)
    pub is_keyframe: bool,
    /// 编码格式
    pub codec: CodecType,
    /// 包含 NALU 头或完整切片的原始字节
    pub payload: Bytes,
}

/// 摄像头核心领域模型
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Camera {
    pub id: i64,
    pub camera_id: String,
    pub name: String,
    pub protocol: String,
    pub rtsp_url: String,
    pub sub_rtsp_url: String,
    pub remark: String,
    pub transport_policy: TransportPolicy,
    pub last_probe_status: ProbeStatus,
    pub last_probe_at: Option<i64>,
    pub last_probe_error_code: String,
    pub last_success_at: Option<i64>,
    pub last_codec: String,
    pub last_width: u32,
    pub last_height: u32,
    pub last_fps: f64,
    pub gb28181_device_id: Option<String>,
    pub gb28181_channel_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 创建摄像头请求参数
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCameraRequest {
    pub name: String,
    pub protocol: Option<CameraProtocol>,
    pub rtsp_url: String,
    pub sub_rtsp_url: Option<String>,
    pub remark: Option<String>,
    pub transport_policy: Option<TransportPolicy>,
    pub gb28181_device_id: Option<String>,
    pub gb28181_channel_id: Option<String>,
}

/// 更新摄像头请求参数
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCameraRequest {
    pub name: Option<String>,
    pub rtsp_url: Option<String>,
    pub sub_rtsp_url: Option<String>,
    pub remark: Option<String>,
    pub transport_policy: Option<TransportPolicy>,
    pub gb28181_device_id: Option<String>,
    pub gb28181_channel_id: Option<String>,
}

/// 探活结果结构体
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResult {
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
}

/// 摄像头探活状态变更 WebSocket 广播事件
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraProbeEvent {
    pub camera_id: String,
    pub status: ProbeStatus,
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub error_code: String,
}

/// 摄像头高频遥测状态广播事件 (ByteTrack 目标数与运动活跃度)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraTelemetryEvent {
    pub camera_id: String,
    pub active_tracks: usize,
    pub person_count: usize,
    pub car_count: usize,
    pub motion_score: f32,
    pub is_motion_gated: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_probe_status_serialization_and_alias() {
        let status = ProbeStatus::Healthy;
        let json = serde_json::to_string(&status).expect("serialize healthy");
        assert_eq!(json, "\"healthy\"");

        // 兼容 success 别名反序列化
        let deserialized: ProbeStatus =
            serde_json::from_str("\"success\"").expect("deserialize success alias");
        assert_eq!(deserialized, ProbeStatus::Healthy);

        let degraded: ProbeStatus =
            serde_json::from_str("\"degraded\"").expect("deserialize degraded");
        assert_eq!(degraded, ProbeStatus::Degraded);
        assert_eq!(degraded.as_str(), "degraded");

        assert_eq!(
            ProbeStatus::from_str_loose("reconnecting"),
            ProbeStatus::Degraded
        );
        assert_eq!(ProbeStatus::from_str_loose("OFFLINE"), ProbeStatus::Failed);
    }

    #[test]
    fn test_camera_request_deserialization() {
        let json = r#"{
            "name": "CAM-01",
            "rtspUrl": "rtsp://127.0.0.1:8554/live",
            "transportPolicy": "tcp"
        }"#;

        let req: CreateCameraRequest = serde_json::from_str(json).expect("parse create req");
        assert_eq!(req.name, "CAM-01");
        assert_eq!(req.rtsp_url, "rtsp://127.0.0.1:8554/live");
        assert_eq!(req.transport_policy, Some(TransportPolicy::Tcp));
        assert_eq!(req.protocol, None);
    }

    #[test]
    fn test_encoded_packet_creation() {
        let packet = EncodedPacket {
            pts_ms: 1747584000123,
            is_keyframe: true,
            codec: CodecType::H264,
            payload: Bytes::from_static(&[0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1f]),
        };

        assert!(packet.is_keyframe);
        assert_eq!(packet.codec, CodecType::H264);
        assert_eq!(packet.payload.len(), 8);
    }
}
