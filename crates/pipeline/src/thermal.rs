//! 工业级边缘温控与热反压降载引擎 (Thermal Guard & Load Shedder)
//!
//! 适配嵌入式 Linux (如 Rockchip RK3588 / 华为昇腾 310B / Atlas 200I 等无风扇被动散热工控盒)：
//! 1. 周期性采样 `/sys/class/thermal/thermal_zone*/temp` 核心结温；
//! 2. 梯级温控决策：
//!    - 正常状态 (< 75℃)：0% 负载衰减，全速推理；
//!    - 预警降频 (75℃ ~ 85℃)：50% 阶梯跳帧降载 (1/2 帧率)；
//!    - 临界过温 (>= 85℃)：75% 应急削峰降载 (1/4 帧率)，全力防范 SoC 硬件触发不可逆的热关机 (Thermal Shutdown)。

use std::path::{Path, PathBuf};

/// 硬件热状态级别
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThermalLevel {
    /// 正常温度 (< 75℃)
    Normal,
    /// 预警高热 (75℃ ~ 85℃)
    Warning,
    /// 临界极热 (>= 85℃)
    Critical,
}

/// 工业级热反压守卫
#[derive(Debug, Clone)]
pub struct ThermalGuard {
    zone_path: PathBuf,
    warning_threshold_celsius: f32,
    critical_threshold_celsius: f32,
}

impl Default for ThermalGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl ThermalGuard {
    pub fn new() -> Self {
        Self::with_thresholds(
            PathBuf::from("/sys/class/thermal/thermal_zone0/temp"),
            75.0,
            85.0,
        )
    }

    pub fn with_thresholds(
        zone_path: PathBuf,
        warning_threshold_celsius: f32,
        critical_threshold_celsius: f32,
    ) -> Self {
        Self {
            zone_path,
            warning_threshold_celsius,
            critical_threshold_celsius,
        }
    }

    pub fn with_custom_path(zone_path: impl Into<PathBuf>) -> Self {
        Self::with_thresholds(zone_path.into(), 75.0, 85.0)
    }

    /// 读取当前芯片温度（摄氏度）
    pub fn read_temperature(&self) -> Option<f32> {
        read_soc_temperature(&self.zone_path)
    }

    /// 计算当前热状态级别
    pub fn current_level(&self) -> ThermalLevel {
        match self.read_temperature() {
            Some(temp) if temp >= self.critical_threshold_celsius => ThermalLevel::Critical,
            Some(temp) if temp >= self.warning_threshold_celsius => ThermalLevel::Warning,
            _ => ThermalLevel::Normal,
        }
    }

    /// 获取当前建议的跳帧间隔 (Skip stride)
    ///
    /// - Normal: 0 (不跳帧，每帧均处理)
    /// - Warning: 1 (隔 1 帧处理 1 帧，即 50% 降载)
    /// - Critical: 3 (每 4 帧仅处理 1 帧，即 75% 削峰)
    pub fn recommended_drop_stride(&self) -> u32 {
        match self.current_level() {
            ThermalLevel::Normal => 0,
            ThermalLevel::Warning => 1,
            ThermalLevel::Critical => 3,
        }
    }

    /// 判定指定帧序号是否应当被丢弃以缓解硬件发热
    pub fn should_drop_frame(&self, frame_seq: u64) -> bool {
        let stride = self.recommended_drop_stride();
        if stride == 0 {
            return false;
        }
        // 当 stride = 1 时，frame_seq % 2 != 0 的帧丢弃
        // 当 stride = 3 时，frame_seq % 4 != 0 的帧丢弃
        !frame_seq.is_multiple_of(stride as u64 + 1)
    }
}

/// 读取 Linux sysfs 中的温度毫摄氏度并转换为摄氏度
fn read_soc_temperature(path: &Path) -> Option<f32> {
    if !path.is_file() {
        return None;
    }

    let content = std::fs::read_to_string(path).ok()?;
    let raw_val: f32 = content.trim().parse().ok()?;

    // Linux 内核标准导出单位通常为毫摄氏度 (如 65000 代表 65.0℃)
    if raw_val > 1000.0 {
        Some(raw_val / 1000.0)
    } else {
        Some(raw_val)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_thermal_guard_decision_logic() {
        let temp_dir =
            std::env::temp_dir().join(format!("test_thermal_{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(&temp_dir).expect("创建测试临时目录成功");
        let temp_file = temp_dir.join("temp");

        let guard = ThermalGuard::with_thresholds(temp_file.clone(), 75.0, 85.0);

        // 1. 无温度文件时回退为 Normal
        assert_eq!(guard.current_level(), ThermalLevel::Normal);
        assert_eq!(guard.recommended_drop_stride(), 0);
        assert!(!guard.should_drop_frame(0));
        assert!(!guard.should_drop_frame(1));

        // 2. 模拟正常温度 55.0℃ (55000 毫摄氏度)
        fs::write(&temp_file, b"55000\n").expect("写入测试温度数据成功");
        assert_eq!(guard.read_temperature(), Some(55.0));
        assert_eq!(guard.current_level(), ThermalLevel::Normal);
        assert_eq!(guard.recommended_drop_stride(), 0);

        // 3. 模拟预警温度 78.5℃ (78500)
        fs::write(&temp_file, b"78500\n").expect("写入测试温度数据成功");
        assert_eq!(guard.read_temperature(), Some(78.5));
        assert_eq!(guard.current_level(), ThermalLevel::Warning);
        assert_eq!(guard.recommended_drop_stride(), 1);
        // 验证 50% 丢帧率
        assert!(!guard.should_drop_frame(0)); // 0 % 2 == 0 保留
        assert!(guard.should_drop_frame(1)); // 1 % 2 != 0 丢弃
        assert!(!guard.should_drop_frame(2)); // 2 % 2 == 0 保留
        assert!(guard.should_drop_frame(3)); // 3 % 2 != 0 丢弃

        // 4. 模拟临界危险温度 90.2℃ (90200)
        fs::write(&temp_file, b"90200\n").expect("写入测试温度数据成功");
        assert_eq!(guard.read_temperature(), Some(90.2));
        assert_eq!(guard.current_level(), ThermalLevel::Critical);
        assert_eq!(guard.recommended_drop_stride(), 3);
        // 验证 75% 丢帧率
        assert!(!guard.should_drop_frame(0)); // 0 % 4 == 0 保留
        assert!(guard.should_drop_frame(1)); // 丢弃
        assert!(guard.should_drop_frame(2)); // 丢弃
        assert!(guard.should_drop_frame(3)); // 丢弃
        assert!(!guard.should_drop_frame(4)); // 4 % 4 == 0 保留

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
