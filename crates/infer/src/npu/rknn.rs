//! Rockchip RKNN NPU 设备实现
//!
//! 支持 RK3568、RK3576、RK3588 等芯片的 NPU 监控。
//! 通过 `/sys/class/devfreq/` 读取 NPU 状态。

use std::path::{Path, PathBuf};

use super::{NpuCoreMetrics, NpuDevice, NpuDeviceMetrics, NpuDeviceType, NpuError};

/// RKNN NPU 核心信息
struct RknnCore {
    /// 核心索引
    index: u32,
    /// devfreq 路径
    devfreq_path: PathBuf,
}

/// RKNN NPU 设备
pub struct RknnNpuDevice {
    /// 设备 ID
    device_id: String,
    /// 核心列表
    cores: Vec<RknnCore>,
    /// 温度传感器路径
    thermal_path: Option<PathBuf>,
}

impl std::fmt::Debug for RknnNpuDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RknnNpuDevice")
            .field("device_id", &self.device_id)
            .field("core_count", &self.cores.len())
            .field("thermal_path", &self.thermal_path)
            .finish()
    }
}

/// 解析 devfreq load 输出
///
/// Linux/Rockchip devfreq load 格式为 "busy_time@total_time"（例如 "35000000@100000000"），
/// 利用率百分比为 (busy_time / total_time) * 100.0。
/// 若只有单一数字，则视作已计算的百分比。
pub(crate) fn parse_devfreq_load(content: &str) -> f64 {
    let trimmed = content.trim();
    if let Some((busy_str, total_str)) = trimmed.split_once('@') {
        let busy: f64 = busy_str.trim().parse().unwrap_or(0.0);
        let total_digits: String = total_str
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        let total: f64 = total_digits.parse().unwrap_or(0.0);
        if total > 0.0 {
            let pct = (busy / total) * 100.0;
            return (pct * 10.0).round() / 10.0;
        }
    } else if let Ok(val) = trimmed.parse::<f64>() {
        return (val.clamp(0.0, 100.0) * 10.0).round() / 10.0;
    }
    0.0
}

impl RknnNpuDevice {
    /// 创建 RKNN NPU 设备实例
    ///
    /// # Arguments
    /// * `core_paths` - 核心的 devfreq 路径列表
    fn new(core_paths: Vec<PathBuf>) -> Self {
        let cores: Vec<RknnCore> = core_paths
            .into_iter()
            .enumerate()
            .map(|(idx, path)| RknnCore {
                index: idx as u32,
                devfreq_path: path,
            })
            .collect();

        let device_id = match cores.len() {
            3 => "rk3588-npu",
            2 => "rk3576-npu",
            1 => "rk3568-npu",
            _ => "rknn-npu",
        }
        .to_string();

        // 尝试查找 NPU 温度传感器
        let thermal_path = Self::find_npu_thermal_zone();

        Self {
            device_id,
            cores,
            thermal_path,
        }
    }

    /// 查找 NPU 温度传感器
    fn find_npu_thermal_zone() -> Option<PathBuf> {
        let thermal_base = Path::new("/sys/class/thermal");
        if !thermal_base.exists() {
            return None;
        }

        // 遍历所有 thermal zone，查找类型为 "npu-thermal" 或 "npu" 的
        if let Ok(entries) = std::fs::read_dir(thermal_base) {
            for entry in entries.flatten() {
                let type_path = entry.path().join("type");
                if let Ok(type_str) = std::fs::read_to_string(&type_path) {
                    let type_str = type_str.trim().to_lowercase();
                    if type_str.contains("npu") || type_str.contains("nna") {
                        let temp_path = entry.path().join("temp");
                        if temp_path.exists() {
                            return Some(temp_path);
                        }
                    }
                }
            }
        }

        None
    }

    /// 读取 devfreq 负载
    fn read_devfreq_load(path: &Path) -> Result<f64, NpuError> {
        let load_path = path.join("load");
        let content = std::fs::read_to_string(&load_path).map_err(|e| {
            NpuError::IoError(format!("Failed to read {}: {}", load_path.display(), e))
        })?;

        Ok(parse_devfreq_load(&content))
    }

    /// 读取 devfreq 频率
    fn read_devfreq_freq(path: &Path) -> Result<u32, NpuError> {
        let freq_path = path.join("cur_freq");
        let content = std::fs::read_to_string(&freq_path).map_err(|e| {
            NpuError::IoError(format!("Failed to read {}: {}", freq_path.display(), e))
        })?;

        let freq_hz: u64 = content.trim().parse().unwrap_or(0);
        Ok((freq_hz / 1_000_000) as u32) // Hz -> MHz
    }

    /// 读取 devfreq 最大频率
    fn read_devfreq_max_freq(path: &Path) -> Result<u32, NpuError> {
        let freq_path = path.join("max_freq");
        let content = std::fs::read_to_string(&freq_path).map_err(|e| {
            NpuError::IoError(format!("Failed to read {}: {}", freq_path.display(), e))
        })?;

        let freq_hz: u64 = content.trim().parse().unwrap_or(0);
        Ok((freq_hz / 1_000_000) as u32) // Hz -> MHz
    }

    /// 读取温度传感器
    fn read_temperature(&self) -> Result<Option<f32>, NpuError> {
        if let Some(ref path) = self.thermal_path {
            let content = std::fs::read_to_string(path)
                .map_err(|e| NpuError::IoError(format!("Failed to read thermal: {}", e)))?;

            let temp_millideg: i32 = content.trim().parse().unwrap_or(0);
            Ok(Some(temp_millideg as f32 / 1000.0))
        } else {
            Ok(None)
        }
    }
}

impl NpuDevice for RknnNpuDevice {
    fn device_id(&self) -> &str {
        &self.device_id
    }

    fn device_type(&self) -> NpuDeviceType {
        NpuDeviceType::RockchipRknn
    }

    fn core_count(&self) -> u32 {
        self.cores.len() as u32
    }

    fn utilization(&self) -> Result<f64, NpuError> {
        if self.cores.is_empty() {
            return Ok(0.0);
        }

        let mut total_util = 0.0;
        for core in &self.cores {
            let load = Self::read_devfreq_load(&core.devfreq_path)?;
            total_util += load;
        }

        Ok(total_util / self.cores.len() as f64)
    }

    fn memory_info(&self) -> Result<(u64, u64, u64), NpuError> {
        // RK3588 NPU 内存通常与 DDR 共享，无法直接读取
        // 返回 0 表示不可用
        Ok((0, 0, 0))
    }

    fn temperature(&self) -> Result<Option<f32>, NpuError> {
        self.read_temperature()
    }

    fn frequency_mhz(&self) -> Result<(u32, u32), NpuError> {
        if self.cores.is_empty() {
            return Ok((0, 0));
        }

        // 返回第一个核心的频率信息
        let core = &self.cores[0];
        let current = Self::read_devfreq_freq(&core.devfreq_path)?;
        let max = Self::read_devfreq_max_freq(&core.devfreq_path)?;

        Ok((current, max))
    }

    fn active_sessions(&self) -> u32 {
        // 需要通过 RKNN Runtime API 获取，这里返回 0
        0
    }

    fn collect_metrics(&self) -> Result<NpuDeviceMetrics, NpuError> {
        let (total_mb, used_mb, reserved_mb) = self.memory_info()?;
        let temperature = self.temperature()?;

        let mut cores = Vec::new();
        for core in &self.cores {
            let utilization = Self::read_devfreq_load(&core.devfreq_path)?;
            let frequency = Self::read_devfreq_freq(&core.devfreq_path)?;

            cores.push(NpuCoreMetrics {
                core_id: core.index,
                utilization_percent: utilization,
                frequency_mhz: frequency,
                power_watts: None,
            });
        }

        Ok(NpuDeviceMetrics {
            device_id: self.device_id.clone(),
            device_type: NpuDeviceType::RockchipRknn,
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

/// 探测系统中的 RKNN 设备
///
/// 扫描 `/sys/class/devfreq/` 目录，查找 `ff100000.npu`、`ff110000.npu` 等
pub fn detect_devices() -> Result<Vec<Box<dyn NpuDevice>>, NpuError> {
    let devfreq_base = Path::new("/sys/class/devfreq");
    if !devfreq_base.exists() {
        return Ok(Vec::new());
    }

    let mut core_paths = Vec::new();

    // 扫描 devfreq 目录，查找 NPU 核心
    if let Ok(entries) = std::fs::read_dir(devfreq_base) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();

            // RK3588 NPU 核心命名: ff100000.npu, ff110000.npu, ff120000.npu
            // RK3568/RK3576: 类似模式
            if name_str.contains("npu") || name_str.contains("nna") {
                let load_path = entry.path().join("load");
                if load_path.exists() {
                    core_paths.push(entry.path());
                }
            }
        }
    }

    if core_paths.is_empty() {
        return Ok(Vec::new());
    }

    // 按路径排序，确保核心索引一致
    core_paths.sort();

    Ok(vec![Box::new(RknnNpuDevice::new(core_paths))])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rknn_device_type() {
        let device = RknnNpuDevice::new(vec![PathBuf::from("/sys/class/devfreq/ff100000.npu")]);
        assert_eq!(device.device_type(), NpuDeviceType::RockchipRknn);
    }

    #[test]
    fn test_rknn_core_count() {
        let device = RknnNpuDevice::new(vec![
            PathBuf::from("/sys/class/devfreq/ff100000.npu"),
            PathBuf::from("/sys/class/devfreq/ff110000.npu"),
            PathBuf::from("/sys/class/devfreq/ff120000.npu"),
        ]);
        assert_eq!(device.core_count(), 3);
        assert_eq!(device.device_id(), "rk3588-npu");
    }

    #[test]
    fn test_parse_devfreq_load() {
        assert_eq!(parse_devfreq_load("35000000@100000000"), 35.0);
        assert_eq!(parse_devfreq_load("82300000@100000000ns\n"), 82.3);
        assert_eq!(parse_devfreq_load("50.5"), 50.5);
        assert_eq!(parse_devfreq_load("0@100000"), 0.0);
        assert_eq!(parse_devfreq_load("invalid"), 0.0);
    }
}
