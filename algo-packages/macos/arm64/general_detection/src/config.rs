//! 算法配置定义与 COCO 80 类别位掩码过滤

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

/// 实例运行时配置
#[derive(Debug, Clone, Deserialize)]
pub struct InstanceConfig {
    #[serde(default = "default_confidence")]
    pub confidence_threshold: f32,
    #[serde(default = "default_iou")]
    pub iou_threshold: f32,
    #[serde(default = "default_target_classes")]
    pub target_classes: Vec<String>,
    #[serde(default)]
    pub custom_alarm_label: Option<String>,
}

impl Default for InstanceConfig {
    fn default() -> Self {
        Self {
            confidence_threshold: default_confidence(),
            iou_threshold: default_iou(),
            target_classes: default_target_classes(),
            custom_alarm_label: None,
        }
    }
}

/// COCO 80 类别的 O(1) 位掩码过滤
#[derive(Debug, Copy, Clone)]
pub struct ClassMask(u128);

impl ClassMask {
    pub fn from_classes(classes: &[String]) -> Self {
        if classes.is_empty() {
            // 空数组默认放行所有类别
            return Self(u128::MAX);
        }
        let mut mask = 0u128;
        for c in classes {
            if let Some(id) = get_coco_class_id(c.as_str()) {
                if id < 80 {
                    mask |= 1 << id;
                }
            }
        }
        Self(mask)
    }

    #[inline]
    pub fn is_enabled(&self, class_id: usize) -> bool {
        if class_id >= 80 {
            false
        } else {
            (self.0 & (1 << class_id)) != 0
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
}
