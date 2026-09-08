//! NPU 设备抽象层
//!
//! 提供跨平台的 NPU 设备监控能力，支持：
//! - Rockchip RKNN（多核心）
//! - 华为 Ascend（ACL API）
//! - 其他平台的降级处理

#[cfg(target_os = "linux")]
pub mod ascend;
pub mod monitor;
#[cfg(target_os = "linux")]
pub mod rknn;

use std::fmt;

/// NPU 设备类型
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum NpuDeviceType {
    /// 未知设备类型
    #[default]
    Unknown,
    /// Rockchip RKNN NPU
    RockchipRknn,
    /// 华为 Ascend NPU
    AscendAcl,
}

impl fmt::Display for NpuDeviceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RockchipRknn => write!(f, "rknn"),
            Self::AscendAcl => write!(f, "ascend"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

/// NPU 设备错误
#[derive(Debug, Clone)]
pub enum NpuError {
    /// 设备未找到
    DeviceNotFound,
    /// 无效的核心索引
    InvalidCoreIndex(u32),
    /// I/O 错误
    IoError(String),
    /// 权限不足
    PermissionDenied,
    /// 驱动不支持
    DriverNotSupported,
    /// ACL 错误码
    AclError(i32),
    /// 其他错误
    Other(String),
}

impl fmt::Display for NpuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeviceNotFound => write!(f, "NPU device not found"),
            Self::InvalidCoreIndex(idx) => write!(f, "Invalid core index: {}", idx),
            Self::IoError(msg) => write!(f, "I/O error: {}", msg),
            Self::PermissionDenied => write!(f, "Permission denied"),
            Self::DriverNotSupported => write!(f, "Driver not supported"),
            Self::AclError(code) => write!(f, "ACL error: {}", code),
            Self::Other(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for NpuError {}

/// NPU 核心指标
#[derive(Debug, Clone, Default)]
pub struct NpuCoreMetrics {
    /// 核心索引
    pub core_id: u32,
    /// 利用率百分比 (0.0 - 100.0)
    pub utilization_percent: f64,
    /// 当前频率 (MHz)
    pub frequency_mhz: u32,
    /// 功耗 (瓦特)，如不可用则为 None
    pub power_watts: Option<f32>,
}

/// NPU 设备指标
#[derive(Debug, Clone, Default)]
pub struct NpuDeviceMetrics {
    /// 设备 ID（如 "rk3588-npu0", "ascend-310b-0"）
    pub device_id: String,
    /// 设备类型
    pub device_type: NpuDeviceType,
    /// 各核心指标
    pub cores: Vec<NpuCoreMetrics>,
    /// 总内存 (MB)
    pub total_memory_mb: u64,
    /// 已用内存 (MB)
    pub used_memory_mb: u64,
    /// 预留内存 (MB)
    pub reserved_memory_mb: u64,
    /// 温度 (摄氏度)，如不可用则为 None
    pub temperature: Option<f32>,
    /// 当前并发推理会话数
    pub active_sessions: u32,
    /// 累计推理次数
    pub inference_count: u64,
}

/// NPU 设备抽象 trait
///
/// 所有 NPU 平台实现此 trait，提供统一的监控接口。
/// 实现者必须确保：
/// - 所有方法是非阻塞的（内部可使用 spawn_blocking）
/// - 采集失败时返回合理的默认值或错误，不 panic
/// - 符合各自平台的驱动规范
pub trait NpuDevice: Send + Sync {
    /// 获取设备唯一标识
    fn device_id(&self) -> &str;

    /// 获取设备类型
    fn device_type(&self) -> NpuDeviceType;

    /// 获取核心数量
    fn core_count(&self) -> u32;

    /// 获取设备利用率 (0.0 - 100.0)
    ///
    /// 返回所有核心的加权平均值或主核心利用率
    fn utilization(&self) -> Result<f64, NpuError>;

    /// 获取设备内存信息
    fn memory_info(&self) -> Result<(u64, u64, u64), NpuError>;

    /// 获取设备温度 (摄氏度)
    fn temperature(&self) -> Result<Option<f32>, NpuError>;

    /// 获取设备频率信息 (MHz)
    ///
    /// 返回 (当前频率, 最大频率)
    fn frequency_mhz(&self) -> Result<(u32, u32), NpuError>;

    /// 获取当前并发推理会话数
    fn active_sessions(&self) -> u32;

    /// 收集完整的设备指标
    fn collect_metrics(&self) -> Result<NpuDeviceMetrics, NpuError> {
        let (total_mb, used_mb, reserved_mb) = self.memory_info()?;
        let utilization = self.utilization()?;
        let temperature = self.temperature()?;
        let (current_freq, _max_freq) = self.frequency_mhz()?;

        // 构建核心指标（简化版，具体实现在各平台模块）
        let cores = (0..self.core_count())
            .map(|i| NpuCoreMetrics {
                core_id: i,
                utilization_percent: utilization,
                frequency_mhz: current_freq,
                power_watts: None,
            })
            .collect();

        Ok(NpuDeviceMetrics {
            device_id: self.device_id().to_string(),
            device_type: self.device_type(),
            cores,
            total_memory_mb: total_mb,
            used_memory_mb: used_mb,
            reserved_memory_mb: reserved_mb,
            temperature,
            active_sessions: self.active_sessions(),
            inference_count: 0,
        })
    }
}

/// 自动探测系统中的 NPU 设备
///
/// 按优先级探测：
/// 1. Rockchip RKNN
/// 2. 华为 Ascend
/// 3. 返回空列表（无 NPU 设备）
pub fn detect_npu_devices() -> Vec<Box<dyn NpuDevice>> {
    #[cfg(target_os = "linux")]
    {
        let mut devices: Vec<Box<dyn NpuDevice>> = Vec::new();

        if let Ok(rknn_devices) = rknn::detect_devices() {
            devices.extend(rknn_devices);
        }

        if let Ok(ascend_devices) = ascend::detect_devices() {
            devices.extend(ascend_devices);
        }

        devices
    }

    #[cfg(not(target_os = "linux"))]
    {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_npu_device_type_display() {
        assert_eq!(NpuDeviceType::RockchipRknn.to_string(), "rknn");
        assert_eq!(NpuDeviceType::AscendAcl.to_string(), "ascend");
        assert_eq!(NpuDeviceType::Unknown.to_string(), "unknown");
    }

    #[test]
    fn test_detect_npu_devices_empty_on_non_linux() {
        // 在测试环境中（通常是 macOS），探测应返回空列表
        let devices = detect_npu_devices();
        assert!(devices.is_empty());
    }

    #[test]
    fn test_npu_error_display() {
        assert_eq!(NpuError::DeviceNotFound.to_string(), "NPU device not found");
        assert_eq!(
            NpuError::InvalidCoreIndex(5).to_string(),
            "Invalid core index: 5"
        );
    }
}
