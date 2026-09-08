use serde::Deserialize;

fn default_detection_threshold() -> f32 {
    0.5
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

/// 人脸识别算法实例配置。
#[derive(Debug, Clone, Deserialize)]
pub struct InstanceConfig {
    #[serde(default = "default_detection_threshold")]
    pub detection_confidence_threshold: f32,
    #[serde(default = "default_min_face_size")]
    pub min_face_size: u32,
    #[serde(default)]
    pub quality_thresholds: QualityThresholds,
}

impl Default for InstanceConfig {
    fn default() -> Self {
        Self {
            detection_confidence_threshold: default_detection_threshold(),
            min_face_size: default_min_face_size(),
            quality_thresholds: QualityThresholds::default(),
        }
    }
}

impl InstanceConfig {
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
#[derive(Debug, Clone, Deserialize)]
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
    fn default_config_is_valid() {
        let config = InstanceConfig::default();
        assert!(config.validate().is_ok());
        assert_eq!(config.min_face_size, 30);
    }

    #[test]
    fn config_round_trip_and_range_validation() {
        let config: InstanceConfig = serde_json::from_str(
            r#"{
                "detection_confidence_threshold": 0.65,
                "min_face_size": 48,
                "quality_thresholds": {
                    "min_score": 0.4,
                    "max_yaw": 35.0,
                    "max_pitch": 25.0,
                    "max_blur": 0.6
                }
            }"#,
        )
        .expect("配置 JSON 应可解析");
        assert!(config.validate().is_ok());
        assert_eq!(config.min_face_size, 48);

        let mut invalid = config;
        invalid.detection_confidence_threshold = 1.2;
        assert!(invalid.validate().is_err());
    }
}
