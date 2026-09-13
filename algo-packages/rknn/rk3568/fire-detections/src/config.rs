//! 烟火检测算法配置

use std::collections::HashSet;

use algo_sdk::env::PackageEnv;
use serde::Deserialize;

/// 模型支持的类别
pub const FIRE_SMOKE_CLASSES: [&str; 2] = ["fire", "smoke"];

pub const DEFAULT_CONFIDENCE: f32 = 0.25;
pub const DEFAULT_IOU: f32 = 0.45;
pub const DEFAULT_CONFIRM_WINDOW: usize = 5;
pub const DEFAULT_CONFIRM_THRESHOLD: usize = 3;
pub const DEFAULT_TEMPORAL_VARIANCE_THRESHOLD: f32 = 50.0;

fn default_target_classes() -> Vec<String> {
    vec!["fire".to_string(), "smoke".to_string()]
}

#[derive(Deserialize, Default)]
struct RawInstanceConfig {
    #[serde(default)]
    confidence_threshold: Option<f32>,
    #[serde(default)]
    iou_threshold: Option<f32>,
    #[serde(default)]
    target_classes: Option<Vec<String>>,
    #[serde(default)]
    custom_alarm_label: Option<String>,
    #[serde(default)]
    confirm_window: Option<usize>,
    #[serde(default)]
    confirm_threshold: Option<usize>,
    #[serde(default)]
    temporal_variance_threshold: Option<f32>,
}

/// 实例运行时配置
#[derive(Debug, Clone, PartialEq)]
pub struct InstanceConfig {
    pub confidence_threshold: f32,
    pub iou_threshold: f32,
    /// 监控的目标类别（如只监控烟雾可设为 ["smoke"]）
    pub target_classes: Vec<String>,
    /// 自定义告警标签（覆盖模型原始类别名）
    pub custom_alarm_label: Option<String>,
    /// 多帧确认滑动窗口大小
    pub confirm_window: usize,
    /// 窗口内需命中的最小帧数
    pub confirm_threshold: usize,
    /// 时序颜色方差阈值（低于此值的候选框被判定为稳定光源误报）
    pub temporal_variance_threshold: f32,

    /// 记录宿主任务配置显式下发的参数名（用于执行三级优先级隔离）
    pub explicit_fields: HashSet<String>,
}

impl<'de> Deserialize<'de> for InstanceConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawInstanceConfig::deserialize(deserializer)?;
        let mut explicit_fields = HashSet::new();

        macro_rules! track_field {
            ($opt:expr, $name:ident, $default:expr) => {
                match $opt {
                    Some(v) => {
                        explicit_fields.insert(stringify!($name).to_string());
                        v
                    }
                    None => $default,
                }
            };
        }

        let confidence_threshold = track_field!(
            raw.confidence_threshold,
            confidence_threshold,
            DEFAULT_CONFIDENCE
        );
        let iou_threshold = track_field!(raw.iou_threshold, iou_threshold, DEFAULT_IOU);
        let target_classes =
            track_field!(raw.target_classes, target_classes, default_target_classes());
        let custom_alarm_label = raw.custom_alarm_label.inspect(|_| {
            explicit_fields.insert("custom_alarm_label".to_string());
        });
        let confirm_window =
            track_field!(raw.confirm_window, confirm_window, DEFAULT_CONFIRM_WINDOW);
        let confirm_threshold = track_field!(
            raw.confirm_threshold,
            confirm_threshold,
            DEFAULT_CONFIRM_THRESHOLD
        );
        let temporal_variance_threshold = track_field!(
            raw.temporal_variance_threshold,
            temporal_variance_threshold,
            DEFAULT_TEMPORAL_VARIANCE_THRESHOLD
        );

        Ok(Self {
            confidence_threshold,
            iou_threshold,
            target_classes,
            custom_alarm_label,
            confirm_window,
            confirm_threshold,
            temporal_variance_threshold,
            explicit_fields,
        })
    }
}

impl Default for InstanceConfig {
    fn default() -> Self {
        Self {
            confidence_threshold: DEFAULT_CONFIDENCE,
            iou_threshold: DEFAULT_IOU,
            target_classes: default_target_classes(),
            custom_alarm_label: None,
            confirm_window: DEFAULT_CONFIRM_WINDOW,
            confirm_threshold: DEFAULT_CONFIRM_THRESHOLD,
            temporal_variance_threshold: DEFAULT_TEMPORAL_VARIANCE_THRESHOLD,
            explicit_fields: HashSet::new(),
        }
    }
}

impl InstanceConfig {
    /// 注入当前算法包私有 `.env` 的参数覆盖。
    ///
    /// 【三级优先级阶梯原则】：
    /// 1. 宿主显式下发的任务配置最高级：若宿主已传递该字段，严格保护，不被 `.env` 覆盖；
    /// 2. 宿主未传递该字段时：优先使用 `.env` 局部配置；
    /// 3. 若 `.env` 也未设置：维持代码硬编码默认值。
    pub fn apply_env(&mut self, env: &PackageEnv) {
        macro_rules! apply_env_field {
            ($field:ident, $getter:ident) => {
                if !self.explicit_fields.contains(stringify!($field)) {
                    if let Some(v) = env.$getter(stringify!($field)) {
                        self.$field = v.into();
                    }
                }
            };
        }

        apply_env_field!(confidence_threshold, get_f32);
        apply_env_field!(iou_threshold, get_f32);
        apply_env_field!(confirm_window, get_usize);
        apply_env_field!(confirm_threshold, get_usize);
        apply_env_field!(temporal_variance_threshold, get_f32);

        if !self.explicit_fields.contains("custom_alarm_label") {
            if let Some(v) = env.get_str("custom_alarm_label") {
                self.custom_alarm_label = Some(v);
            }
        }
    }
}

/// 轻量 2 类位掩码过滤
#[derive(Debug, Copy, Clone)]
pub struct ClassMask {
    mask: u8,
}

impl ClassMask {
    pub fn from_classes(classes: &[String]) -> Self {
        if classes.is_empty() {
            return Self { mask: 0b11 };
        }
        let mask = classes.iter().fold(0u8, |acc, c| match c.as_str() {
            "fire" => acc | 0b01,
            "smoke" => acc | 0b10,
            _ => acc,
        });
        Self { mask }
    }

    #[inline(always)]
    pub fn is_enabled(&self, class_id: usize) -> bool {
        class_id < 2 && (self.mask & (1 << class_id)) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let cfg = InstanceConfig::default();
        assert_eq!(cfg.confidence_threshold, 0.25);
        assert_eq!(cfg.iou_threshold, 0.45);
        assert_eq!(cfg.target_classes, vec!["fire", "smoke"]);
        assert_eq!(cfg.confirm_window, 5);
        assert_eq!(cfg.confirm_threshold, 3);
        assert_eq!(cfg.temporal_variance_threshold, 50.0);
    }

    #[test]
    fn test_custom_config_deserialization() {
        let json = r#"{"confidence_threshold": 0.5, "confirm_window": 10, "confirm_threshold": 7}"#;
        let cfg: InstanceConfig = serde_json::from_str(json).expect("解析失败");
        assert_eq!(cfg.confidence_threshold, 0.5);
        assert_eq!(cfg.confirm_window, 10);
        assert_eq!(cfg.confirm_threshold, 7);
    }

    #[test]
    fn test_class_mask() {
        let mask = ClassMask::from_classes(&["fire".to_string()]);
        assert!(mask.is_enabled(0)); // fire
        assert!(!mask.is_enabled(1)); // smoke

        let mask = ClassMask::from_classes(&["smoke".to_string()]);
        assert!(!mask.is_enabled(0));
        assert!(mask.is_enabled(1));

        let mask = ClassMask::from_classes(&["fire".to_string(), "smoke".to_string()]);
        assert!(mask.is_enabled(0));
        assert!(mask.is_enabled(1));
    }

    #[test]
    fn test_class_mask_empty_enables_all() {
        let mask = ClassMask::from_classes(&[]);
        assert!(mask.is_enabled(0));
        assert!(mask.is_enabled(1));
    }

    #[test]
    fn test_precedence_host_overrides_env_and_env_overrides_default() {
        let host_json = r#"{"confidence_threshold": 0.75}"#;
        let mut config: InstanceConfig = serde_json::from_str(host_json).expect("解析宿主配置失败");

        let env = PackageEnv::parse_str(
            r#"
            CONFIDENCE_THRESHOLD = 0.15
            CONFIRM_WINDOW = 8
            TEMPORAL_VARIANCE_THRESHOLD = 65.0
            "#,
        );

        config.apply_env(&env);

        // 1. 宿主显式指定的置信度保持 0.75，不被 .env 覆盖
        assert_eq!(config.confidence_threshold, 0.75);
        // 2. 宿主未指定的从 .env 获取
        assert_eq!(config.confirm_window, 8);
        assert_eq!(config.temporal_variance_threshold, 65.0);
        // 3. 宿主和 .env 都未指定的维持代码默认值
        assert_eq!(config.confirm_threshold, 3);
    }
}
