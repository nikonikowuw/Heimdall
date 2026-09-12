//! 烟火检测算法配置

use serde::Deserialize;

/// 模型支持的类别
pub const FIRE_SMOKE_CLASSES: [&str; 2] = ["fire", "smoke"];

fn default_confidence() -> f32 {
    0.25
}

fn default_iou() -> f32 {
    0.45
}

fn default_target_classes() -> Vec<String> {
    vec!["fire".to_string(), "smoke".to_string()]
}

/// 多帧确认窗口大小（帧数）
fn default_confirm_window() -> usize {
    5
}

/// 多帧确认阈值（窗口内需命中帧数）
fn default_confirm_threshold() -> usize {
    3
}

/// 时序颜色方差验证阈值（低于此值判定为稳定光源误报）
fn default_temporal_variance_threshold() -> f32 {
    50.0
}

/// 实例运行时配置
#[derive(Debug, Clone, Deserialize)]
pub struct InstanceConfig {
    #[serde(default = "default_confidence")]
    pub confidence_threshold: f32,

    #[serde(default = "default_iou")]
    pub iou_threshold: f32,

    /// 监控的目标类别（如只监控烟雾可设为 ["smoke"]）
    #[serde(default = "default_target_classes")]
    pub target_classes: Vec<String>,

    /// 自定义告警标签（覆盖模型原始类别名）
    #[serde(default)]
    pub custom_alarm_label: Option<String>,

    /// 多帧确认滑动窗口大小
    #[serde(default = "default_confirm_window")]
    pub confirm_window: usize,

    /// 窗口内需命中的最小帧数
    #[serde(default = "default_confirm_threshold")]
    pub confirm_threshold: usize,

    /// 时序颜色方差阈值（低于此值的候选框被判定为稳定光源误报）
    #[serde(default = "default_temporal_variance_threshold")]
    pub temporal_variance_threshold: f32,
}

impl Default for InstanceConfig {
    fn default() -> Self {
        Self {
            confidence_threshold: default_confidence(),
            iou_threshold: default_iou(),
            target_classes: default_target_classes(),
            custom_alarm_label: None,
            confirm_window: default_confirm_window(),
            confirm_threshold: default_confirm_threshold(),
            temporal_variance_threshold: default_temporal_variance_threshold(),
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
        let mut mask = 0u8;
        for c in classes {
            match c.as_str() {
                "fire" => mask |= 0b01,
                "smoke" => mask |= 0b10,
                _ => {}
            }
        }
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
}
