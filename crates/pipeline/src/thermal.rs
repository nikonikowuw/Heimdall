//! 工业级边缘温控与热安全闭环引擎 (Thermal Safety Engine & Dynamic Load Shedder)
//!
//! 专为嵌入式边缘异构平台（Rockchip RK3588、华为昇腾 Ascend 310B / Atlas 200I/500 等无风扇被动散热设备）设计：
//! 1. 多 Thermal Zone 实时探测与热点（Hotspot）峰值感知（CPU、GPU、NPU、DDR、PMIC）；
//! 2. 硬件平台与机箱散热档案（SoC Thermal Policy Profiles）；
//! 3. 采样防抖（Debounce）与迟滞回退（Hysteresis），杜绝阈值附近剧烈振荡；
//! 4. 传感器失效保守安全模式（Sensor Failure Fallback），不盲目假设 Normal；
//! 5. 全闭环控制矩阵（ThermalActionPlan）：动态跳帧降载、任务准入阻断、非关键流削峰与系统级告警。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::{info, warn};

/// 硬件热状态级别
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThermalLevel {
    /// 正常温度（设备在安全温度区间内稳定运行）
    Normal,
    /// 预警高热（开始平滑阶梯跳帧降载，抑制结温进一步攀升）
    Warning,
    /// 临界极热（高强度削峰降载，暂停新任务准入，防止热雪崩）
    Critical,
    /// 紧急切断（断开非关键辅码流，停止常规推理，防止硬件触发内核级不可逆紧急热关机）
    Emergency,
    /// 传感器未知/读取失效（进入保守安全模式，执行 50% 保护性限流并报警）
    Conservative,
}

/// 边缘异构平台温控策略配置
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThermalPolicyConfig {
    /// 预警温度阈值（摄氏度）
    pub warning_temp: f32,
    /// 临界高热阈值（摄氏度）
    pub critical_temp: f32,
    /// 紧急切断阈值（摄氏度）
    pub emergency_temp: f32,
    /// 迟滞回退温差（摄氏度，例如 4.0℃，降温必须低于 threshold - hysteresis 才能降级）
    pub hysteresis_delta: f32,
    /// 升温判定防抖连续采样次数（防止偶发瞬态温度尖峰毛刺误触状态跃迁）
    pub debounce_samples: usize,
    /// 降温恢复观察连续采样次数（平滑冷却观察期，杜绝冷热频繁往复振荡）
    pub recovery_cooldown_samples: usize,
    /// 传感器失效时是否执行保守安全限流模式
    pub conservative_on_sensor_failure: bool,
    /// 采样周期建议
    pub sample_interval: Duration,
}

impl Default for ThermalPolicyConfig {
    fn default() -> Self {
        Self::conservative_default()
    }
}

impl ThermalPolicyConfig {
    /// 保守安全默认配置（适用于未明确硬件型号的通用工控环境）
    pub fn conservative_default() -> Self {
        Self {
            warning_temp: 75.0,
            critical_temp: 85.0,
            emergency_temp: 92.0,
            hysteresis_delta: 4.0,
            debounce_samples: 2,
            recovery_cooldown_samples: 3,
            conservative_on_sensor_failure: true,
            sample_interval: Duration::from_secs(2),
        }
    }

    /// 瑞芯微 RK3588 无风扇工控平台散热档案
    ///
    /// RK3588 工业核心通常结温上限为 105℃，在被动散热金属外壳设计下：
    /// - 80℃ 启动轻度降载；
    /// - 90℃ 启动强力削峰并阻断新任务；
    /// - 96℃ 触发紧急避险；迟滞温差 5℃。
    pub fn rk3588_fanless() -> Self {
        Self {
            warning_temp: 80.0,
            critical_temp: 90.0,
            emergency_temp: 96.0,
            hysteresis_delta: 5.0,
            debounce_samples: 2,
            recovery_cooldown_samples: 4,
            conservative_on_sensor_failure: true,
            sample_interval: Duration::from_secs(2),
        }
    }

    /// 华为昇腾 Ascend 310B / Atlas 200I A2 边缘平台散热档案
    ///
    /// 昇腾 DaVinci NPU 核心对持续高温敏感：
    /// - 75℃ 启动预警降载；
    /// - 83℃ 启动临界保护；
    /// - 88℃ 触发紧急避险；迟滞温差 4℃。
    pub fn ascend_310b_edge() -> Self {
        Self {
            warning_temp: 75.0,
            critical_temp: 83.0,
            emergency_temp: 88.0,
            hysteresis_delta: 4.0,
            debounce_samples: 2,
            recovery_cooldown_samples: 3,
            conservative_on_sensor_failure: true,
            sample_interval: Duration::from_secs(2),
        }
    }

    /// 华为昇腾 Atlas 500 工业边缘小站散热档案
    pub fn atlas_500_industrial() -> Self {
        Self {
            warning_temp: 78.0,
            critical_temp: 86.0,
            emergency_temp: 92.0,
            hysteresis_delta: 4.0,
            debounce_samples: 2,
            recovery_cooldown_samples: 3,
            conservative_on_sensor_failure: true,
            sample_interval: Duration::from_secs(2),
        }
    }
}

/// 单个热区（Thermal Zone）读数快照
#[derive(Debug, Clone, PartialEq)]
pub struct ThermalZoneInfo {
    pub id: usize,
    pub name: String,
    pub temp_celsius: f32,
    pub path: PathBuf,
}

/// 完整的热安全闭环行动方案
#[derive(Debug, Clone, PartialEq)]
pub struct ThermalActionPlan {
    /// 当前判定的热保护等级
    pub level: ThermalLevel,
    /// 峰值核心结温（摄氏度，None 表示传感器全部失效）
    pub peak_temperature: Option<f32>,
    /// 各热区分项采样信息
    pub zone_readings: Vec<ThermalZoneInfo>,
    /// 建议的子码流推理跳帧步长 (0=全帧率, 1=50%降载, 3=75%削峰, u32::MAX=完全停止推理)
    pub drop_stride: u32,
    /// 准入控制：是否阻断新创建分析流水线任务
    pub pause_new_tasks: bool,
    /// 降载决策：是否卸载/释放非关键监控流
    pub shed_non_critical_streams: bool,
    /// 是否需要上报系统级温控或传感器告警
    pub trigger_alarm: bool,
    /// 告警或状态说明
    pub alarm_message: Option<String>,
}

/// 工业级热安全守卫与动态降载引擎
#[derive(Debug, Clone)]
pub struct ThermalGuard {
    /// 监控的 thermal zone 路径列表
    zone_paths: Vec<PathBuf>,
    /// 热控制策略
    policy: ThermalPolicyConfig,

    // === 内部状态机（防抖与迟滞跟踪） ===
    current_level: ThermalLevel,
    last_peak_temp: Option<f32>,
    /// 针对当前潜在更高等级的连续采样计数（用于防抖）
    consecutive_higher_level_count: usize,
    /// 针对当前潜在更低等级的连续冷却采样计数（用于迟滞恢复）
    consecutive_lower_level_count: usize,
    /// 连续采样失败计数
    consecutive_failure_count: usize,
}

impl Default for ThermalGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl ThermalGuard {
    /// 创建默认守卫（自动探测系统可用 thermal zones，使用保守策略）
    pub fn new() -> Self {
        let zones = detect_system_thermal_zones();
        let fallback_zones = if zones.is_empty() {
            vec![PathBuf::from("/sys/class/thermal/thermal_zone0/temp")]
        } else {
            zones
        };
        Self::with_custom_zones(fallback_zones, ThermalPolicyConfig::default())
    }

    /// 针对指定平台配置创建守卫
    pub fn with_policy(policy: ThermalPolicyConfig) -> Self {
        let zones = detect_system_thermal_zones();
        let fallback_zones = if zones.is_empty() {
            vec![PathBuf::from("/sys/class/thermal/thermal_zone0/temp")]
        } else {
            zones
        };
        Self::with_custom_zones(fallback_zones, policy)
    }

    /// 指定单个热区路径与阈值（保持原有向后兼容 API）
    pub fn with_thresholds(
        zone_path: PathBuf,
        warning_threshold_celsius: f32,
        critical_threshold_celsius: f32,
    ) -> Self {
        let mut policy = ThermalPolicyConfig::conservative_default();
        policy.warning_temp = warning_threshold_celsius;
        policy.critical_temp = critical_threshold_celsius;
        policy.emergency_temp = critical_threshold_celsius + 7.0;
        Self::with_custom_zones(vec![zone_path], policy)
    }

    /// 指定单个路径与默认策略（保持原有向后兼容 API）
    pub fn with_custom_path(zone_path: impl Into<PathBuf>) -> Self {
        Self::with_custom_zones(vec![zone_path.into()], ThermalPolicyConfig::default())
    }

    /// 指定多个 thermal zone 路径与完整策略配置
    pub fn with_custom_zones(zone_paths: Vec<PathBuf>, policy: ThermalPolicyConfig) -> Self {
        Self {
            zone_paths,
            policy,
            current_level: ThermalLevel::Normal,
            last_peak_temp: None,
            consecutive_higher_level_count: 0,
            consecutive_lower_level_count: 0,
            consecutive_failure_count: 0,
        }
    }

    /// 快捷预设：Rockchip RK3588
    pub fn for_rk3588() -> Self {
        Self::with_policy(ThermalPolicyConfig::rk3588_fanless())
    }

    /// 快捷预设：Huawei Ascend 310B
    pub fn for_ascend_310b() -> Self {
        Self::with_policy(ThermalPolicyConfig::ascend_310b_edge())
    }

    /// 获取受监控的热区路径列表
    pub fn zone_paths(&self) -> &[PathBuf] {
        &self.zone_paths
    }

    /// 获取当前生效的温控策略
    pub fn policy(&self) -> &ThermalPolicyConfig {
        &self.policy
    }

    /// 读取所有配置热区的结温并提取最高热点温度（摄氏度）
    pub fn read_temperature(&self) -> Option<f32> {
        let readings = self.sample_zones();
        readings
            .into_iter()
            .map(|z| z.temp_celsius)
            .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
    }

    /// 采样所有热区
    pub fn sample_zones(&self) -> Vec<ThermalZoneInfo> {
        let mut results = Vec::new();
        for (idx, path) in self.zone_paths.iter().enumerate() {
            if let Some(temp) = read_soc_temperature(path) {
                let name = read_zone_type(path).unwrap_or_else(|| format!("zone_{idx}"));
                results.push(ThermalZoneInfo {
                    id: idx,
                    name,
                    temp_celsius: temp,
                    path: path.clone(),
                });
            }
        }
        results
    }

    /// 获取当前生效的热状态级别（只读查询）
    pub fn current_level(&self) -> ThermalLevel {
        self.current_level
    }

    /// 获取当前建议的跳帧间隔 (Skip stride)
    ///
    /// - Normal: 0 (不跳帧，每帧均推理)
    /// - Warning: 1 (隔 1 帧推理 1 帧，即 50% 降载)
    /// - Critical: 3 (每 4 帧仅推理 1 帧，即 75% 削峰)
    /// - Emergency: u32::MAX (全部跳过)
    /// - Conservative: 1 (传感器故障时的 50% 保护性降载)
    pub fn recommended_drop_stride(&self) -> u32 {
        match self.current_level {
            ThermalLevel::Normal => 0,
            ThermalLevel::Warning => 1,
            ThermalLevel::Critical => 3,
            ThermalLevel::Emergency => u32::MAX,
            ThermalLevel::Conservative => 1,
        }
    }

    /// 判定指定帧序号是否应当被丢弃以缓解硬件发热
    pub fn should_drop_frame(&self, frame_seq: u64) -> bool {
        let stride = self.recommended_drop_stride();
        if stride == 0 {
            return false;
        }
        if stride == u32::MAX {
            return true;
        }
        !frame_seq.is_multiple_of(stride as u64 + 1)
    }

    /// 执行一次完整的热状态采样、防抖计算与闭环行动方案推导
    pub fn evaluate(&mut self) -> ThermalActionPlan {
        let readings = self.sample_zones();
        let peak_temp = readings
            .iter()
            .map(|z| z.temp_celsius)
            .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        // 1. 传感器故障 / 无读数状态处理
        if peak_temp.is_none() {
            self.consecutive_failure_count += 1;
            if self.policy.conservative_on_sensor_failure && self.consecutive_failure_count >= 2 {
                if self.current_level != ThermalLevel::Conservative {
                    warn!(
                        consecutive_failures = self.consecutive_failure_count,
                        "ThermalGuard: 温度传感器读取失效，进入 Conservative 保守安全限流模式"
                    );
                    self.current_level = ThermalLevel::Conservative;
                }
                return ThermalActionPlan {
                    level: ThermalLevel::Conservative,
                    peak_temperature: None,
                    zone_readings: readings,
                    drop_stride: 1,
                    pause_new_tasks: true,
                    shed_non_critical_streams: false,
                    trigger_alarm: true,
                    alarm_message: Some(
                        "设备温度传感器连续读取失败，系统进入保守安全限流模式 (50% 降载)"
                            .to_string(),
                    ),
                };
            }
        } else {
            self.consecutive_failure_count = 0;
        }

        let temp = match peak_temp {
            Some(t) => t,
            None => {
                // 单次抖动未超阈值时保持原状态
                return self.build_action_plan(peak_temp, readings);
            }
        };

        self.last_peak_temp = Some(temp);

        // 2. 基于迟滞与防抖的状态机跃迁
        let raw_target_level = self.calculate_raw_level(temp);

        if raw_target_level > self.current_level {
            // 升温方向：需要连续达到 debounce_samples 才真正升温（防止瞬态尖峰毛刺）
            self.consecutive_higher_level_count += 1;
            self.consecutive_lower_level_count = 0;

            if self.consecutive_higher_level_count >= self.policy.debounce_samples {
                info!(
                    previous_level = ?self.current_level,
                    new_level = ?raw_target_level,
                    peak_temp = temp,
                    "ThermalGuard: 触发热状态升温降载跃迁"
                );
                self.current_level = raw_target_level;
                self.consecutive_higher_level_count = 0;
            }
        } else if raw_target_level < self.current_level {
            // 降温方向：必须满足迟滞温差 (Hysteresis) 且连续平稳降温达到 recovery_cooldown_samples
            if self.can_downgrade_with_hysteresis(temp) {
                self.consecutive_lower_level_count += 1;
                self.consecutive_higher_level_count = 0;

                if self.consecutive_lower_level_count >= self.policy.recovery_cooldown_samples {
                    let next_lower = self.step_downgrade_level();
                    info!(
                        previous_level = ?self.current_level,
                        new_level = ?next_lower,
                        peak_temp = temp,
                        "ThermalGuard: 结温已充分冷却，渐进平稳降级恢复负载"
                    );
                    self.current_level = next_lower;
                    self.consecutive_lower_level_count = 0;
                }
            } else {
                // 处于迟滞带（Deadband / Hysteresis Band）中，维持原等级不变，避免振荡
                self.consecutive_lower_level_count = 0;
            }
        } else {
            // 状态稳定
            self.consecutive_higher_level_count = 0;
            self.consecutive_lower_level_count = 0;
        }

        self.build_action_plan(peak_temp, readings)
    }

    /// 计算不考虑迟滞状态时的裸目标等级
    fn calculate_raw_level(&self, temp: f32) -> ThermalLevel {
        if temp >= self.policy.emergency_temp {
            ThermalLevel::Emergency
        } else if temp >= self.policy.critical_temp {
            ThermalLevel::Critical
        } else if temp >= self.policy.warning_temp {
            ThermalLevel::Warning
        } else {
            ThermalLevel::Normal
        }
    }

    /// 校验当前温度是否已跌破当前等级减去迟滞温差的门限
    fn can_downgrade_with_hysteresis(&self, temp: f32) -> bool {
        let delta = self.policy.hysteresis_delta;
        match self.current_level {
            ThermalLevel::Emergency => temp < (self.policy.emergency_temp - delta),
            ThermalLevel::Critical => temp < (self.policy.critical_temp - delta),
            ThermalLevel::Warning => temp < (self.policy.warning_temp - delta),
            ThermalLevel::Normal => false,
            ThermalLevel::Conservative => temp < (self.policy.warning_temp - delta),
        }
    }

    /// 单步渐进式降级（避免从 Emergency 瞬间恢复到 Normal）
    fn step_downgrade_level(&self) -> ThermalLevel {
        match self.current_level {
            ThermalLevel::Emergency => ThermalLevel::Critical,
            ThermalLevel::Critical => ThermalLevel::Warning,
            ThermalLevel::Warning => ThermalLevel::Normal,
            ThermalLevel::Conservative => ThermalLevel::Normal,
            ThermalLevel::Normal => ThermalLevel::Normal,
        }
    }

    /// 生成完整的行动决策闭环方案
    fn build_action_plan(
        &self,
        peak_temp: Option<f32>,
        readings: Vec<ThermalZoneInfo>,
    ) -> ThermalActionPlan {
        let drop_stride = self.recommended_drop_stride();
        let (pause_new_tasks, shed_non_critical, trigger_alarm, alarm_message) =
            match self.current_level {
                ThermalLevel::Normal => (false, false, false, None),
                ThermalLevel::Warning => (
                    false,
                    false,
                    true,
                    Some(format!(
                        "硬件核心温度偏高 ({:.1}℃)，触发 50% 阶梯跳帧动态降载",
                        peak_temp.unwrap_or(0.0)
                    )),
                ),
                ThermalLevel::Critical => (
                    true,
                    false,
                    true,
                    Some(format!(
                        "硬件核心温度进入临界状态 ({:.1}℃)，执行 75% 削峰降载并暂停新任务准入",
                        peak_temp.unwrap_or(0.0)
                    )),
                ),
                ThermalLevel::Emergency => (
                    true,
                    true,
                    true,
                    Some(format!(
                        "硬件核心温度突破紧急阈值 ({:.1}℃)，紧急停止非关键推理以防硬件物理损坏",
                        peak_temp.unwrap_or(0.0)
                    )),
                ),
                ThermalLevel::Conservative => (
                    true,
                    false,
                    true,
                    Some("温度传感器读取不可用，系统运行于保守 50% 降载模式".to_string()),
                ),
            };

        ThermalActionPlan {
            level: self.current_level,
            peak_temperature: peak_temp,
            zone_readings: readings,
            drop_stride,
            pause_new_tasks,
            shed_non_critical_streams: shed_non_critical,
            trigger_alarm,
            alarm_message,
        }
    }
}

/// 自动探测 Linux 系统中所有可用的 thermal zones
fn detect_system_thermal_zones() -> Vec<PathBuf> {
    let thermal_base = Path::new("/sys/class/thermal");
    if !thermal_base.is_dir() {
        return Vec::new();
    }

    let mut zones = Vec::new();
    if let Ok(entries) = std::fs::read_dir(thermal_base) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(file_name) = path.file_name().and_then(|s| s.to_str()) {
                if file_name.starts_with("thermal_zone") {
                    let temp_path = path.join("temp");
                    if temp_path.is_file() {
                        zones.push(temp_path);
                    }
                }
            }
        }
    }
    // 排序确保稳定次序
    zones.sort();
    zones
}

/// 读取 thermal zone 的类型名称
fn read_zone_type(temp_path: &Path) -> Option<String> {
    let parent = temp_path.parent()?;
    let type_path = parent.join("type");
    if type_path.is_file() {
        std::fs::read_to_string(type_path)
            .ok()
            .map(|s| s.trim().to_string())
    } else {
        None
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

impl PartialOrd for ThermalLevel {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ThermalLevel {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let weight = |l: &ThermalLevel| match l {
            ThermalLevel::Normal => 0,
            ThermalLevel::Conservative => 1,
            ThermalLevel::Warning => 2,
            ThermalLevel::Critical => 3,
            ThermalLevel::Emergency => 4,
        };
        weight(self).cmp(&weight(other))
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

        let mut guard = ThermalGuard::with_thresholds(temp_file.clone(), 75.0, 85.0);

        // 1. 无温度文件时：连续失败进入 Conservative 安全模式，而不是盲目 Normal
        let plan0 = guard.evaluate();
        assert_eq!(plan0.level, ThermalLevel::Normal); // 首次失败
        let plan1 = guard.evaluate(); // 第二次连续失败触发 Conservative 保守安全
        assert_eq!(plan1.level, ThermalLevel::Conservative);
        assert_eq!(plan1.drop_stride, 1);
        assert!(plan1.pause_new_tasks);
        assert!(plan1.trigger_alarm);

        // 2. 模拟正常温度 55.0℃ (55000 毫摄氏度)
        fs::write(&temp_file, b"55000\n").expect("写入测试温度数据成功");
        assert_eq!(guard.read_temperature(), Some(55.0));
        // 传感器恢复后，经历 recovery_cooldown_samples 次稳定确认回到 Normal
        guard.evaluate();
        guard.evaluate();
        let plan_normal = guard.evaluate();
        assert_eq!(plan_normal.level, ThermalLevel::Normal);
        assert_eq!(plan_normal.drop_stride, 0);
        assert!(!plan_normal.pause_new_tasks);
        assert!(!plan_normal.shed_non_critical_streams);

        // 3. 模拟单次偶发尖峰 80.0℃ (防抖测试：单次不应立即跳级)
        fs::write(&temp_file, b"80000\n").expect("写入");
        let plan_spike = guard.evaluate();
        assert_eq!(
            plan_spike.level,
            ThermalLevel::Normal,
            "单次温度毛刺应被防抖过滤"
        );

        // 再次采样 80.0℃：满足 debounce_samples = 2，正式升级为 Warning
        let plan_warn = guard.evaluate();
        assert_eq!(plan_warn.level, ThermalLevel::Warning);
        assert_eq!(plan_warn.drop_stride, 1);
        assert!(plan_warn.trigger_alarm);
        // 验证 50% 丢帧率
        assert!(!guard.should_drop_frame(0)); // 0 % 2 == 0 保留
        assert!(guard.should_drop_frame(1)); // 1 % 2 != 0 丢弃
        assert!(!guard.should_drop_frame(2)); // 2 % 2 == 0 保留
        assert!(guard.should_drop_frame(3)); // 3 % 2 != 0 丢弃

        // 4. 模拟临界危险温度 90.2℃ (90200)
        fs::write(&temp_file, b"90200\n").expect("写入");
        guard.evaluate(); // 第 1 次防抖
        let plan_crit = guard.evaluate(); // 第 2 次确认
        assert_eq!(plan_crit.level, ThermalLevel::Critical);
        assert_eq!(plan_crit.drop_stride, 3);
        assert!(plan_crit.pause_new_tasks, "临界状态必须暂停新任务");
        // 验证 75% 丢帧率
        assert!(!guard.should_drop_frame(0));
        assert!(guard.should_drop_frame(1));
        assert!(guard.should_drop_frame(2));
        assert!(guard.should_drop_frame(3));
        assert!(!guard.should_drop_frame(4));

        // 5. 迟滞回退测试 (Hysteresis)
        // 策略中 critical_temp = 85.0, hysteresis_delta = 4.0 -> 回退点必须 < 81.0℃
        // 降到 83.0℃ 时仍在迟滞死区内，不得降级！
        fs::write(&temp_file, b"83000\n").expect("写入");
        let plan_deadband = guard.evaluate();
        assert_eq!(
            plan_deadband.level,
            ThermalLevel::Critical,
            "处于迟滞带内应保持原状态"
        );

        // 跌破迟滞门限 79.0℃ (< 81.0℃)，连续平稳采样 3 次渐进降为 Warning
        fs::write(&temp_file, b"79000\n").expect("写入");
        guard.evaluate();
        guard.evaluate();
        let plan_downgrade = guard.evaluate();
        assert_eq!(plan_downgrade.level, ThermalLevel::Warning);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_multi_zone_hotspot_peak_detection() {
        let temp_dir =
            std::env::temp_dir().join(format!("test_multizone_{}", uuid::Uuid::new_v4().simple()));
        fs::create_dir_all(&temp_dir).expect("创建测试临时目录成功");

        let z0_dir = temp_dir.join("thermal_zone0");
        let z1_dir = temp_dir.join("thermal_zone1");
        fs::create_dir_all(&z0_dir).expect("z0");
        fs::create_dir_all(&z1_dir).expect("z1");

        fs::write(z0_dir.join("type"), b"cpu-thermal\n").expect("type");
        fs::write(z0_dir.join("temp"), b"60000\n").expect("temp"); // 60℃

        fs::write(z1_dir.join("type"), b"npu-thermal\n").expect("type");
        fs::write(z1_dir.join("temp"), b"82000\n").expect("temp"); // 82℃ (热点)

        let mut guard = ThermalGuard::with_custom_zones(
            vec![z0_dir.join("temp"), z1_dir.join("temp")],
            ThermalPolicyConfig::ascend_310b_edge(),
        );

        // 读取峰值热点
        assert_eq!(guard.read_temperature(), Some(82.0));

        let plan = guard.evaluate();
        assert_eq!(plan.zone_readings.len(), 2);
        assert_eq!(plan.peak_temperature, Some(82.0));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_platform_profiles() {
        let rk = ThermalPolicyConfig::rk3588_fanless();
        assert_eq!(rk.warning_temp, 80.0);
        assert_eq!(rk.critical_temp, 90.0);

        let ascend = ThermalPolicyConfig::ascend_310b_edge();
        assert_eq!(ascend.warning_temp, 75.0);
        assert_eq!(ascend.critical_temp, 83.0);
    }
}
