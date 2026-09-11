//! Rockchip RKNN NPU 设备实现
//!
//! 支持 RK3568、RK3576、RK3588 等芯片的 NPU 状态与多核利用率监控。
//! 优先通过 debugfs (`/sys/kernel/debug/rknpu/load`) 获取准确的单核/多核负载，
//! 并结合 devfreq 与 thermal 子系统采集频率与温度。

use std::path::{Path, PathBuf};

use super::{NpuCoreMetrics, NpuDevice, NpuDeviceMetrics, NpuDeviceType, NpuError};

/// RKNN NPU 设备
pub struct RknnNpuDevice {
    /// 设备 ID（如 "rk3588-npu", "rk3576-npu", "rk3568-npu"）
    device_id: String,
    /// 核心数量
    core_count: u32,
    /// 负载读取路径（优先 /sys/kernel/debug/rknpu/load，备选 /proc/rknpu/load）
    load_path: Option<PathBuf>,
    /// devfreq 路径（用于读取频率 cur_freq / max_freq）
    devfreq_path: Option<PathBuf>,
    /// 温度传感器路径
    thermal_path: Option<PathBuf>,
}

impl std::fmt::Debug for RknnNpuDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RknnNpuDevice")
            .field("device_id", &self.device_id)
            .field("core_count", &self.core_count)
            .field("load_path", &self.load_path)
            .field("devfreq_path", &self.devfreq_path)
            .field("thermal_path", &self.thermal_path)
            .finish()
    }
}

/// 剥离可能的前缀 "npu load:"（ASCII 大小写无关，字符边界安全）
fn strip_npu_load_prefix(s: &str) -> &str {
    let trimmed = s.trim_start();
    let pattern = "npu load:";
    if trimmed.len() >= pattern.len() && trimmed[..pattern.len()].eq_ignore_ascii_case(pattern) {
        return trimmed[pattern.len()..].trim();
    }
    // 兜底安全查找，严格保证切片索引位于 UTF-8 字符边界上
    let pattern_bytes = pattern.as_bytes();
    let bytes = trimmed.as_bytes();
    if bytes.len() >= pattern_bytes.len() {
        for i in 0..=bytes.len() - pattern_bytes.len() {
            if bytes[i..i + pattern_bytes.len()].eq_ignore_ascii_case(pattern_bytes) {
                let after = i + pattern_bytes.len();
                if trimmed.is_char_boundary(after) {
                    return trimmed[after..].trim();
                }
            }
        }
    }
    trimmed
}

/// 检查字符串是否包含目标子串（ASCII 大小写无关，零堆分配）
fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    let needle_bytes = needle.as_bytes();
    let haystack_bytes = haystack.as_bytes();
    if needle_bytes.is_empty() {
        return true;
    }
    if haystack_bytes.len() < needle_bytes.len() {
        return false;
    }
    haystack_bytes
        .windows(needle_bytes.len())
        .any(|w| w.eq_ignore_ascii_case(needle_bytes))
}

/// 解析 Rockchip RKNPU 驱动导出的负载信息
///
/// 官方 RKNPU 驱动在 `/sys/kernel/debug/rknpu/load` 输出以下格式：
/// 1. 多核模式（RK3588 3核、RK3576 2核）：
///    - `NPU load: Core0: 15%, Core1: 20%, Core2: 0%`
///    - `NPU load:  Core 0:  15%, Core 1:  20%, Core 2:   0%`
/// 2. 单核模式（RK3568 等）：
///    - `NPU load:  25%` 或 `NPU load: 25%`
///    - `NPU load: Core0: 25%`
/// 3. Devfreq 兼容格式：
///    - `35000000@100000000`
///    - `50.5`
pub(crate) fn parse_rknpu_load(content: &str) -> Vec<f64> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    // 剥离可能的前缀 "NPU load:"（ASCII 大小写无关，零堆分配）
    let body = strip_npu_load_prefix(trimmed);

    // 检查是否包含核心信息（"core" 或以逗号分割的多项）
    if contains_ignore_ascii_case(body, "core") || body.contains(',') {
        let mut core_utils = Vec::new();
        for segment in body.split(',') {
            let seg = segment.trim();
            if seg.is_empty() {
                continue;
            }
            // 提取冒号后或纯文本中的百分比数值
            let val_part = if let Some((_, right)) = seg.split_once(':') {
                right
            } else {
                seg
            };
            let num_str: String = val_part
                .chars()
                .filter(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if let Ok(pct) = num_str.parse::<f64>() {
                core_utils.push((pct.clamp(0.0, 100.0) * 10.0).round() / 10.0);
            }
        }
        if !core_utils.is_empty() {
            return core_utils;
        }
    }

    // 尝试单核百分比解析，例如 "25%" 或 " 25.5 % "
    if body.contains('%') {
        let num_str: String = body
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        if let Ok(pct) = num_str.parse::<f64>() {
            return vec![(pct.clamp(0.0, 100.0) * 10.0).round() / 10.0];
        }
    }

    // 回退到 devfreq busy_time@total_time 或纯浮点数
    let fallback = parse_devfreq_load(trimmed);
    vec![fallback]
}

/// 解析 devfreq load 输出（兼容历史与第三方格式）
///
/// 格式支持：
/// - "busy_time@total_time"（例如 "35000000@100000000"）
/// - 纯浮点数（例如 "50.5"）
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

/// 根据文本特征匹配 Rockchip SoC 型号及默认核心数
fn match_soc_model(text: &str) -> Option<(&'static str, u32)> {
    if contains_ignore_ascii_case(text, "rk3588") {
        Some(("rk3588-npu", 3))
    } else if contains_ignore_ascii_case(text, "rk3576") {
        Some(("rk3576-npu", 2))
    } else if contains_ignore_ascii_case(text, "rk3568")
        || contains_ignore_ascii_case(text, "rk3566")
    {
        Some(("rk3568-npu", 1))
    } else if contains_ignore_ascii_case(text, "rk3562") {
        Some(("rk3562-npu", 1))
    } else if contains_ignore_ascii_case(text, "rv1106")
        || contains_ignore_ascii_case(text, "rv1103")
    {
        Some(("rv1106-npu", 1))
    } else {
        None
    }
}

/// 从设备树或系统信息中探测 Rockchip SoC 型号及默认核心数
fn detect_soc_model() -> (&'static str, u32) {
    // 检查设备树 compatible 属性
    for path in &[
        "/proc/device-tree/compatible",
        "/sys/firmware/devicetree/base/compatible",
    ] {
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Some(soc) = match_soc_model(&content) {
                return soc;
            }
        }
    }

    // 回退检查 /proc/cpuinfo
    if let Ok(cpuinfo) = std::fs::read_to_string("/proc/cpuinfo") {
        if let Some(soc) = match_soc_model(&cpuinfo) {
            return soc;
        }
    }

    ("rknn-npu", 1)
}

/// 查找 RKNPU 负载读取节点
fn find_load_path() -> Option<PathBuf> {
    let candidates = ["/sys/kernel/debug/rknpu/load", "/proc/rknpu/load"];
    for candidate in candidates {
        let path = Path::new(candidate);
        if path.exists() {
            return Some(path.to_path_buf());
        }
    }
    None
}

/// 查找 NPU 的 devfreq 目录（用于获取当前与最大频率）
fn find_npu_devfreq_path() -> Option<PathBuf> {
    let devfreq_base = Path::new("/sys/class/devfreq");
    if !devfreq_base.exists() {
        return None;
    }

    if let Ok(entries) = std::fs::read_dir(devfreq_base) {
        for entry in entries.flatten() {
            let name_str = entry.file_name().to_string_lossy().to_lowercase();
            // RK3588: fdab0000.npu, RK3568: fde40000.npu, RK3576: 27c00000.npu
            if name_str.contains("npu") || name_str.contains("nna") || name_str.contains("rknpu") {
                let cur_freq = entry.path().join("cur_freq");
                if cur_freq.exists() {
                    return Some(entry.path());
                }
            }
        }
    }

    None
}

/// 查找 NPU 温度传感器节点
fn find_npu_thermal_zone() -> Option<PathBuf> {
    let thermal_base = Path::new("/sys/class/thermal");
    if !thermal_base.exists() {
        return None;
    }

    if let Ok(entries) = std::fs::read_dir(thermal_base) {
        for entry in entries.flatten() {
            let type_path = entry.path().join("type");
            if let Ok(type_str) = std::fs::read_to_string(&type_path) {
                if contains_ignore_ascii_case(&type_str, "npu")
                    || contains_ignore_ascii_case(&type_str, "nna")
                {
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

/// 读取 devfreq 目录下的当前频率与最大频率 (MHz)
fn read_devfreq_frequencies(devfreq_path: Option<&Path>) -> (u32, u32) {
    if let Some(path) = devfreq_path {
        let cur = std::fs::read_to_string(path.join("cur_freq"))
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .map(|hz| (hz / 1_000_000) as u32)
            .unwrap_or(0);
        let max = std::fs::read_to_string(path.join("max_freq"))
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .map(|hz| (hz / 1_000_000) as u32)
            .unwrap_or(cur);
        (cur, max)
    } else {
        (0, 0)
    }
}

impl RknnNpuDevice {
    /// 创建默认配置的 RKNN NPU 设备实例
    pub fn new(soc_id: impl Into<String>, core_count: u32) -> Self {
        Self {
            device_id: soc_id.into(),
            core_count,
            load_path: find_load_path(),
            devfreq_path: find_npu_devfreq_path(),
            thermal_path: find_npu_thermal_zone(),
        }
    }

    /// 使用指定路径创建 RKNN NPU 设备实例（便于单元测试与自定义挂载）
    pub fn with_paths(
        device_id: impl Into<String>,
        core_count: u32,
        load_path: Option<PathBuf>,
        devfreq_path: Option<PathBuf>,
        thermal_path: Option<PathBuf>,
    ) -> Self {
        Self {
            device_id: device_id.into(),
            core_count,
            load_path,
            devfreq_path,
            thermal_path,
        }
    }

    /// 读取各核心的利用率百分比
    pub(crate) fn read_core_utilizations(&self) -> Vec<f64> {
        // 1. 优先读取已配置的 load_path，未配置时动态探测一次（避免已配置路径故障时重复冗余 I/O）
        let dynamic_path;
        let candidate_path = match self.load_path.as_deref() {
            Some(p) => Some(p),
            None => {
                dynamic_path = find_load_path();
                dynamic_path.as_deref()
            }
        };

        if let Some(path) = candidate_path {
            if let Ok(content) = std::fs::read_to_string(path) {
                let utils = parse_rknpu_load(&content);
                if !utils.is_empty() {
                    return utils;
                }
            }
        }

        // 2. 兼容检查 devfreq/load
        if let Some(ref path) = self.devfreq_path {
            let devfreq_load_file = path.join("load");
            if let Ok(content) = std::fs::read_to_string(devfreq_load_file) {
                let util = parse_devfreq_load(&content);
                return vec![util];
            }
        }

        Vec::new()
    }
    /// 获取校准后的各核心利用率与有效核心数
    ///
    /// 核心校准逻辑：
    /// 1. 若驱动输出了更多核心，以驱动实际核心数为准；
    /// 2. 若驱动仅输出单一全局利用率，扩展至全部核心；
    /// 3. 若采集到部分核心（少于配置数），缺失核心补 0.0，杜绝克隆 Core 0；
    /// 4. 至少保证 1 个核心。
    pub(crate) fn calibrated_core_utilizations(&self) -> (Vec<f64>, usize) {
        let mut core_utils = self.read_core_utilizations();
        let count = (self.core_count as usize).max(core_utils.len()).max(1);
        if core_utils.len() < count {
            if core_utils.len() == 1 {
                let global_val = core_utils[0];
                core_utils.resize(count, global_val);
            } else {
                core_utils.resize(count, 0.0);
            }
        }
        (core_utils, count)
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
        self.core_count
    }

    fn utilization(&self) -> Result<f64, NpuError> {
        let (core_utils, count) = self.calibrated_core_utilizations();
        if core_utils.is_empty() {
            return Ok(0.0);
        }
        let sum: f64 = core_utils.iter().sum();
        let avg = sum / count as f64;
        Ok((avg * 10.0).round() / 10.0)
    }

    fn memory_info(&self) -> Result<(u64, u64, u64), NpuError> {
        // Rockchip NPU 与 CPU/GPU 共享物理 DDR，通常通过 CMA/DMA-BUF 分配，无专属显存
        Ok((0, 0, 0))
    }

    fn temperature(&self) -> Result<Option<f32>, NpuError> {
        if let Some(ref path) = self.thermal_path {
            let content = std::fs::read_to_string(path)
                .map_err(|e| NpuError::IoError(format!("Failed to read thermal: {}", e)))?;

            let temp_millideg: i32 = content.trim().parse().unwrap_or(0);
            Ok(Some(temp_millideg as f32 / 1000.0))
        } else {
            Ok(None)
        }
    }

    fn frequency_mhz(&self) -> Result<(u32, u32), NpuError> {
        Ok(read_devfreq_frequencies(self.devfreq_path.as_deref()))
    }

    fn active_sessions(&self) -> u32 {
        0
    }

    fn collect_metrics(&self) -> Result<NpuDeviceMetrics, NpuError> {
        let (total_mb, used_mb, reserved_mb) = self.memory_info()?;
        let temperature = self.temperature()?;
        let (current_freq, _max_freq) = self.frequency_mhz()?;

        let (core_utils, count) = self.calibrated_core_utilizations();

        let mut cores = Vec::with_capacity(count);
        for (idx, util) in core_utils.into_iter().enumerate().take(count) {
            cores.push(NpuCoreMetrics {
                core_id: idx as u32,
                utilization_percent: util,
                frequency_mhz: current_freq,
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
pub fn detect_devices() -> Result<Vec<Box<dyn NpuDevice>>, NpuError> {
    let has_rknpu_dev = Path::new("/dev/rknpu").exists();
    let has_debugfs =
        Path::new("/sys/kernel/debug/rknpu").exists() || Path::new("/proc/rknpu").exists();
    let devfreq_path = find_npu_devfreq_path();

    // 若无任何物理设备节点 (/dev/rknpu)、debugfs 或 devfreq 驱动节点，安全返回空列表
    if !has_rknpu_dev && !has_debugfs && devfreq_path.is_none() {
        return Ok(Vec::new());
    }

    let (soc_id, default_cores) = detect_soc_model();
    let load_path = find_load_path();
    let thermal_path = find_npu_thermal_zone();

    let device =
        RknnNpuDevice::with_paths(soc_id, default_cores, load_path, devfreq_path, thermal_path);

    Ok(vec![Box::new(device)])
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_rknn_device_type() {
        let device = RknnNpuDevice::new("rk3568-npu", 1);
        assert_eq!(device.device_type(), NpuDeviceType::RockchipRknn);
    }

    #[test]
    fn test_rknn_core_count() {
        let device = RknnNpuDevice::new("rk3588-npu", 3);
        assert_eq!(device.core_count(), 3);
        assert_eq!(device.device_id(), "rk3588-npu");
    }

    #[test]
    fn test_parse_rknpu_load_rk3588_multi_core() {
        // 格式 1: 带空格的多核输出
        let output1 = "NPU load:  Core 0:  15%, Core 1:  20%, Core 2:   0%\n";
        let utils1 = parse_rknpu_load(output1);
        assert_eq!(utils1, vec![15.0, 20.0, 0.0]);

        // 格式 2: 紧凑多核输出
        let output2 = "NPU load: Core0: 15%, Core1: 20%, Core2: 0%";
        let utils2 = parse_rknpu_load(output2);
        assert_eq!(utils2, vec![15.0, 20.0, 0.0]);

        // 格式 3: 零负载
        let output3 = "NPU load: Core0:  0%, Core1:  0%, Core2:  0%";
        let utils3 = parse_rknpu_load(output3);
        assert_eq!(utils3, vec![0.0, 0.0, 0.0]);

        // 格式 4: 高负载与浮点数
        let output4 = "NPU load: Core0: 98.5%, Core1: 100.0%, Core2: 45.2%";
        let utils4 = parse_rknpu_load(output4);
        assert_eq!(utils4, vec![98.5, 100.0, 45.2]);
    }

    #[test]
    fn test_parse_rknpu_load_rk3576_two_core() {
        let output = "NPU load:  Core 0:  10%, Core 1:  25%\n";
        let utils = parse_rknpu_load(output);
        assert_eq!(utils, vec![10.0, 25.0]);
    }

    #[test]
    fn test_parse_rknpu_load_rk3568_single_core() {
        // 单核百分比
        let output1 = "NPU load:  25%\n";
        assert_eq!(parse_rknpu_load(output1), vec![25.0]);

        let output2 = "NPU load: 25%";
        assert_eq!(parse_rknpu_load(output2), vec![25.0]);

        let output3 = "NPU load: Core0: 25%";
        assert_eq!(parse_rknpu_load(output3), vec![25.0]);

        let output4 = "NPU load: Core 0: 30%";
        assert_eq!(parse_rknpu_load(output4), vec![30.0]);
    }

    #[test]
    fn test_parse_devfreq_load_compat() {
        assert_eq!(parse_devfreq_load("35000000@100000000"), 35.0);
        assert_eq!(parse_devfreq_load("82300000@100000000ns\n"), 82.3);
        assert_eq!(parse_devfreq_load("50.5"), 50.5);
        assert_eq!(parse_devfreq_load("0@100000"), 0.0);
        assert_eq!(parse_devfreq_load("invalid"), 0.0);

        // parse_rknpu_load 兼容 devfreq 格式
        assert_eq!(parse_rknpu_load("35000000@100000000"), vec![35.0]);
        assert_eq!(parse_rknpu_load("50.5"), vec![50.5]);
        assert_eq!(parse_rknpu_load(""), Vec::<f64>::new());
    }

    #[test]
    fn test_rknn_device_metrics_collection_rk3588_e2e() {
        let temp_dir = std::env::temp_dir().join(format!("rknn_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);

        let load_file = temp_dir.join("load");
        let cur_freq_file = temp_dir.join("cur_freq");
        let max_freq_file = temp_dir.join("max_freq");
        let temp_file = temp_dir.join("temp");

        std::fs::write(
            &load_file,
            "NPU load:  Core 0:  15%, Core 1:  20%, Core 2:   0%\n",
        )
        .unwrap();
        std::fs::write(&cur_freq_file, "1000000000\n").unwrap();
        std::fs::write(&max_freq_file, "1000000000\n").unwrap();
        std::fs::write(&temp_file, "42500\n").unwrap();

        let device = RknnNpuDevice::with_paths(
            "rk3588-npu",
            3,
            Some(load_file),
            Some(temp_dir.clone()),
            Some(temp_file),
        );

        assert_eq!(device.device_id(), "rk3588-npu");
        assert_eq!(device.core_count(), 3);
        assert_eq!(device.frequency_mhz().unwrap(), (1000, 1000));
        assert_eq!(device.temperature().unwrap(), Some(42.5));

        // (15.0 + 20.0 + 0.0) / 3 = 11.666... 约 11.7
        assert_eq!(device.utilization().unwrap(), 11.7);

        let metrics = device.collect_metrics().unwrap();
        assert_eq!(metrics.device_id, "rk3588-npu");
        assert_eq!(metrics.device_type, NpuDeviceType::RockchipRknn);
        assert_eq!(metrics.cores.len(), 3);
        assert_eq!(metrics.cores[0].core_id, 0);
        assert_eq!(metrics.cores[0].utilization_percent, 15.0);
        assert_eq!(metrics.cores[0].frequency_mhz, 1000);
        assert_eq!(metrics.cores[1].core_id, 1);
        assert_eq!(metrics.cores[1].utilization_percent, 20.0);
        assert_eq!(metrics.cores[2].core_id, 2);
        assert_eq!(metrics.cores[2].utilization_percent, 0.0);
        assert_eq!(metrics.temperature, Some(42.5));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_rknn_device_metrics_collection_rk3568_e2e() {
        let temp_dir = std::env::temp_dir().join(format!("rknn_test_3568_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);

        let load_file = temp_dir.join("load");
        std::fs::write(&load_file, "NPU load:  35%\n").unwrap();

        let device = RknnNpuDevice::with_paths("rk3568-npu", 1, Some(load_file), None, None);

        assert_eq!(device.device_id(), "rk3568-npu");
        assert_eq!(device.core_count(), 1);
        assert_eq!(device.utilization().unwrap(), 35.0);

        let metrics = device.collect_metrics().unwrap();
        assert_eq!(metrics.cores.len(), 1);
        assert_eq!(metrics.cores[0].core_id, 0);
        assert_eq!(metrics.cores[0].utilization_percent, 35.0);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_parse_rknpu_load_mixed_case() {
        let output = "nPu LoAd: CoRe 0: 45.5%, cOrE 1: 60.0%\n";
        assert_eq!(parse_rknpu_load(output), vec![45.5, 60.0]);
    }

    #[test]
    fn test_match_soc_model() {
        assert_eq!(match_soc_model("rockchip,rk3588"), Some(("rk3588-npu", 3)));
        assert_eq!(
            match_soc_model("rockchip,rk3576-evb"),
            Some(("rk3576-npu", 2))
        );
        assert_eq!(match_soc_model("rockchip,rk3568"), Some(("rk3568-npu", 1)));
        assert_eq!(match_soc_model("rockchip,rk3566"), Some(("rk3568-npu", 1)));
        assert_eq!(match_soc_model("rockchip,rv1106"), Some(("rv1106-npu", 1)));
        assert_eq!(match_soc_model("allwinner,h616"), None);
    }

    #[test]
    fn test_rknn_device_metrics_collection_partial_core_zero_fill() {
        let temp_dir =
            std::env::temp_dir().join(format!("rknn_test_partial_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);

        let load_file = temp_dir.join("load");
        // 驱动仅返回了 2 个核心，但设备配置为 3 核
        std::fs::write(&load_file, "NPU load: Core0: 15%, Core1: 20%\n").unwrap();

        let device = RknnNpuDevice::with_paths("rk3588-npu", 3, Some(load_file), None, None);

        let metrics = device.collect_metrics().unwrap();
        assert_eq!(metrics.cores.len(), 3);
        assert_eq!(metrics.cores[0].core_id, 0);
        assert_eq!(metrics.cores[0].utilization_percent, 15.0);
        assert_eq!(metrics.cores[1].core_id, 1);
        assert_eq!(metrics.cores[1].utilization_percent, 20.0);
        // 缺失的核心必须填 0.0，杜绝克隆 Core 0 的 15.0
        assert_eq!(metrics.cores[2].core_id, 2);
        assert_eq!(metrics.cores[2].utilization_percent, 0.0);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_rknn_device_metrics_collection_single_fallback_expansion() {
        let temp_dir =
            std::env::temp_dir().join(format!("rknn_test_single_fallback_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);

        let load_file = temp_dir.join("load");
        // 驱动返回了全局单负载
        std::fs::write(&load_file, "NPU load: 50%\n").unwrap();

        let device = RknnNpuDevice::with_paths("rk3588-npu", 3, Some(load_file), None, None);

        let metrics = device.collect_metrics().unwrap();
        assert_eq!(metrics.cores.len(), 3);
        // 全局单负载应扩展至所有核心
        assert_eq!(metrics.cores[0].utilization_percent, 50.0);
        assert_eq!(metrics.cores[1].utilization_percent, 50.0);
        assert_eq!(metrics.cores[2].utilization_percent, 50.0);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_strip_npu_load_prefix_utf8_safe() {
        // 前缀带有中文字符或非 ASCII 字符，不应 panic 且能正常处理
        let s1 = "测试文本 NPU load: Core 0: 25%";
        assert_eq!(strip_npu_load_prefix(s1), "Core 0: 25%");

        let s2 = "   NPU LOAD: 30%";
        assert_eq!(strip_npu_load_prefix(s2), "30%");

        let s3 = "纯中文字符串无前缀";
        assert_eq!(strip_npu_load_prefix(s3), "纯中文字符串无前缀");
    }

    #[test]
    fn test_utilization_matches_overall_utilization_on_partial_cores() {
        let temp_dir =
            std::env::temp_dir().join(format!("rknn_test_util_match_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);

        let load_file = temp_dir.join("load");
        // 3 核设备但仅上报 2 个核心：15% 和 20%
        // 校准后为 [15.0, 20.0, 0.0]，平均值为 (15 + 20 + 0) / 3 = 11.666... -> 11.7%
        std::fs::write(&load_file, "NPU load: Core0: 15%, Core1: 20%\n").unwrap();

        let device = RknnNpuDevice::with_paths("rk3588-npu", 3, Some(load_file), None, None);

        let util = device.utilization().unwrap();
        assert_eq!(util, 11.7);

        let metrics = device.collect_metrics().unwrap();
        let overall = (metrics
            .cores
            .iter()
            .map(|c| c.utilization_percent)
            .sum::<f64>()
            / metrics.cores.len() as f64
            * 10.0)
            .round()
            / 10.0;
        assert_eq!(util, overall);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_detect_devices_on_host() {
        // 在开发机/非 Rockchip 目标板上，设备探测应优雅返回，不 panic
        let result = detect_devices();
        assert!(result.is_ok());
    }
}
