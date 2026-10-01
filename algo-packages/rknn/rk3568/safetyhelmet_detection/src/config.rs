//! 安全帽检测算法配置定义
//!
//! 模型检测 2 个类别：`Hardhat`（安全帽）、`NO-Hardhat`（未戴安全帽）。

pub use algo_sdk::env::PackageEnv;
pub use algo_sdk::models::yolo::StandardYoloConfig as InstanceConfig;

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

        assert_eq!(config.confidence_threshold, 0.8);
        assert_eq!(config.iou_threshold, 0.6);
        assert_eq!(config.custom_alarm_label.as_deref(), Some("调试告警"));
    }
}
