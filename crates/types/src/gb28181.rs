use serde::{Deserialize, Serialize};

/// 系统级 GB/T 28181 本机 SIP 服务配置
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SysGb28181Config {
    pub sip_id: String,
    pub sip_domain: String,
    pub sip_port: u16,
    pub sip_password: String,
    pub rtp_port_range_start: u16,
    pub rtp_port_range_end: u16,
    pub auto_catalog_sync: bool,
    pub heartbeat_timeout_sec: u32,
    pub updated_at_ms: i64,
}

impl Default for SysGb28181Config {
    fn default() -> Self {
        Self {
            sip_id: "34020000002000000001".to_string(),
            sip_domain: "3402000000".to_string(),
            sip_port: 5060,
            sip_password: "admin123".to_string(),
            rtp_port_range_start: 30000,
            rtp_port_range_end: 30500,
            auto_catalog_sync: true,
            heartbeat_timeout_sec: 180,
            updated_at_ms: 0,
        }
    }
}

/// 修改 GB28181 配置请求
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateGb28181ConfigRequest {
    pub sip_id: Option<String>,
    pub sip_domain: Option<String>,
    pub sip_port: Option<u16>,
    pub sip_password: Option<String>,
    pub rtp_port_range_start: Option<u16>,
    pub rtp_port_range_end: Option<u16>,
    pub auto_catalog_sync: Option<bool>,
    pub heartbeat_timeout_sec: Option<u32>,
}

/// GB28181 服务运行健康与指标
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Gb28181ServerHealth {
    pub running: bool,
    pub sip_port: u16,
    pub transport: String,
    pub online_devices_count: usize,
    pub total_devices_count: usize,
    pub active_streams_count: usize,
}

/// GB28181 配置与健康响应
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Gb28181ConfigResponse {
    pub config: SysGb28181Config,
    pub health: Gb28181ServerHealth,
}

/// 已向本平台注册的 GB28181 物理设备（如 IPC、NVR）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Gb28181DeviceDto {
    pub device_id: String,
    pub name: String,
    pub ip_addr: String,
    pub sip_port: u16,
    pub transport: String,
    pub status: String,
    pub channel_count: u32,
    pub last_keepalive_ms: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub channels: Vec<Gb28181ChannelDto>,
}

/// GB28181 设备通道详情
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Gb28181ChannelDto {
    pub device_id: String,
    pub channel_id: String,
    pub name: String,
    pub manufacturer: String,
    pub model: String,
    pub status: String,
    pub parent_id: String,
    pub sub_stream_supported: bool,
    pub last_seen_ms: i64,
    pub is_imported: bool,
    pub camera_id: Option<String>,
}

/// 批量纳管通道项
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportGbChannelItem {
    pub device_id: String,
    pub channel_id: String,
    pub name: Option<String>,
    pub stream_mode: Option<String>,
}

/// 批量纳管通道请求
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchImportGbChannelsRequest {
    pub channels: Vec<ImportGbChannelItem>,
}

/// 批量纳管结果
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchImportGbChannelsResponse {
    pub imported_count: usize,
    pub camera_ids: Vec<String>,
}

/// 局域网主动发现（ONVIF / RTSP）的设备信息
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredDevice {
    pub ip: String,
    pub port: u16,
    pub name: String,
    pub manufacturer: String,
    pub model: String,
    pub protocol: String,
    pub xaddrs: Option<String>,
    pub rtsp_url: Option<String>,
}
