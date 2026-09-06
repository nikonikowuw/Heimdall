//! 模型张量后处理与 Letterbox 坐标反算

use algo_sdk::cv::types::PreprocessMode;
use algo_sdk::math::{unmap_box, NormBox};

use crate::config::{ClassMask, InstanceConfig, COCO_CLASSES};

const MODEL_INPUT_WIDTH: f32 = 640.0;
const MODEL_INPUT_HEIGHT: f32 = 384.0;
const TOTAL_CANDIDATES: usize = 300;

/// 解析并反算模型输出候选框
pub fn parse_and_unmap_detections(
    net_out: &[f32],
    config: &InstanceConfig,
    class_mask: &ClassMask,
    mode: &PreprocessMode,
    orig_w: u32,
    orig_h: u32,
) -> Vec<NormBox> {
    if net_out.len() != TOTAL_CANDIDATES * 6 || orig_w == 0 || orig_h == 0 {
        return Vec::new();
    }

    let mut boxes = Vec::new();

    for i in 0..TOTAL_CANDIDATES {
        let offset = i * 6;
        let score = net_out[offset + 4];
        let cls_id_float = net_out[offset + 5];

        if !score.is_finite() || score < config.confidence_threshold {
            continue;
        }

        if !cls_id_float.is_finite() || cls_id_float < 0.0 {
            continue;
        }

        let cls_id = cls_id_float.round() as usize;
        if cls_id >= 80 || !class_mask.is_enabled(cls_id) {
            continue;
        }

        let x1 = net_out[offset];
        let y1 = net_out[offset + 1];
        let x2 = net_out[offset + 2];
        let y2 = net_out[offset + 3];

        if !x1.is_finite() || !y1.is_finite() || !x2.is_finite() || !y2.is_finite() {
            continue;
        }

        if x2 <= x1 || y2 <= y1 {
            continue;
        }

        // 归一化至模型尺度 [640, 384] 的 [0.0, 1.0] 空间
        let norm_x = x1 / MODEL_INPUT_WIDTH;
        let norm_y = y1 / MODEL_INPUT_HEIGHT;
        let norm_w = (x2 - x1) / MODEL_INPUT_WIDTH;
        let norm_h = (y2 - y1) / MODEL_INPUT_HEIGHT;

        let raw_box = NormBox::new(norm_x, norm_y, norm_w, norm_h, score, cls_id as u32)
            .with_label(COCO_CLASSES[cls_id]);

        // 精准扣除四周黑边并还原至原始视频帧真实比例 [0.0, 1.0]
        let mut final_box = unmap_box(&raw_box, mode, orig_w, orig_h);

        // 如果配置了自定义告警标签且非空，覆盖默认类别标签
        if let Some(ref custom_label) = config.custom_alarm_label {
            if !custom_label.is_empty() {
                final_box.label = Some(Box::leak(custom_label.clone().into_boxed_str()));
            }
        }

        boxes.push(final_box);
    }

    boxes
}

#[cfg(test)]
mod tests {
    use super::*;
    use algo_sdk::cv::types::LetterboxLayout;

    #[test]
    fn test_postprocess_filters_and_unmaps() {
        let mut net_out = vec![0.0f32; 1800];
        // 目标 0: person, 置信度 0.9, 坐标 [100, 50, 200, 150]
        net_out[0] = 100.0;
        net_out[1] = 50.0;
        net_out[2] = 200.0;
        net_out[3] = 150.0;
        net_out[4] = 0.9;
        net_out[5] = 0.0; // person

        // 目标 1: 低置信度 (0.2 < 0.45)
        net_out[6 + 4] = 0.2;
        net_out[6 + 5] = 0.0;

        let config = InstanceConfig::default();
        let mask = ClassMask::from_classes(&config.target_classes);
        let layout = LetterboxLayout {
            scaled_w: 640,
            scaled_h: 360,
            pad_left: 0,
            pad_top: 12,
            scale: 1.0,
            dst_w: 640,
            dst_h: 384,
        };
        let mode = PreprocessMode::Letterbox(layout);

        let boxes = parse_and_unmap_detections(&net_out, &config, &mask, &mode, 1920, 1080);
        assert_eq!(boxes.len(), 1);
        assert_eq!(boxes[0].class_id, 0);
        assert_eq!(boxes[0].label, Some("person"));
        assert!(boxes[0].confidence >= 0.9);
        assert!(boxes[0].x > 0.0 && boxes[0].x < 1.0);
        assert!(boxes[0].w > 0.0 && boxes[0].w < 1.0);
    }
}
