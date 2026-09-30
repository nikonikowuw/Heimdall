//! 安全帽检测算法配置定义
//!
//! 模型检测 2 个类别：`Hardhat`（安全帽）、`NO-Hardhat`（未戴安全帽）。

use algo_sdk::algo_config;
pub use algo_sdk::env::PackageEnv;

/// 模型输出类别
pub const HELMET_CLASSES: [&str; 2] = ["Hardhat", "NO-Hardhat"];

pub const DEFAULT_CONFIDENCE: f32 = 0.45;
pub const DEFAULT_IOU: f32 = 0.45;

algo_config! {
    /// 实例运行时配置
    #[derive(Debug, Clone, PartialEq)]
    pub struct InstanceConfig {
        /// 检测框置信度低于该值的结果将被过滤
        pub confidence_threshold: f32 = DEFAULT_CONFIDENCE,

        /// 非极大值抑制 IoU 阈值
        pub iou_threshold: f32 = DEFAULT_IOU,

        /// 自定义业务告警标签（可选，覆盖模型原生类别名）
        pub custom_alarm_label: Option<String> = None,
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

    #[test]
    fn test_precedence_host_overrides_env_and_env_overrides_default() {
        // 宿主只传递了 confidence_threshold=0.8，未传递 iou_threshold
        let host_json = r#"{"confidence_threshold": 0.8}"#;
        let mut config: InstanceConfig = serde_json::from_str(host_json).expect("解析宿主配置失败");

        let env = PackageEnv::parse_str(
            r#"
            CONFIDENCE_THRESHOLD = 0.2
            IOU_THRESHOLD = 0.6
            CUSTOM_ALARM_LABEL = "调试告警"
            "#,
        );

        config.apply_env(&env);

        // 1. 宿主显式传递的置信度保持 0.8，不被 .env 覆盖
        assert_eq!(config.confidence_threshold, 0.8);
        // 2. 宿主未传递的从 .env 获取
        assert_eq!(config.iou_threshold, 0.6);
        assert_eq!(config.custom_alarm_label.as_deref(), Some("调试告警"));
    }
}
