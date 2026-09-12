//! 安全帽检测后处理：委托 algo-sdk 通用 YOLOv8 解析器
//!
//! 模型输出 9 个张量（P3/P4/P5 各 3 个）：
//! - `box`  [1, 64, H, W]  — DFL 16-bin 表示的 4 坐标
//! - `cls`  [1,  2, H, W]  — Hardhat / NO-Hardhat 分类得分
//! - `score_sum` [1, 1, H, W] — cls 各通道求和，用于快速过滤

use algo_sdk::cv::postprocess::{parse_yolov8_int8, Yolov8ParseContext, Yolov8RknnConfig};
use algo_sdk::cv::types::PreprocessMode;
use algo_sdk::math::NormBox;

use crate::config::InstanceConfig;
use crate::rknn::RknnInferenceOutput;

pub const MODEL_INPUT_WIDTH: f32 = 640.0;
pub const MODEL_INPUT_HEIGHT: f32 = 384.0;

/// 单浮点输出模式的锚点总数
pub const TOTAL_ANCHORS: usize = 5040;
/// 单浮点输出模式的通道数
pub const NUM_CHANNELS: usize = 84;

/// 安全帽类别标签
const HELMET_CLASSES: [&str; 2] = ["Hardhat", "NO-Hardhat"];

/// 通用 YOLOv8 RKNN 配置（安全帽模型参数）
fn sdk_config() -> Yolov8RknnConfig {
    Yolov8RknnConfig {
        model_input_w: MODEL_INPUT_WIDTH,
        model_input_h: MODEL_INPUT_HEIGHT,
        dfl_bins: 16,
        num_classes: 2,
        use_score_sum: true,
    }
}

/// 统一入口：解析 RKNN 推理输出
pub fn parse_and_unmap_output(
    output: &RknnInferenceOutput<'_>,
    config: &InstanceConfig,
    custom_label: Option<&'static str>,
    mode: &PreprocessMode,
    orig_w: u32,
    orig_h: u32,
) -> Vec<NormBox> {
    if orig_w == 0 || orig_h == 0 {
        return Vec::new();
    }

    match output {
        RknnInferenceOutput::MultiBranch(branches) => {
            let ctx = Yolov8ParseContext {
                branches,
                config: &sdk_config(),
                conf_threshold: config.confidence_threshold,
                iou_threshold: config.iou_threshold,
                labels: &HELMET_CLASSES,
                label_fn: None,
                mode,
                orig_w,
                orig_h,
            };
            let mut boxes = parse_yolov8_int8(&ctx);
            // 自定义标签覆盖
            if let Some(label) = custom_label {
                for b in &mut boxes {
                    b.label = Some(label);
                }
            }
            boxes
        }
        RknnInferenceOutput::SingleFloat(slice) => {
            parse_single_float_fallback(slice, config, custom_label, mode, orig_w, orig_h)
        }
    }
}

/// 单浮点输出的 fallback 解析（debug_cpu_fallback_path 使用）
fn parse_single_float_fallback(
    net_out: &[f32],
    config: &InstanceConfig,
    custom_label: Option<&'static str>,
    mode: &PreprocessMode,
    orig_w: u32,
    orig_h: u32,
) -> Vec<NormBox> {
    use algo_sdk::math::{fast_nms, unmap_box};

    if net_out.len() < NUM_CHANNELS * TOTAL_ANCHORS {
        return Vec::new();
    }

    let mut candidates = Vec::with_capacity(32);
    let conf_thresh = config.confidence_threshold;

    for i in 0..TOTAL_ANCHORS {
        let score_0 = net_out[4 * TOTAL_ANCHORS + i];
        let score_1 = net_out[5 * TOTAL_ANCHORS + i];

        let (best_class, max_score) = if score_0 >= score_1 {
            (0usize, score_0)
        } else {
            (1usize, score_1)
        };

        if !max_score.is_finite() || max_score < conf_thresh {
            continue;
        }

        let cx = net_out[i];
        let cy = net_out[TOTAL_ANCHORS + i];
        let w = net_out[2 * TOTAL_ANCHORS + i];
        let h = net_out[3 * TOTAL_ANCHORS + i];

        let raw = NormBox {
            x: ((cx - w * 0.5) / MODEL_INPUT_WIDTH).clamp(0.0, 1.0),
            y: ((cy - h * 0.5) / MODEL_INPUT_HEIGHT).clamp(0.0, 1.0),
            w: (w / MODEL_INPUT_WIDTH).clamp(0.0, 1.0),
            h: (h / MODEL_INPUT_HEIGHT).clamp(0.0, 1.0),
            confidence: max_score,
            class_id: best_class as u32,
            label: if let Some(custom) = custom_label {
                Some(custom)
            } else {
                HELMET_CLASSES.get(best_class).copied()
            },
        };
        candidates.push(unmap_box(&raw, mode, orig_w, orig_h));
    }

    if !candidates.is_empty() {
        fast_nms(&mut candidates, config.iou_threshold);
    }
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;
    use algo_sdk::cv::types::LetterboxLayout;

    fn test_mode() -> PreprocessMode {
        PreprocessMode::Letterbox(LetterboxLayout {
            scaled_w: 640,
            scaled_h: 360,
            pad_left: 0,
            pad_top: 12,
            scale: 1.0 / 3.0,
            dst_w: 640,
            dst_h: 384,
        })
    }

    #[test]
    fn test_single_float_fallback_decoding() {
        let mut net_out = vec![0.0f32; NUM_CHANNELS * TOTAL_ANCHORS];

        // 锚点 10: Hardhat (class 0), 中心 (320, 192), 宽高 (100, 80), 得分 0.95
        let a = 10;
        net_out[a] = 320.0;
        net_out[TOTAL_ANCHORS + a] = 192.0;
        net_out[2 * TOTAL_ANCHORS + a] = 100.0;
        net_out[3 * TOTAL_ANCHORS + a] = 80.0;
        net_out[4 * TOTAL_ANCHORS + a] = 0.95;

        let config = InstanceConfig::default();
        let out = RknnInferenceOutput::SingleFloat(&net_out);
        let boxes = parse_and_unmap_output(&out, &config, None, &test_mode(), 1920, 1080);

        assert_eq!(boxes.len(), 1);
        assert_eq!(boxes[0].class_id, 0);
        assert_eq!(boxes[0].label, Some("Hardhat"));
        assert!((boxes[0].confidence - 0.95).abs() < 1e-5);
    }

    #[test]
    fn test_single_float_two_classes() {
        let mut net_out = vec![0.0f32; NUM_CHANNELS * TOTAL_ANCHORS];

        // 锚点 5: NO-Hardhat (class 1), 置信度 0.87
        let a = 5;
        net_out[a] = 100.0;
        net_out[TOTAL_ANCHORS + a] = 200.0;
        net_out[2 * TOTAL_ANCHORS + a] = 60.0;
        net_out[3 * TOTAL_ANCHORS + a] = 50.0;
        net_out[(4 + 1) * TOTAL_ANCHORS + a] = 0.87;

        let config = InstanceConfig::default();
        let out = RknnInferenceOutput::SingleFloat(&net_out);
        let boxes = parse_and_unmap_output(&out, &config, None, &test_mode(), 1920, 1080);

        assert_eq!(boxes.len(), 1);
        assert_eq!(boxes[0].class_id, 1);
        assert_eq!(boxes[0].label, Some("NO-Hardhat"));
    }

    #[test]
    fn test_custom_label_override() {
        let mut net_out = vec![0.0f32; NUM_CHANNELS * TOTAL_ANCHORS];
        let a = 10;
        net_out[a] = 320.0;
        net_out[TOTAL_ANCHORS + a] = 192.0;
        net_out[2 * TOTAL_ANCHORS + a] = 100.0;
        net_out[3 * TOTAL_ANCHORS + a] = 80.0;
        net_out[4 * TOTAL_ANCHORS + a] = 0.92;

        let config = InstanceConfig {
            custom_alarm_label: Some("安全检查".to_string()),
            ..Default::default()
        };
        let out = RknnInferenceOutput::SingleFloat(&net_out);
        let boxes =
            parse_and_unmap_output(&out, &config, Some("安全检查"), &test_mode(), 1920, 1080);

        assert_eq!(boxes.len(), 1);
        assert_eq!(boxes[0].label, Some("安全检查"));
    }
}
