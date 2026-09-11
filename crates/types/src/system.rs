use serde::{Deserialize, Serialize};

// ─── 系统概览（只读） ───

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemOverview {
    pub software_version: String,
    pub device_model: String,
    pub os_info: String,
    pub kernel_version: String,
    pub uptime_seconds: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ip_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mac_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_interface: Option<String>,
    // === 保留旧字段以向后兼容 ===
    pub cpu_usage_percent: f64,
    pub memory_usage_percent: f64,
    pub memory_used_mb: u64,
    pub memory_total_mb: u64,
    pub npu_usage_percent: Option<f64>,
    pub disk_usage_percent: f64,
    pub disk_used_gb: f64,
    pub disk_total_gb: f64,
    pub active_cameras: u32,
    pub total_cameras: u32,
    pub active_tasks: u32,
    pub today_alarms: u32,
    pub today_captures: u32,
    // === 新增字段 ===
    pub cpu: CpuMetrics,
    pub memory: MemoryMetrics,
    pub npu: Option<NpuMetrics>,
    pub network: Vec<NetworkInterfaceMetrics>,
    pub thermal: ThermalMetrics,
    pub disk: DiskMetrics,
}

// ─── CPU 指标 ───

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuMetrics {
    pub overall_percent: f64,
    pub per_core: Vec<CoreMetrics>,
    pub temperature: Option<f32>,
    pub frequency_mhz: Option<u32>,
    pub top_processes: Vec<ProcessMetrics>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreMetrics {
    pub core_id: u32,
    pub usage_percent: f64,
    pub frequency_mhz: Option<u32>,
    pub temperature: Option<f32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessMetrics {
    pub pid: u32,
    pub name: String,
    pub cpu_percent: f64,
    pub memory_mb: u64,
}

// ─── 内存指标 ───

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryMetrics {
    pub total_mb: u64,
    pub used_mb: u64,
    pub available_mb: u64,
    pub cached_mb: u64,
    pub buffer_mb: u64,
    pub swap_total_mb: u64,
    pub swap_used_mb: u64,
}

// ─── NPU 指标 ───

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NpuMetrics {
    pub device_type: String,
    pub cores: Vec<NpuCoreMetrics>,
    pub total_memory_mb: u64,
    pub used_memory_mb: u64,
    pub temperature: Option<f32>,
    pub active_sessions: u32,
    pub inference_count: u64,
}

impl NpuMetrics {
    /// 所有核心的平均利用率百分比，无核心时返回 None
    pub fn avg_utilization(&self) -> Option<f64> {
        if self.cores.is_empty() {
            None
        } else {
            let sum: f64 = self.cores.iter().map(|c| c.utilization_percent).sum();
            Some(sum / self.cores.len() as f64)
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NpuCoreMetrics {
    pub core_id: u32,
    pub utilization_percent: f64,
    pub frequency_mhz: u32,
    pub power_watts: Option<f32>,
}

// ─── 网络接口指标 ───

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkInterfaceMetrics {
    pub name: String,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_packets: u64,
    pub tx_packets: u64,
    pub rx_errors: u64,
    pub tx_errors: u64,
    pub rx_dropped: u64,
    pub tx_dropped: u64,
    pub speed_mbps: Option<u32>,
    pub link_up: bool,
}

// ─── 温度指标 ───

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThermalMetrics {
    pub zones: Vec<ThermalZone>,
    pub throttle_active: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThermalZone {
    pub name: String,
    pub temperature: f32,
    pub type_label: String,
}

// ─── 磁盘指标 ───

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskMetrics {
    pub total_gb: f64,
    pub used_gb: f64,
    pub available_gb: f64,
    pub inode_total: u64,
    pub inode_used: u64,
    pub inode_available: u64,
}

// ─── 存储配置 ───

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageConfig {
    pub alarm_retention_days: u32,
    pub alarm_quota_mb: u64,
    pub recognition_retention_days: u32,
    pub recognition_quota_mb: u64,
    pub capture_retention_days: u32,
    pub capture_quota_mb: u64,
    pub overwrite_mode: OverwriteMode,
    pub auto_cleanup_enabled: bool,
    pub min_free_ratio: f64,
    pub target_free_ratio: f64,
    pub emergency_free_ratio: f64,
    pub critical_free_ratio: f64,
    pub batch_delete_size: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OverwriteMode {
    Overwrite,
    Stop,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            alarm_retention_days: 30,
            alarm_quota_mb: 0,
            recognition_retention_days: 14,
            recognition_quota_mb: 0,
            capture_retention_days: 7,
            capture_quota_mb: 0,
            overwrite_mode: OverwriteMode::Overwrite,
            auto_cleanup_enabled: true,
            min_free_ratio: 0.15,
            target_free_ratio: 0.25,
            emergency_free_ratio: 0.08,
            critical_free_ratio: 0.05,
            batch_delete_size: 100,
        }
    }
}

impl StorageConfig {
    /// 校验水位层级关系
    pub fn validate(&self) -> Result<(), String> {
        if self.target_free_ratio <= self.min_free_ratio {
            return Err("target_free_ratio must be greater than min_free_ratio".into());
        }
        if self.emergency_free_ratio >= self.min_free_ratio {
            return Err("emergency_free_ratio must be less than min_free_ratio".into());
        }
        if self.critical_free_ratio >= self.emergency_free_ratio {
            return Err("critical_free_ratio must be less than emergency_free_ratio".into());
        }
        if self.alarm_retention_days == 0 || self.alarm_retention_days > 365 {
            return Err("alarm_retention_days must be 1-365".into());
        }
        if self.recognition_retention_days == 0 || self.recognition_retention_days > 365 {
            return Err("recognition_retention_days must be 1-365".into());
        }
        if self.capture_retention_days == 0 || self.capture_retention_days > 365 {
            return Err("capture_retention_days must be 1-365".into());
        }
        if self.batch_delete_size < 10 || self.batch_delete_size > 500 {
            return Err("batch_delete_size must be 10-500".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageStatus {
    pub total_gb: f64,
    pub used_gb: f64,
    pub available_gb: f64,
    pub usage_percent: f64,
    pub health_level: StorageHealthLevel,
    pub alarm_count: u32,
    pub alarm_size_mb: f64,
    pub recognition_count: u32,
    pub recognition_size_mb: f64,
    pub capture_count: u32,
    pub capture_size_mb: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum StorageHealthLevel {
    Normal,
    Evicting,
    Emergency,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvictionReport {
    pub deleted_count: u32,
    pub freed_mb: f64,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForceSyncResponse {
    pub synced: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetTimeResponse {
    pub applied: bool,
    pub previous_time: i64,
    pub new_time: i64,
}

// ─── 对时服务 ───

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeStatus {
    pub system_time: i64,
    pub timezone: String,
    pub timezone_offset: i64,
    pub ntp_synced: bool,
    pub ntp_service: String,
    pub ntp_server: Option<String>,
    pub offset_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimeConfig {
    pub ntp_enabled: bool,
    pub ntp_server: String,
    pub timezone: String,
}

impl Default for TimeConfig {
    fn default() -> Self {
        Self {
            ntp_enabled: true,
            ntp_server: "pool.ntp.org".to_string(),
            timezone: "Asia/Shanghai".to_string(),
        }
    }
}

// ─── 网络配置 ───

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkInterface {
    pub name: String,
    #[serde(rename = "type")]
    pub interface_type: NetworkInterfaceType,
    pub state: NetworkInterfaceState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carrier: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duplex: Option<String>,
    pub mac: String,
    pub manager: NetworkManager,
    pub ipv4: Option<IpConfig>,
    pub capabilities: InterfaceCapabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NetworkInterfaceType {
    Ethernet,
    Wifi,
    Loopback,
    Virtual,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NetworkInterfaceState {
    Up,
    Down,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NetworkManager {
    Networkmanager,
    SystemdNetworkd,
    Netplan,
    Ifupdown,
    Connman,
    Unmanaged,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IpConfig {
    pub method: IpMethod,
    pub address: Option<String>,
    pub prefix: Option<u32>,
    pub gateway: Option<String>,
    pub dns: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metric: Option<u32>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum IpMethod {
    Dhcp,
    Static,
    #[default]
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterfaceCapabilities {
    pub can_modify_ip: bool,
    pub can_set_dhcp: bool,
    pub can_set_static: bool,
    pub is_management_interface: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkInterfacesResponse {
    pub interfaces: Vec<NetworkInterface>,
    pub pending_operation: Option<NetworkChangeOperation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkUpdateResult {
    pub applied: bool,
    pub operation: Option<NetworkChangeOperation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkChangeOperation {
    pub id: String,
    pub status: OperationStatus,
    pub interface_name: String,
    pub old_config: IpConfig,
    pub new_config: IpConfig,
    pub created_at: i64,
    pub confirm_deadline_ms: i64,
    pub new_access_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OperationStatus {
    PendingConfirm,
    Confirmed,
    Restoring,
    Restored,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationConfirmResult {
    pub status: OperationStatus,
    pub confirmed_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkDiagnosticRequest {
    pub target: String,
    pub diagnostic_type: NetworkDiagnosticType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interface: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NetworkDiagnosticType {
    Ping,
    Dns,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkDiagnosticResult {
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<f64>,
    pub message: String,
}
