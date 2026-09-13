//! 算法配置定义与 COCO 80 类别位掩码过滤

use std::collections::HashSet;

use algo_sdk::env::PackageEnv;
use serde::Deserialize;

pub const COCO_CLASSES: [&str; 80] = [
    "person",
    "bicycle",
    "car",
    "motorcycle",
    "airplane",
    "bus",
    "train",
    "truck",
    "boat",
    "traffic light",
    "fire hydrant",
    "stop sign",
    "parking meter",
    "bench",
    "bird",
    "cat",
    "dog",
    "horse",
    "sheep",
    "cow",
    "elephant",
    "bear",
    "zebra",
    "giraffe",
    "backpack",
    "umbrella",
    "handbag",
    "tie",
    "suitcase",
    "frisbee",
    "skis",
    "snowboard",
    "sports ball",
    "kite",
    "baseball bat",
    "baseball glove",
    "skateboard",
    "surfboard",
    "tennis racket",
    "bottle",
    "wine glass",
    "cup",
    "fork",
    "knife",
    "spoon",
    "bowl",
    "banana",
    "apple",
    "sandwich",
    "orange",
    "broccoli",
    "carrot",
    "hot dog",
    "pizza",
    "donut",
    "cake",
    "chair",
    "couch",
    "potted plant",
    "bed",
    "dining table",
    "toilet",
    "tv",
    "laptop",
    "mouse",
    "remote",
    "keyboard",
    "cell phone",
    "microwave",
    "oven",
    "toaster",
    "sink",
    "refrigerator",
    "book",
    "clock",
    "vase",
    "scissors",
    "teddy bear",
    "hair drier",
    "toothbrush",
];

pub fn get_coco_class_id(name: &str) -> Option<usize> {
    COCO_CLASSES.iter().position(|&c| c == name)
}

fn default_confidence() -> f32 {
    0.45
}

fn default_iou() -> f32 {
    0.45
}

fn default_target_classes() -> Vec<String> {
    vec![
        "person".to_string(),
        "car".to_string(),
        "motorcycle".to_string(),
        "bicycle".to_string(),
        "bus".to_string(),
        "truck".to_string(),
    ]
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
}

/// 实例运行时配置
#[derive(Debug, Clone, PartialEq)]
pub struct InstanceConfig {
    pub confidence_threshold: f32,
    pub iou_threshold: f32,
    pub target_classes: Vec<String>,
    pub custom_alarm_label: Option<String>,
    pub explicit_fields: HashSet<String>,
}

impl<'de> Deserialize<'de> for InstanceConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawInstanceConfig::deserialize(deserializer)?;
        let mut explicit_fields = HashSet::new();

        let confidence_threshold = if let Some(v) = raw.confidence_threshold {
            explicit_fields.insert("confidence_threshold".to_string());
            v
        } else {
            default_confidence()
        };

        let iou_threshold = if let Some(v) = raw.iou_threshold {
            explicit_fields.insert("iou_threshold".to_string());
            v
        } else {
            default_iou()
        };

        let target_classes = if let Some(v) = raw.target_classes {
            explicit_fields.insert("target_classes".to_string());
            v
        } else {
            default_target_classes()
        };

        let custom_alarm_label = if let Some(v) = raw.custom_alarm_label {
            explicit_fields.insert("custom_alarm_label".to_string());
            Some(v)
        } else {
            None
        };

        Ok(Self {
            confidence_threshold,
            iou_threshold,
            target_classes,
            custom_alarm_label,
            explicit_fields,
        })
    }
}

impl Default for InstanceConfig {
    fn default() -> Self {
        Self {
            confidence_threshold: default_confidence(),
            iou_threshold: default_iou(),
            target_classes: default_target_classes(),
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
        if !self.explicit_fields.contains("target_classes") {
            if let Some(v) = env.get_str("target_classes") {
                self.target_classes = v
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
        }
    }
}

/// COCO 80 类别的 O(1) 位掩码过滤
#[derive(Debug, Copy, Clone)]
pub struct ClassMask {
    low: u64,
    high: u64,
    all_enabled: bool,
}

impl ClassMask {
    pub fn from_classes(classes: &[String]) -> Self {
        if classes.is_empty() {
            // 空数组默认放行所有类别
            return Self {
                low: u64::MAX,
                high: u64::MAX,
                all_enabled: true,
            };
        }
        let mut low = 0u64;
        let mut high = 0u64;
        let mut count = 0;
        for c in classes {
            if let Some(id) = get_coco_class_id(c.as_str()) {
                if id < 64 {
                    low |= 1 << id;
                    count += 1;
                } else if id < 80 {
                    high |= 1 << (id - 64);
                    count += 1;
                }
            }
        }
        Self {
            low,
            high,
            all_enabled: count >= 80,
        }
    }

    #[inline(always)]
    pub fn is_all_enabled(&self) -> bool {
        self.all_enabled
    }

    #[inline(always)]
    pub fn is_enabled(&self, class_id: usize) -> bool {
        if self.all_enabled {
            true
        } else if class_id < 64 {
            (self.low & (1u64 << class_id)) != 0
        } else if class_id < 80 {
            (self.high & (1u64 << (class_id - 64))) != 0
        } else {
            false
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
        assert_eq!(cfg.target_classes.len(), 6);

        let mask = ClassMask::from_classes(&cfg.target_classes);
        assert!(mask.is_enabled(0)); // person
        assert!(mask.is_enabled(2)); // car
        assert!(!mask.is_enabled(14)); // bird
    }

    #[test]
    fn test_custom_config_deserialization() {
        let json = r#"{"confidence_threshold": 0.6, "target_classes": ["dog", "cat"]}"#;
        let cfg: InstanceConfig = serde_json::from_str(json).expect("解析失败");
        assert_eq!(cfg.confidence_threshold, 0.6);
        assert_eq!(cfg.iou_threshold, 0.45);
        assert_eq!(cfg.target_classes, vec!["dog", "cat"]);

        let mask = ClassMask::from_classes(&cfg.target_classes);
        assert!(mask.is_enabled(15)); // cat
        assert!(mask.is_enabled(16)); // dog
        assert!(!mask.is_enabled(0)); // person
    }

    #[test]
    fn test_precedence_host_overrides_env_and_env_overrides_default() {
        let host_json = r#"{"confidence_threshold": 0.8}"#;
        let mut config: InstanceConfig = serde_json::from_str(host_json).expect("解析宿主配置失败");

        let env = PackageEnv::parse_str(
            r#"
            CONFIDENCE_THRESHOLD = 0.2
            IOU_THRESHOLD = 0.6
            TARGET_CLASSES = "bus,truck"
            "#,
        );

        config.apply_env(&env);

        // 1. 宿主显式指定的置信度保持 0.8
        assert_eq!(config.confidence_threshold, 0.8);
        // 2. 宿主未指定的从 .env 获取
        assert_eq!(config.iou_threshold, 0.6);
        assert_eq!(config.target_classes, vec!["bus", "truck"]);
    }
}
