//! 烟火检测算法配置

use algo_sdk::algo_config;
pub use algo_sdk::env::PackageEnv;

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

algo_config! {
    /// 实例运行时配置
    #[derive(Debug, Clone, PartialEq)]
    pub struct InstanceConfig {
        pub confidence_threshold: f32 = DEFAULT_CONFIDENCE,
        pub iou_threshold: f32 = DEFAULT_IOU,
        /// 监控的目标类别（如只监控烟雾可设为 ["smoke"]）
        pub target_classes: Vec<String> = default_target_classes(),
        /// 自定义告警标签（覆盖模型原始类别名）
        pub custom_alarm_label: Option<String> = None,
        /// 多帧确认滑动窗口大小
        pub confirm_window: usize = DEFAULT_CONFIRM_WINDOW,
        /// 窗口内需命中的最小帧数
        pub confirm_threshold: usize = DEFAULT_CONFIRM_THRESHOLD,
        /// 时序颜色方差阈值（低于此值的候选框被判定为稳定光源误报）
        pub temporal_variance_threshold: f32 = DEFAULT_TEMPORAL_VARIANCE_THRESHOLD,
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
