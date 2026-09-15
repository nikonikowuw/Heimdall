use algo_sdk::env::PackageEnv;
use serde::Deserialize;

const FIELD_DETECTION_CONF: u8 = 1 << 0;
const FIELD_PERSON_CONF: u8 = 1 << 1;
const FIELD_MIN_FACE_SIZE: u8 = 1 << 2;
const FIELD_QUALITY_MIN_SCORE: u8 = 1 << 3;
const FIELD_QUALITY_MAX_YAW: u8 = 1 << 4;
const FIELD_QUALITY_MAX_PITCH: u8 = 1 << 5;
const FIELD_QUALITY_MAX_BLUR: u8 = 1 << 6;

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
    person_confidence_threshold: Option<f32>,
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
    pub person_confidence_threshold: f32,
    pub min_face_size: u32,
    pub quality_thresholds: QualityThresholds,

    /// 记录宿主任务配置显式下发的参数位掩码（用于执行三级优先级隔离）
    explicit_fields: u8,
}

impl Default for InstanceConfig {
    fn default() -> Self {
        Self {
            // 与 config.schema.json 默认值、.env.example 及 RK3576 姊妹包保持一致：
            // 缺省回退必须等同于控制台未改动表单时的下发值，否则“缺省路径”与“表单路径”行为分叉。
            detection_confidence_threshold: 0.25,
            person_confidence_threshold: 0.4,
            min_face_size: 30,
            quality_thresholds: QualityThresholds::default(),
            explicit_fields: 0,
        }
    }
}

impl<'de> Deserialize<'de> for InstanceConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawInstanceConfig::deserialize(deserializer)?;
        let mut explicit = 0u8;
        let set_f32 = |explicit: &mut u8, mask: u8, opt: Option<f32>, target: &mut f32| {
            if let Some(val) = opt {
                *explicit |= mask;
                *target = val;
            }
        };

        let mut config = Self::default();
        set_f32(
            &mut explicit,
            FIELD_DETECTION_CONF,
            raw.detection_confidence_threshold,
            &mut config.detection_confidence_threshold,
        );
        set_f32(
            &mut explicit,
            FIELD_PERSON_CONF,
            raw.person_confidence_threshold,
            &mut config.person_confidence_threshold,
        );
        if let Some(v) = raw.min_face_size {
            explicit |= FIELD_MIN_FACE_SIZE;
            config.min_face_size = v;
        }

        if let Some(nested) = raw.quality_thresholds {
            set_f32(
                &mut explicit,
                FIELD_QUALITY_MIN_SCORE,
                nested.min_score,
                &mut config.quality_thresholds.min_score,
            );
            set_f32(
                &mut explicit,
                FIELD_QUALITY_MAX_YAW,
                nested.max_yaw,
                &mut config.quality_thresholds.max_yaw,
            );
            set_f32(
                &mut explicit,
                FIELD_QUALITY_MAX_PITCH,
                nested.max_pitch,
                &mut config.quality_thresholds.max_pitch,
            );
            set_f32(
                &mut explicit,
                FIELD_QUALITY_MAX_BLUR,
                nested.max_blur,
                &mut config.quality_thresholds.max_blur,
            );
        }
        set_f32(
            &mut explicit,
            FIELD_QUALITY_MIN_SCORE,
            raw.quality_min_score,
            &mut config.quality_thresholds.min_score,
        );
        set_f32(
            &mut explicit,
            FIELD_QUALITY_MAX_YAW,
            raw.quality_max_yaw,
            &mut config.quality_thresholds.max_yaw,
        );
        set_f32(
            &mut explicit,
            FIELD_QUALITY_MAX_PITCH,
            raw.quality_max_pitch,
            &mut config.quality_thresholds.max_pitch,
        );
        set_f32(
            &mut explicit,
            FIELD_QUALITY_MAX_BLUR,
            raw.quality_max_blur,
            &mut config.quality_thresholds.max_blur,
        );

        config.explicit_fields = explicit;
        Ok(config)
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
        let apply_f32 = |mask: u8, key: &str, target: &mut f32| {
            if self.explicit_fields & mask == 0 {
                if let Some(v) = env.get_f32(key) {
                    *target = v;
                }
            }
        };

        apply_f32(
            FIELD_DETECTION_CONF,
            "detection_confidence_threshold",
            &mut self.detection_confidence_threshold,
        );
        apply_f32(
            FIELD_PERSON_CONF,
            "person_confidence_threshold",
            &mut self.person_confidence_threshold,
        );
        if self.explicit_fields & FIELD_MIN_FACE_SIZE == 0 {
            if let Some(v) = env.get_u32("min_face_size") {
                self.min_face_size = v;
            }
        }
        apply_f32(
            FIELD_QUALITY_MIN_SCORE,
            "quality_min_score",
            &mut self.quality_thresholds.min_score,
        );
        apply_f32(
            FIELD_QUALITY_MAX_YAW,
            "quality_max_yaw",
            &mut self.quality_thresholds.max_yaw,
        );
        apply_f32(
            FIELD_QUALITY_MAX_PITCH,
            "quality_max_pitch",
            &mut self.quality_thresholds.max_pitch,
        );
        apply_f32(
            FIELD_QUALITY_MAX_BLUR,
            "quality_max_blur",
            &mut self.quality_thresholds.max_blur,
        );
    }

    /// 校验来自 ABI 配置 JSON 的数值范围。
    pub fn validate(&self) -> Result<(), String> {
        if !self.detection_confidence_threshold.is_finite()
            || !(0.0..=1.0).contains(&self.detection_confidence_threshold)
        {
            return Err("detection_confidence_threshold 必须位于 [0, 1]".to_string());
        }
        if !self.person_confidence_threshold.is_finite()
            || !(0.0..=1.0).contains(&self.person_confidence_threshold)
        {
            return Err("person_confidence_threshold 必须位于 [0, 1]".to_string());
        }
        if self.min_face_size == 0 {
            return Err("min_face_size 必须大于 0".to_string());
        }
        self.quality_thresholds.validate()
    }
}

/// 质量门控阈值。
#[derive(Debug, Clone, Copy, Deserialize, PartialEq)]
pub struct QualityThresholds {
    #[serde(default = "QualityThresholds::default_min_score")]
    pub min_score: f32,
    #[serde(default = "QualityThresholds::default_max_yaw")]
    pub max_yaw: f32,
    #[serde(default = "QualityThresholds::default_max_pitch")]
    pub max_pitch: f32,
    #[serde(default = "QualityThresholds::default_max_blur")]
    pub max_blur: f32,
}

impl QualityThresholds {
    const fn default_min_score() -> f32 {
        0.3
    }
    const fn default_max_yaw() -> f32 {
        45.0
    }
    const fn default_max_pitch() -> f32 {
        30.0
    }
    const fn default_max_blur() -> f32 {
        0.7
    }

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

impl Default for QualityThresholds {
    fn default() -> Self {
        Self {
            min_score: Self::default_min_score(),
            max_yaw: Self::default_max_yaw(),
            max_pitch: Self::default_max_pitch(),
            max_blur: Self::default_max_blur(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_expected_values() {
        let config = InstanceConfig::default();
        assert_eq!(config.detection_confidence_threshold, 0.25);
        assert_eq!(config.person_confidence_threshold, 0.4);
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
            "person_confidence_threshold": 0.35,
            "min_face_size": 48,
            "quality_min_score": 0.55,
            "quality_max_yaw": 30.0,
            "quality_max_pitch": 20.0,
            "quality_max_blur": 0.6
        }"#;

        let config: InstanceConfig = serde_json::from_str(json).expect("解析扁平配置应当成功");
        assert_eq!(config.detection_confidence_threshold, 0.4);
        assert_eq!(config.person_confidence_threshold, 0.35);
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

    fn load_schema_properties() -> serde_json::Map<String, serde_json::Value> {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/config.schema.json");
        let raw = std::fs::read_to_string(path).expect("读取 config.schema.json 失败");
        let schema: serde_json::Value =
            serde_json::from_str(&raw).expect("解析 config.schema.json 失败");
        schema
            .get("properties")
            .and_then(|value| value.as_object())
            .expect("config.schema.json 缺少 properties")
            .clone()
    }

    /// 运行时自有字段必须全部出现在控制台 schema 中。
    ///
    /// schema 的 `additionalProperties: false` 会让控制台无法下发未声明字段，
    /// 字段一旦漏写，运行时只能吃硬编码默认值（历史缺陷：`person_confidence_threshold`）。
    #[test]
    fn schema_exposes_every_runtime_field() {
        let properties = load_schema_properties();
        for key in [
            "detection_confidence_threshold",
            "person_confidence_threshold",
            "min_face_size",
            "quality_min_score",
            "quality_max_yaw",
            "quality_max_pitch",
            "quality_max_blur",
        ] {
            let prop = properties
                .get(key)
                .unwrap_or_else(|| panic!("config.schema.json 缺少运行时字段 {key}"));
            assert!(
                prop.get("default").is_some(),
                "config.schema.json 的 {key} 必须声明 default，否则与运行时缺省值无从对齐"
            );
        }
    }

    /// schema 声明的默认值必须与运行时缺省值逐字段一致。
    ///
    /// 否则「控制台未改动表单」与「宿主未下发该字段」两条路径会得到不同阈值
    /// （历史缺陷：detection 在 schema 为 0.25、运行时为 0.5）。
    #[test]
    fn schema_defaults_match_runtime_defaults() {
        let properties = load_schema_properties();
        let defaults: serde_json::Map<String, serde_json::Value> = properties
            .iter()
            .filter_map(|(key, prop)| {
                prop.get("default")
                    .map(|value| (key.clone(), value.clone()))
            })
            .collect();
        let from_schema: InstanceConfig =
            serde_json::from_value(serde_json::Value::Object(defaults))
                .expect("schema 默认值应当可被 InstanceConfig 解析");
        let runtime = InstanceConfig::default();

        assert_eq!(
            from_schema.detection_confidence_threshold,
            runtime.detection_confidence_threshold
        );
        assert_eq!(
            from_schema.person_confidence_threshold,
            runtime.person_confidence_threshold
        );
        assert_eq!(from_schema.min_face_size, runtime.min_face_size);
        assert_eq!(from_schema.quality_thresholds, runtime.quality_thresholds);
    }

    #[test]
    fn test_precedence_host_overrides_env_and_env_overrides_default() {
        let host_json = r#"{
            "detection_confidence_threshold": 0.85,
            "person_confidence_threshold": 0.65
        }"#;
        let mut config: InstanceConfig =
            serde_json::from_str(host_json).expect("解析宿主配置应当成功");

        let env = PackageEnv::parse_str(
            r#"
            DETECTION_CONFIDENCE_THRESHOLD = 0.10
            PERSON_CONFIDENCE_THRESHOLD = 0.20
            MIN_FACE_SIZE = 50
            QUALITY_MIN_SCORE = 0.60
            "#,
        );

        config.apply_env(&env);

        assert_eq!(config.detection_confidence_threshold, 0.85);
        assert_eq!(config.person_confidence_threshold, 0.65);
        assert_eq!(config.min_face_size, 50);
        assert_eq!(config.quality_thresholds.min_score, 0.60);
        assert_eq!(config.quality_thresholds.max_yaw, 45.0);
    }
}
