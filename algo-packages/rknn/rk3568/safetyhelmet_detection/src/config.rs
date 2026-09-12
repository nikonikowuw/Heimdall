//! 安全帽检测算法配置定义
//!
//! 模型检测 2 个类别：`Hardhat`（安全帽）、`NO-Hardhat`（未戴安全帽）。

use serde::Deserialize;

/// 模型输出类别
pub const HELMET_CLASSES: [&str; 2] = ["Hardhat", "NO-Hardhat"];

fn default_confidence() -> f32 {
    0.45
}

fn default_iou() -> f32 {
    0.45
}

/// 实例运行时配置
#[derive(Debug, Clone, Deserialize)]
pub struct InstanceConfig {
    /// 检测框置信度低于该值的结果将被过滤
    #[serde(default = "default_confidence")]
    pub confidence_threshold: f32,

    /// 非极大值抑制 IoU 阈值
    #[serde(default = "default_iou")]
    pub iou_threshold: f32,

    /// 自定义业务告警标签（可选，覆盖模型原生类别名）
    #[serde(default)]
    pub custom_alarm_label: Option<String>,
}

impl Default for InstanceConfig {
    fn default() -> Self {
        Self {
            confidence_threshold: default_confidence(),
            iou_threshold: default_iou(),
            custom_alarm_label: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let cfg = InstanceConfig::default();
        assert_eq!(cfg.confidence_threshold, 0.45);
        assert_eq!(cfg.iou_threshold, 0.45);
        assert!(cfg.custom_alarm_label.is_none());
    }

    #[test]
    fn test_custom_config_deserialization() {
        let json = r#"{"confidence_threshold": 0.6, "iou_threshold": 0.5, "custom_alarm_label": "工地安全检查"}"#;
        let cfg: InstanceConfig = serde_json::from_str(json).expect("解析失败");
        assert_eq!(cfg.confidence_threshold, 0.6);
        assert_eq!(cfg.iou_threshold, 0.5);
        assert_eq!(cfg.custom_alarm_label.as_deref(), Some("工地安全检查"));
    }

    #[test]
    fn test_partial_config_uses_defaults() {
        let json = r#"{"confidence_threshold": 0.3}"#;
        let cfg: InstanceConfig = serde_json::from_str(json).expect("解析失败");
        assert_eq!(cfg.confidence_threshold, 0.3);
        assert_eq!(cfg.iou_threshold, 0.45);
        assert!(cfg.custom_alarm_label.is_none());
    }
}
