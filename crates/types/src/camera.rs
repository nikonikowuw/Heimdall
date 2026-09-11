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

/// 码流通道类型 (主码流 / 子码流分流)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum StreamType {
    #[default]
    Main,
    Sub,
}

impl StreamType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Sub => "sub",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "sub" => Self::Sub,
            _ => Self::Main,
        }
    }
}

/// 业务流唯一标识（封装 camera_id 与 stream_type，杜绝裸字符串拼接与 Primitive Obsession）
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StreamKey {
    pub camera_id: String,
    pub stream_type: StreamType,
}

impl StreamKey {
    pub fn new(camera_id: impl Into<String>, stream_type: StreamType) -> Self {
        Self {
            camera_id: camera_id.into(),
            stream_type,
        }
    }

    pub fn main(camera_id: impl Into<String>) -> Self {
        Self::new(camera_id, StreamType::Main)
    }

    pub fn sub(camera_id: impl Into<String>) -> Self {
        Self::new(camera_id, StreamType::Sub)
    }

    pub fn as_str_key(&self) -> String {
        format!("{}:{}", self.camera_id, self.stream_type.as_str())
    }
}

impl std::fmt::Display for StreamKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.camera_id, self.stream_type.as_str())
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

    /// 健康状态优先级权重 (在线 1 > 待探活/新设备 2 > 波动 3 > 故障离线 4)
    pub fn priority(&self) -> u8 {
        match self {
            Self::Healthy => 1,
            Self::Never => 2,
            Self::Degraded => 3,
            Self::Failed => 4,
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

/// 分析码流选择偏好
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StreamMode {
    /// 自动探测：优先尝试子码流，若无子码流或不可用则自适应回退到主码流
    #[default]
    Auto,
    /// 强制使用主码流进行高精度常驻 AI 分析（画面清晰，细节完整，推荐中小路数）
    Main,
    /// 强制使用子码流进行低能耗常驻 AI 分析（节约算力，超多路并发）
    Sub,
}

/// 判断已决议的分析流是否实际使用主码流。
///
/// `analysis_url` 是任务启动前已经完成模式选择、探活和降级后的有效 URL，
/// 因此该函数也覆盖 Auto/Sub 模式在无可用子码流时回退到主码流的情况。
pub fn is_effective_main_stream(main_url: &str, analysis_url: &str) -> bool {
    let main_url = main_url.trim();
    let analysis_url = analysis_url.trim();
    analysis_url.is_empty() || main_url == analysis_url
}

impl StreamMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Main => "main",
            Self::Sub => "sub",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "main" => Self::Main,
            "sub" => Self::Sub,
            _ => Self::Auto,
        }
    }
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
    #[serde(default)]
    pub stream_mode: StreamMode,
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

impl Camera {
    /// 判定该摄像头是否实质采用主码流进行常驻 AI 分析。
    ///
    /// 满足以下任一条件即视为主码流分析模式：
    /// - 用户显式指定 `StreamMode::Main`
    /// - 子码流 URL 为空（无子码流可用）
    /// - 子码流 URL 与主码流 URL 相同（实质单流）
    pub fn is_main_stream_analysis(&self) -> bool {
        self.stream_mode == StreamMode::Main
            || is_effective_main_stream(&self.rtsp_url, &self.sub_rtsp_url)
    }
}

/// 创建摄像头请求参数
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCameraRequest {
    pub name: String,
    pub protocol: Option<CameraProtocol>,
    pub rtsp_url: String,
    pub sub_rtsp_url: Option<String>,
    pub stream_mode: Option<StreamMode>,
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
    pub stream_mode: Option<StreamMode>,
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
    fn test_effective_main_stream_selection() {
        assert!(is_effective_main_stream(
            " rtsp://camera/main ",
            "rtsp://camera/main"
        ));
        assert!(is_effective_main_stream("rtsp://camera/main", ""));
        assert!(!is_effective_main_stream(
            "rtsp://camera/main",
            "rtsp://camera/sub"
        ));

        let mut camera = Camera {
            id: 1,
            camera_id: "cam".to_string(),
            name: "cam".to_string(),
            protocol: "rtsp".to_string(),
            rtsp_url: "rtsp://camera/main".to_string(),
            sub_rtsp_url: "rtsp://camera/sub".to_string(),
            stream_mode: StreamMode::Main,
            remark: String::new(),
            transport_policy: TransportPolicy::Auto,
            last_probe_status: ProbeStatus::Healthy,
            last_probe_at: None,
            last_probe_error_code: String::new(),
            last_success_at: None,
            last_codec: "h264".to_string(),
            last_width: 1920,
            last_height: 1080,
            last_fps: 25.0,
            gb28181_device_id: None,
            gb28181_channel_id: None,
            created_at: 0,
            updated_at: 0,
        };
        assert!(camera.is_main_stream_analysis());

        camera.stream_mode = StreamMode::Auto;
        assert!(!camera.is_main_stream_analysis());
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
