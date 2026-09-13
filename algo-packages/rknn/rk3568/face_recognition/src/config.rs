use std::collections::HashSet;

use algo_sdk::env::PackageEnv;
use serde::Deserialize;

fn default_detection_threshold() -> f32 {
    0.25
}

fn default_min_face_size() -> u32 {
    30
}

fn default_min_quality_score() -> f32 {
    0.3
}

fn default_max_yaw() -> f32 {
    45.0
}

fn default_max_pitch() -> f32 {
    30.0
}

fn default_max_blur() -> f32 {
    0.7
}

#[derive(Deserialize, Default)]
struct RawQualityThresholds {
    #[serde(default)]
    min_score: Option<f32>,
    #[serde(default)]
    max_yaw: Option<f32>,
    #[serde(default)]
    max_pitch: Option<f32>,
    #[serde(default)]
    max_blur: Option<f32>,
}

/// 内部反序列化辅助结构体，同时兼容扁平字段与旧版嵌套对象
#[derive(Deserialize, Default)]
struct RawInstanceConfig {
    #[serde(default)]
    detection_confidence_threshold: Option<f32>,
    #[serde(default)]
    min_face_size: Option<u32>,
    #[serde(default)]
    quality_thresholds: Option<RawQualityThresholds>,
    #[serde(default)]
    quality_min_score: Option<f32>,
    #[serde(default)]
    quality_max_yaw: Option<f32>,
    #[serde(default)]
    quality_max_pitch: Option<f32>,
    #[serde(default)]
    quality_max_blur: Option<f32>,
}

/// 人脸识别算法实例配置。
#[derive(Debug, Clone, PartialEq)]
pub struct InstanceConfig {
    pub detection_confidence_threshold: f32,
    pub min_face_size: u32,
    pub quality_thresholds: QualityThresholds,

    /// 记录宿主任务配置显式下发的参数名（用于执行三级优先级隔离）
    explicit_fields: HashSet<String>,
}

impl<'de> Deserialize<'de> for InstanceConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawInstanceConfig::deserialize(deserializer)?;
        let mut explicit_fields = HashSet::new();

        let detection_confidence_threshold = if let Some(v) = raw.detection_confidence_threshold {
            explicit_fields.insert("detection_confidence_threshold".to_string());
            v
        } else {
            default_detection_threshold()
        };

        let min_face_size = if let Some(v) = raw.min_face_size {
            explicit_fields.insert("min_face_size".to_string());
            v
        } else {
            default_min_face_size()
        };

        let mut thresholds = QualityThresholds::default();
        if let Some(nested) = raw.quality_thresholds {
            if let Some(v) = nested.min_score {
                explicit_fields.insert("quality_min_score".to_string());
                thresholds.min_score = v;
            }
            if let Some(v) = nested.max_yaw {
                explicit_fields.insert("quality_max_yaw".to_string());
                thresholds.max_yaw = v;
            }
            if let Some(v) = nested.max_pitch {
                explicit_fields.insert("quality_max_pitch".to_string());
                thresholds.max_pitch = v;
            }
            if let Some(v) = nested.max_blur {
                explicit_fields.insert("quality_max_blur".to_string());
                thresholds.max_blur = v;
            }
        }
        if let Some(v) = raw.quality_min_score {
            explicit_fields.insert("quality_min_score".to_string());
            thresholds.min_score = v;
        }
        if let Some(v) = raw.quality_max_yaw {
            explicit_fields.insert("quality_max_yaw".to_string());
            thresholds.max_yaw = v;
        }
        if let Some(v) = raw.quality_max_pitch {
            explicit_fields.insert("quality_max_pitch".to_string());
            thresholds.max_pitch = v;
        }
        if let Some(v) = raw.quality_max_blur {
            explicit_fields.insert("quality_max_blur".to_string());
            thresholds.max_blur = v;
        }

        Ok(Self {
            detection_confidence_threshold,
            min_face_size,
            quality_thresholds: thresholds,
            explicit_fields,
        })
    }
}

impl Default for InstanceConfig {
    fn default() -> Self {
        Self {
            detection_confidence_threshold: default_detection_threshold(),
            min_face_size: default_min_face_size(),
            quality_thresholds: QualityThresholds::default(),
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
        if !self
            .explicit_fields
            .contains("detection_confidence_threshold")
        {
            if let Some(v) = env.get_f32("detection_confidence_threshold") {
                self.detection_confidence_threshold = v;
            }
        }
        if !self.explicit_fields.contains("min_face_size") {
            if let Some(v) = env.get_u32("min_face_size") {
                self.min_face_size = v;
            }
        }
        if !self.explicit_fields.contains("quality_min_score") {
            if let Some(v) = env.get_f32("quality_min_score") {
                self.quality_thresholds.min_score = v;
            }
        }
        if !self.explicit_fields.contains("quality_max_yaw") {
            if let Some(v) = env.get_f32("quality_max_yaw") {
                self.quality_thresholds.max_yaw = v;
            }
        }
        if !self.explicit_fields.contains("quality_max_pitch") {
            if let Some(v) = env.get_f32("quality_max_pitch") {
                self.quality_thresholds.max_pitch = v;
            }
        }
        if !self.explicit_fields.contains("quality_max_blur") {
            if let Some(v) = env.get_f32("quality_max_blur") {
                self.quality_thresholds.max_blur = v;
            }
        }
    }

    /// 校验来自 ABI 配置 JSON 的数值范围。
    pub fn validate(&self) -> Result<(), String> {
        if !self.detection_confidence_threshold.is_finite()
            || !(0.0..=1.0).contains(&self.detection_confidence_threshold)
        {
            return Err("detection_confidence_threshold 必须位于 [0, 1]".to_string());
        }
        if self.min_face_size == 0 {
            return Err("min_face_size 必须大于 0".to_string());
        }
        self.quality_thresholds.validate()
    }
}

/// 质量门控阈值。
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct QualityThresholds {
    #[serde(default = "default_min_quality_score")]
    pub min_score: f32,
    #[serde(default = "default_max_yaw")]
    pub max_yaw: f32,
    #[serde(default = "default_max_pitch")]
    pub max_pitch: f32,
    #[serde(default = "default_max_blur")]
    pub max_blur: f32,
}

impl Default for QualityThresholds {
    fn default() -> Self {
        Self {
            min_score: default_min_quality_score(),
            max_yaw: default_max_yaw(),
            max_pitch: default_max_pitch(),
            max_blur: default_max_blur(),
        }
    }
}

impl QualityThresholds {
    pub fn validate(&self) -> Result<(), String> {
        if !self.min_score.is_finite() || !(0.0..=1.0).contains(&self.min_score) {
            return Err("quality_thresholds.min_score 必须位于 [0, 1]".to_string());
        }
        if !self.max_yaw.is_finite() || !(0.0..=180.0).contains(&self.max_yaw) {
            return Err("quality_thresholds.max_yaw 必须位于 [0, 180]".to_string());
        }
        if !self.max_pitch.is_finite() || !(0.0..=90.0).contains(&self.max_pitch) {
            return Err("quality_thresholds.max_pitch 必须位于 [0, 90]".to_string());
        }
        if !self.max_blur.is_finite() || !(0.0..=1.0).contains(&self.max_blur) {
            return Err("quality_thresholds.max_blur 必须位于 [0, 1]".to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_expected_values() {
        let config = InstanceConfig::default();
        assert_eq!(config.detection_confidence_threshold, 0.25);
        assert_eq!(config.min_face_size, 30);
        assert_eq!(config.quality_thresholds.min_score, 0.3);
        assert_eq!(config.quality_thresholds.max_yaw, 45.0);
        assert_eq!(config.quality_thresholds.max_pitch, 30.0);
        assert_eq!(config.quality_thresholds.max_blur, 0.7);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn deserializes_flat_schema_properties() {
        let json = r#"{
            "detection_confidence_threshold": 0.4,
            "min_face_size": 48,
            "quality_min_score": 0.55,
            "quality_max_yaw": 30.0,
            "quality_max_pitch": 20.0,
            "quality_max_blur": 0.6
        }"#;

        let config: InstanceConfig = serde_json::from_str(json).expect("解析扁平配置应当成功");
        assert_eq!(config.detection_confidence_threshold, 0.4);
        assert_eq!(config.min_face_size, 48);
        assert_eq!(config.quality_thresholds.min_score, 0.55);
        assert_eq!(config.quality_thresholds.max_yaw, 30.0);
        assert_eq!(config.quality_thresholds.max_pitch, 20.0);
        assert_eq!(config.quality_thresholds.max_blur, 0.6);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn deserializes_legacy_nested_quality_thresholds() {
        let json = r#"{
            "detection_confidence_threshold": 0.35,
            "min_face_size": 40,
            "quality_thresholds": {
                "min_score": 0.45,
                "max_yaw": 25.0,
                "max_pitch": 15.0,
                "max_blur": 0.5
            }
        }"#;

        let config: InstanceConfig = serde_json::from_str(json).expect("解析嵌套配置应当成功");
        assert_eq!(config.detection_confidence_threshold, 0.35);
        assert_eq!(config.min_face_size, 40);
        assert_eq!(config.quality_thresholds.min_score, 0.45);
        assert_eq!(config.quality_thresholds.max_yaw, 25.0);
        assert_eq!(config.quality_thresholds.max_pitch, 15.0);
        assert_eq!(config.quality_thresholds.max_blur, 0.5);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_precedence_host_overrides_env_and_env_overrides_default() {
        // 宿主只传递了 detection_confidence_threshold=0.85，未传递 min_face_size 和 quality_min_score
        let host_json = r#"{"detection_confidence_threshold": 0.85}"#;
        let mut config: InstanceConfig =
            serde_json::from_str(host_json).expect("解析宿主配置应当成功");

        // 算法包私有 .env
        let env = PackageEnv::parse_str(
            r#"
            # .env 中尝试设置两个参数
            DETECTION_CONFIDENCE_THRESHOLD = 0.10
            MIN_FACE_SIZE = 50
            QUALITY_MIN_SCORE = 0.60
            "#,
        );

        config.apply_env(&env);

        // 1. 宿主显式传递的必须维持 0.85（第一优先级最高，不可被 .env 覆盖）
        assert_eq!(config.detection_confidence_threshold, 0.85);
        // 2. 宿主未传递的 min_face_size 和 quality_min_score 取 .env 中的值（第二优先级）
        assert_eq!(config.min_face_size, 50);
        assert_eq!(config.quality_thresholds.min_score, 0.60);
        // 3. 宿主和 .env 都未传递的 quality_max_yaw 取代码硬编码默认值（第三优先级保底）
        assert_eq!(config.quality_thresholds.max_yaw, 45.0);
    }
}
