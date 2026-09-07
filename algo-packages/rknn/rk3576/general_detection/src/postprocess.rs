//! 模型张量后处理、YOLOv8 锚点解码、NMS 与 Letterbox 坐标反算

use algo_sdk::cv::types::PreprocessMode;
use algo_sdk::math::{fast_nms, unmap_box, NormBox};

use crate::config::{ClassMask, InstanceConfig, COCO_CLASSES};
use crate::rknn::RknnInferenceOutput;

pub const MODEL_INPUT_WIDTH: f32 = 640.0;
pub const MODEL_INPUT_HEIGHT: f32 = 384.0;
pub const TOTAL_ANCHORS: usize = 5040;
pub const NUM_CHANNELS: usize = 84; // 4 bbox (cx, cy, w, h) + 80 class scores

#[inline(always)]
fn dequant_i8(val: i8, zp: i32, scale: f32) -> f32 {
    if !scale.is_finite() || scale <= 0.0 {
        return 0.0;
    }
    (val as i32 - zp) as f32 * scale
}

#[inline(always)]
fn quant_f32(val: f32, zp: i32, scale: f32) -> i8 {
    if !scale.is_finite() || scale <= 0.0 || !val.is_finite() {
        return zp.clamp(-128, 127) as i8;
    }
    let v = val / scale + zp as f32;
    if !v.is_finite() {
        return zp.clamp(-128, 127) as i8;
    }
    v.clamp(-128.0, 127.0).round() as i8
}

fn decode_dfl(before_dfl: &[f32; 64], dfl_box: &mut [f32; 4]) {
    for k in 0..4 {
        let reg = &before_dfl[k * 16..(k + 1) * 16];
        let mut max_v = reg[0];
        for &v in &reg[1..16] {
            if v > max_v {
                max_v = v;
            }
        }
        let mut sum = 0.0f32;
        let mut exp_dfl = [0.0f32; 16];
        for i in 0..16 {
            exp_dfl[i] = (reg[i] - max_v).exp();
            sum += exp_dfl[i];
        }
        let inv_sum = 1.0 / sum;
        let mut acc = 0.0f32;
        for (i, &val) in exp_dfl.iter().enumerate() {
            acc += (val * inv_sum) * (i as f32);
        }
        dfl_box[k] = acc;
    }
}

/// 统一入口：解析 RKNN 推理输出，执行置信度过滤、DFL 解码、NMS 与 Letterbox 坐标反算
pub fn parse_and_unmap_output(
    output: &RknnInferenceOutput<'_>,
    config: &InstanceConfig,
    class_mask: &ClassMask,
    custom_label: Option<&'static str>,
    mode: &PreprocessMode,
    orig_w: u32,
    orig_h: u32,
) -> Vec<NormBox> {
    if orig_w == 0 || orig_h == 0 {
        return Vec::new();
    }

    match output {
        RknnInferenceOutput::MultiBranch(branches) => parse_multi_branch_int8(
            branches,
            config,
            class_mask,
            custom_label,
            mode,
            orig_w,
            orig_h,
        ),
        RknnInferenceOutput::SingleFloat(slice) => parse_single_float(
            slice,
            config,
            class_mask,
            custom_label,
            mode,
            orig_w,
            orig_h,
        ),
    }
}

/// 解析多分支解耦 INT8 张量输出 (YOLOv8 官方 6 输出分层结构)
fn parse_multi_branch_int8(
    branches: &[crate::rknn::RknnTensorOutput<'_>],
    config: &InstanceConfig,
    class_mask: &ClassMask,
    custom_label: Option<&'static str>,
    mode: &PreprocessMode,
    orig_w: u32,
    orig_h: u32,
) -> Vec<NormBox> {
    if branches.len() < 6 {
        return Vec::new();
    }

    let strides = [8, 16, 32];
    let mut candidates = Vec::with_capacity(32);
    let all_enabled = class_mask.is_all_enabled();

    for s in 0..3 {
        let stride = strides[s];
        let grid_w = (MODEL_INPUT_WIDTH as usize) / stride;
        let grid_h = (MODEL_INPUT_HEIGHT as usize) / stride;
        let grid_len = grid_w * grid_h;

        let box_out = &branches[s * 2];
        let cls_out = &branches[s * 2 + 1];

        if cls_out.data.len() < 80 * grid_len || box_out.data.len() < 64 * grid_len {
            continue;
        }

        let cls_th_i8 = quant_f32(config.confidence_threshold, cls_out.zp, cls_out.scale);

        for offset in 0..grid_len {
            let mut max_class_id = -1i32;
            let mut max_score_i8 = -128i8;

            // 遍历 80 类别的得分，快筛过滤
            let mut c_offset = offset;
            if all_enabled {
                for c in 0..80 {
                    // SAFETY: 预先已断言 cls_out.data.len() >= 80 * grid_len，c_offset 最大为 offset + 79 * grid_len < 80 * grid_len
                    let val = unsafe { *cls_out.data.get_unchecked(c_offset) };
                    if val > cls_th_i8 && val > max_score_i8 {
                        max_score_i8 = val;
                        max_class_id = c;
                    }
                    c_offset += grid_len;
                }
            } else {
                for c in 0..80 {
                    if class_mask.is_enabled(c as usize) {
                        // SAFETY: 预先已断言 cls_out.data.len() >= 80 * grid_len，c_offset 最大为 offset + 79 * grid_len < 80 * grid_len
                        let val = unsafe { *cls_out.data.get_unchecked(c_offset) };
                        if val > cls_th_i8 && val > max_score_i8 {
                            max_score_i8 = val;
                            max_class_id = c;
                        }
                    }
                    c_offset += grid_len;
                }
            }

            if max_class_id >= 0 && max_score_i8 > cls_th_i8 {
                let i = offset / grid_w;
                let j = offset % grid_w;
                let mut b_offset = offset;
                let mut before_dfl = [0.0f32; 64];
                for item in &mut before_dfl {
                    // SAFETY: 预先已断言 box_out.data.len() >= 64 * grid_len，b_offset 最大为 offset + 63 * grid_len < 64 * grid_len
                    let val = unsafe { *box_out.data.get_unchecked(b_offset) };
                    *item = dequant_i8(val, box_out.zp, box_out.scale);
                    b_offset += grid_len;
                }
                let mut dfl_box = [0.0f32; 4];
                decode_dfl(&before_dfl, &mut dfl_box);

                let x1 = (-dfl_box[0] + j as f32 + 0.5) * stride as f32;
                let y1 = (-dfl_box[1] + i as f32 + 0.5) * stride as f32;
                let x2 = (dfl_box[2] + j as f32 + 0.5) * stride as f32;
                let y2 = (dfl_box[3] + i as f32 + 0.5) * stride as f32;

                let raw = NormBox {
                    x: (x1 / MODEL_INPUT_WIDTH).clamp(0.0, 1.0),
                    y: (y1 / MODEL_INPUT_HEIGHT).clamp(0.0, 1.0),
                    w: ((x2 - x1) / MODEL_INPUT_WIDTH).clamp(0.0, 1.0),
                    h: ((y2 - y1) / MODEL_INPUT_HEIGHT).clamp(0.0, 1.0),
                    confidence: dequant_i8(max_score_i8, cls_out.zp, cls_out.scale),
                    class_id: max_class_id as u32,
                    label: if let Some(custom) = custom_label {
                        Some(custom)
                    } else {
                        COCO_CLASSES.get(max_class_id as usize).copied()
                    },
                };
                candidates.push(unmap_box(&raw, mode, orig_w, orig_h));
            }
        }
    }

    if candidates.is_empty() {
        return Vec::new();
    }
    fast_nms(&mut candidates, config.iou_threshold);
    candidates
}

/// 解析单浮点切片输出 [1, 84, 5040] (用于 debug_cpu_fallback_path 与单分支模型)
fn parse_single_float(
    net_out: &[f32],
    config: &InstanceConfig,
    class_mask: &ClassMask,
    custom_label: Option<&'static str>,
    mode: &PreprocessMode,
    orig_w: u32,
    orig_h: u32,
) -> Vec<NormBox> {
    if net_out.len() != NUM_CHANNELS * TOTAL_ANCHORS {
        return Vec::new();
    }

    let mut candidates = Vec::with_capacity(32);
    let all_enabled = class_mask.is_all_enabled();

    for i in 0..TOTAL_ANCHORS {
        let mut max_score = 0.0f32;
        let mut best_class = 0usize;

        if all_enabled {
            for c in 0..80 {
                let score = net_out[(4 + c) * TOTAL_ANCHORS + i];
                if score > max_score {
                    max_score = score;
                    best_class = c;
                }
            }
        } else {
            for c in 0..80 {
                if !class_mask.is_enabled(c) {
                    continue;
                }
                let score = net_out[(4 + c) * TOTAL_ANCHORS + i];
                if score > max_score {
                    max_score = score;
                    best_class = c;
                }
            }
        }

        if !max_score.is_finite() || max_score < config.confidence_threshold {
            continue;
        }

        let cx = net_out[i];
        let cy = net_out[TOTAL_ANCHORS + i];
        let w = net_out[2 * TOTAL_ANCHORS + i];
        let h = net_out[3 * TOTAL_ANCHORS + i];

        let x1 = cx - w * 0.5;
        let y1 = cy - h * 0.5;

        let raw = NormBox {
            x: (x1 / MODEL_INPUT_WIDTH).clamp(0.0, 1.0),
            y: (y1 / MODEL_INPUT_HEIGHT).clamp(0.0, 1.0),
            w: (w / MODEL_INPUT_WIDTH).clamp(0.0, 1.0),
            h: (h / MODEL_INPUT_HEIGHT).clamp(0.0, 1.0),
            confidence: max_score,
            class_id: best_class as u32,
            label: if let Some(custom) = custom_label {
                Some(custom)
            } else {
                COCO_CLASSES.get(best_class).copied()
            },
        };
        candidates.push(unmap_box(&raw, mode, orig_w, orig_h));
    }

    if candidates.is_empty() {
        return Vec::new();
    }
    fast_nms(&mut candidates, config.iou_threshold);
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;
    use algo_sdk::cv::types::LetterboxLayout;

    #[test]
    fn test_single_float_fallback_decoding() {
        let mut net_out = vec![0.0f32; NUM_CHANNELS * TOTAL_ANCHORS];

        // 构造锚点 10: 目标为 person (class 0), 中心 (320, 192), 宽高 (100, 80), 得分 0.95
        let anchor_a = 10;
        net_out[anchor_a] = 320.0; // cx
        net_out[TOTAL_ANCHORS + anchor_a] = 192.0; // cy
        net_out[2 * TOTAL_ANCHORS + anchor_a] = 100.0; // w
        net_out[3 * TOTAL_ANCHORS + anchor_a] = 80.0; // h
        net_out[4 * TOTAL_ANCHORS + anchor_a] = 0.95; // score person

        let config = InstanceConfig::default();
        let mask = ClassMask::from_classes(&config.target_classes);

        let layout = LetterboxLayout {
            scaled_w: 640,
            scaled_h: 360,
            pad_left: 0,
            pad_top: 12,
            scale: 1.0 / 3.0,
            dst_w: 640,
            dst_h: 384,
        };
        let mode = PreprocessMode::Letterbox(layout);

        let out = RknnInferenceOutput::SingleFloat(&net_out);
        let boxes = parse_and_unmap_output(&out, &config, &mask, None, &mode, 1920, 1080);

        assert_eq!(boxes.len(), 1);
        let b = &boxes[0];
        assert_eq!(b.class_id, 0);
        assert_eq!(b.label, Some("person"));
        assert_eq!(b.confidence, 0.95);
    }

    #[test]
    fn test_quant_and_dequant_defensive_safety() {
        // scale 为 0 或负数时不能 panic
        assert_eq!(quant_f32(0.5, 0, 0.0), 0);
        assert_eq!(quant_f32(0.5, 0, -1.0), 0);
        assert_eq!(quant_f32(f32::NAN, 0, 0.1), 0);
        assert_eq!(dequant_i8(10, 0, 0.0), 0.0);
        assert_eq!(dequant_i8(10, 0, f32::NAN), 0.0);

        // 正常量化反量化
        let q = quant_f32(0.5, 0, 0.01);
        assert_eq!(q, 50);
        let dq = dequant_i8(q, 0, 0.01);
        assert!((dq - 0.5).abs() < 1e-5);
    }
}
