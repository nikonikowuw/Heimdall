//! 华为 Ascend NPU 设备实现
//!
//! 支持 Ascend 310B、310P、910B 等芯片的 NPU 监控。
//! 通过 Ascend Computing Language (ACL) API 或 sysfs 读取状态。

use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use super::{NpuCoreMetrics, NpuDevice, NpuDeviceMetrics, NpuDeviceType, NpuError};

/// Ascend 设备信息
struct AscendDevice {
    /// 设备索引
    index: u32,
    /// 设备名称
    name: String,
    /// AI Core 数量
    ai_core_count: u32,
    /// 活跃会话计数
    active_sessions: Arc<AtomicU32>,
}

/// Ascend NPU 设备
pub struct AscendNpuDevice {
    /// 设备列表
    devices: Vec<AscendDevice>,
}

impl std::fmt::Debug for AscendNpuDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AscendNpuDevice")
            .field("device_count", &self.devices.len())
            .finish()
    }
}

impl AscendNpuDevice {
    /// 创建 Ascend NPU 设备实例
    fn new(devices: Vec<AscendDevice>) -> Self {
        Self { devices }
    }

    /// 读取 Ascend 设备统计（优先 npu-smi，其次 devfreq）
    fn read_device_stats(index: u32) -> Option<(f64, u64, u64)> {
        // 1. 优先尝试从 npu-smi 提取真实设备统计
        if let Ok(output) = std::process::Command::new("npu-smi")
            .args(["info", "-t", "usages", "-i", &index.to_string()])
            .output()
        {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                let mut util = 0.0;
                let mut used_mb = 0u64;
                let mut total_mb = 0u64;

                for line in text.lines() {
                    let line_lower = line.to_lowercase();
                    if line_lower.contains("aicore") || line_lower.contains("ai core") {
                        if let Some(val_str) = line.split(':').nth(1) {
                            util = val_str
                                .trim()
                                .trim_end_matches('%')
                                .trim()
                                .parse()
                                .unwrap_or(0.0);
                        }
                    } else if line_lower.contains("memory usage")
                        || line_lower.contains("used memory")
                    {
                        if let Some(val_str) = line.split(':').nth(1) {
                            used_mb = val_str
                                .trim()
                                .trim_end_matches("MB")
                                .trim()
                                .parse()
                                .unwrap_or(0);
                        }
                    } else if line_lower.contains("memory capacity")
                        || line_lower.contains("total memory")
                    {
                        if let Some(val_str) = line.split(':').nth(1) {
                            total_mb = val_str
                                .trim()
                                .trim_end_matches("MB")
                                .trim()
                                .parse()
                                .unwrap_or(0);
                        }
                    }
                }
                return Some((util, used_mb, total_mb));
            }
        }

        // 2. 尝试从 /sys/class/devfreq/davinci{index}/load 提取
        let devfreq_load = format!("/sys/class/devfreq/davinci{}/load", index);
        if let Ok(content) = std::fs::read_to_string(&devfreq_load) {
            let util = super::rknn::parse_devfreq_load(&content);
            return Some((util, 0, 0));
        }

        None
    }

    /// 读取温度传感器
    fn read_temperature(index: u32) -> Result<Option<f32>, NpuError> {
        // 1. 尝试 npu-smi
        if let Ok(output) = std::process::Command::new("npu-smi")
            .args(["info", "-t", "temp", "-i", &index.to_string()])
            .output()
        {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                for line in text.lines() {
                    let lower = line.to_lowercase();
                    if lower.contains("temperature") || lower.contains("temp") {
                        if let Some(val_str) = line.split(':').nth(1) {
                            if let Ok(temp) = val_str
                                .trim()
                                .trim_end_matches('C')
                                .trim_end_matches('°')
                                .trim()
                                .parse::<f32>()
                            {
                                return Ok(Some(temp));
                            }
                        }
                    }
                }
            }
        }

        // 2. 尝试 Ascend devfreq 或 thermal zone
        let thermal_paths = [
            format!("/sys/class/devfreq/davinci{}/temp", index),
            format!("/sys/class/thermal/thermal_zone{}/temp", index),
        ];

        for path in &thermal_paths {
            if let Ok(content) = std::fs::read_to_string(path) {
                if let Ok(temp_millideg) = content.trim().parse::<i32>() {
                    let temp = temp_millideg as f32 / 1000.0;
                    if (20.0..120.0).contains(&temp) {
                        return Ok(Some(temp));
                    }
                }
            }
        }

        Ok(None)
    }

    /// 读取频率信息
    fn read_frequency(index: u32) -> Result<(u32, u32), NpuError> {
        let freq_path = format!("/sys/class/devfreq/davinci{}/cur_freq", index);
        let max_freq_path = format!("/sys/class/devfreq/davinci{}/max_freq", index);

        let current = if let Ok(content) = std::fs::read_to_string(&freq_path) {
            content.trim().parse::<u64>().unwrap_or(0) / 1_000_000 // Hz -> MHz
        } else {
            0
        };

        let max = if let Ok(content) = std::fs::read_to_string(&max_freq_path) {
            content.trim().parse::<u64>().unwrap_or(0) / 1_000_000 // Hz -> MHz
        } else {
            0
        };

        Ok((current as u32, max as u32))
    }
}

impl NpuDevice for AscendNpuDevice {
    fn device_id(&self) -> &str {
        // 返回第一个设备的名称
        self.devices
            .first()
            .map(|d| d.name.as_str())
            .unwrap_or("ascend-unknown")
    }

    fn device_type(&self) -> NpuDeviceType {
        NpuDeviceType::AscendAcl
    }

    fn core_count(&self) -> u32 {
        // Ascend 设备的 AI Core 数量
        self.devices.iter().map(|d| d.ai_core_count).sum()
    }

    fn utilization(&self) -> Result<f64, NpuError> {
        let mut total_util = 0.0;
        let mut count = 0;

        for device in &self.devices {
            if let Some((utilization, _, _)) = Self::read_device_stats(device.index) {
                total_util += utilization;
                count += 1;
            }
        }

        if count > 0 {
            Ok(total_util / count as f64)
        } else {
            Ok(0.0)
        }
    }

    fn memory_info(&self) -> Result<(u64, u64, u64), NpuError> {
        let mut total_mb = 0u64;
        let mut used_mb = 0u64;

        for device in &self.devices {
            if let Some((_, used, total)) = Self::read_device_stats(device.index) {
                total_mb += total;
                used_mb += used;
            }
        }

        Ok((total_mb, used_mb, 0))
    }

    fn temperature(&self) -> Result<Option<f32>, NpuError> {
        // 返回最高温度
        let mut max_temp: Option<f32> = None;

        for device in &self.devices {
            if let Some(temp) = Self::read_temperature(device.index)? {
                max_temp = Some(match max_temp {
                    Some(t) => t.max(temp),
                    None => temp,
                });
            }
        }

        Ok(max_temp)
    }

    fn frequency_mhz(&self) -> Result<(u32, u32), NpuError> {
        // 返回第一个设备的频率信息
        if let Some(device) = self.devices.first() {
            Self::read_frequency(device.index)
        } else {
            Ok((0, 0))
        }
    }

    fn active_sessions(&self) -> u32 {
        self.devices
            .first()
            .map(|d| d.active_sessions.load(Ordering::Relaxed))
            .unwrap_or(0)
    }

    fn collect_metrics(&self) -> Result<NpuDeviceMetrics, NpuError> {
        let temperature = self.temperature()?;
        let (current_freq, _max_freq) = self.frequency_mhz()?;

        let mut total_mb = 0u64;
        let mut used_mb = 0u64;
        let mut cores = Vec::new();
        for device in &self.devices {
            let (utilization, used, total) =
                Self::read_device_stats(device.index).unwrap_or((0.0, 0, 0));
            total_mb += total;
            used_mb += used;

            for core_idx in 0..device.ai_core_count {
                cores.push(NpuCoreMetrics {
                    core_id: core_idx,
                    utilization_percent: utilization,
                    frequency_mhz: current_freq,
                    power_watts: None,
                });
            }
        }

        let device_id = if let Some(device) = self.devices.first() {
            device.name.clone()
        } else {
            "ascend-unknown".to_string()
        };

        Ok(NpuDeviceMetrics {
            device_id,
            device_type: NpuDeviceType::AscendAcl,
            cores,
            total_memory_mb: total_mb,
            used_memory_mb: used_mb,
            reserved_memory_mb: 0,
            temperature,
            active_sessions: self.active_sessions(),
            inference_count: 0,
        })
    }
}

/// 探测系统中的 Ascend 设备
///
/// 扫描 `/dev/davinci*` 设备节点
pub fn detect_devices() -> Result<Vec<Box<dyn NpuDevice>>, NpuError> {
    let mut devices = Vec::new();

    // 扫描 davinci 设备节点
    for idx in 0..8 {
        let dev_path = format!("/dev/davinci{}", idx);
        if Path::new(&dev_path).exists() {
            // 读取设备信息
            let model = get_device_model(idx).unwrap_or_else(|| "unknown".to_string());
            let name = format!("ascend-{}-{}", idx, model);
            let ai_core_count = get_ai_core_count(idx).unwrap_or(8); // 默认 8 个 AI Core

            let active_sessions = Arc::new(AtomicU32::new(0));

            devices.push(AscendDevice {
                index: idx,
                name,
                ai_core_count,
                active_sessions,
            });
        }
    }

    if devices.is_empty() {
        return Ok(Vec::new());
    }

    Ok(vec![Box::new(AscendNpuDevice::new(devices))])
}

/// 获取设备型号
fn get_device_model(index: u32) -> Option<String> {
    let model_path = format!("/sys/class/devfreq/davinci{}/device/model", index);
    std::fs::read_to_string(&model_path)
        .ok()
        .map(|s| s.trim().to_string())
}

/// 获取 AI Core 数量
fn get_ai_core_count(index: u32) -> Option<u32> {
    let count_path = format!("/sys/class/devfreq/davinci{}/device/ai_core_count", index);
    std::fs::read_to_string(&count_path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ascend_device_type() {
        let device = AscendNpuDevice::new(vec![AscendDevice {
            index: 0,
            name: "ascend-310b".to_string(),
            ai_core_count: 8,
            active_sessions: Arc::new(AtomicU32::new(0)),
        }]);
        assert_eq!(device.device_type(), NpuDeviceType::AscendAcl);
    }

    #[test]
    fn test_ascend_core_count() {
        let device = AscendNpuDevice::new(vec![
            AscendDevice {
                index: 0,
                name: "ascend-310b-0".to_string(),
                ai_core_count: 8,
                active_sessions: Arc::new(AtomicU32::new(0)),
            },
            AscendDevice {
                index: 1,
                name: "ascend-310b-1".to_string(),
                ai_core_count: 8,
                active_sessions: Arc::new(AtomicU32::new(0)),
            },
        ]);
        assert_eq!(device.core_count(), 16);
    }
}
