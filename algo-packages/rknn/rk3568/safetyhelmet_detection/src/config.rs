//! 安全帽检测算法配置定义
//!
//! 模型检测 2 个类别：`Hardhat`（安全帽）、`NO-Hardhat`（未戴安全帽）。

use std::collections::HashSet;

use algo_sdk::env::PackageEnv;
use serde::Deserialize;

/// 模型输出类别
pub const HELMET_CLASSES: [&str; 2] = ["Hardhat", "NO-Hardhat"];

pub const DEFAULT_CONFIDENCE: f32 = 0.45;
pub const DEFAULT_IOU: f32 = 0.45;

#[derive(Deserialize, Default)]
struct RawInstanceConfig {
    #[serde(default)]
    confidence_threshold: Option<f32>,
    #[serde(default)]
    iou_threshold: Option<f32>,
    #[serde(default)]
    custom_alarm_label: Option<String>,
}

/// 实例运行时配置
#[derive(Debug, Clone, PartialEq)]
pub struct InstanceConfig {
    /// 检测框置信度低于该值的结果将被过滤
    pub confidence_threshold: f32,

    /// 非极大值抑制 IoU 阈值
    pub iou_threshold: f32,

    /// 自定义业务告警标签（可选，覆盖模型原生类别名）
    pub custom_alarm_label: Option<String>,

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
        let mut track = |name: &'static str| {
            explicit_fields.insert(name.to_string());
        };

        let confidence_threshold = raw
            .confidence_threshold
            .inspect(|_| track("confidence_threshold"))
            .unwrap_or(DEFAULT_CONFIDENCE);

        let iou_threshold = raw
            .iou_threshold
            .inspect(|_| track("iou_threshold"))
            .unwrap_or(DEFAULT_IOU);

        let custom_alarm_label = raw
            .custom_alarm_label
            .inspect(|_| track("custom_alarm_label"));

        Ok(Self {
            confidence_threshold,
            iou_threshold,
            custom_alarm_label,
            explicit_fields,
        })
    }
}

impl Default for InstanceConfig {
    fn default() -> Self {
        Self {
            confidence_threshold: DEFAULT_CONFIDENCE,
            iou_threshold: DEFAULT_IOU,
            custom_alarm_label: None,
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
        if !self.explicit_fields.contains("confidence_threshold") {
            if let Some(v) = env.get_f32("confidence_threshold") {
                self.confidence_threshold = v;
            }
        }
        if !self.explicit_fields.contains("iou_threshold") {
            if let Some(v) = env.get_f32("iou_threshold") {
                self.iou_threshold = v;
            }
        }
        if !self.explicit_fields.contains("custom_alarm_label") {
            if let Some(v) = env.get_str("custom_alarm_label") {
                self.custom_alarm_label = Some(v);
            }
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
