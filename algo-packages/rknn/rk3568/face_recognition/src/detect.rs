//! YOLOv8n-face 12 张量解码器
//!
//! 输入：RKNN 模型输出的 12 个 float32 张量（3 尺度 × 4 分支：box, score_sum, cls, kpt）
//! 输出：`Vec<RawFace>`，bbox 和 landmarks 归一化到 [0, 1]

use algo_sdk::cv::LetterboxLayout;
use algo_sdk::error::AlgoError;
use algo_sdk::math::clamp_bbox;

/// 每个尺度的输出分支数
const BRANCHES_PER_SCALE: usize = 4;
/// box DFL 的 bin 数量（每条边 16 个分布）
const DFL_LEN: usize = 16;
/// 每个关键点的通道数 (x_offset, y_offset, visibility_conf)
const KPT_CHANNELS_PER_POINT: usize = 3;
/// 人脸关键点数量
pub const NUM_LANDMARKS: usize = 5;

/// YOLOv8n-face 单候选框
#[derive(Debug, Clone)]
pub struct RawFace {
    /// `[x, y, w, h]` 左上角格式，归一化到 [0, 1]
    pub bbox: [f32; 4],
    /// 5 个关键点坐标，归一化到 [0, 1]
    pub landmarks: [[f32; 2]; NUM_LANDMARKS],
    /// 5 个关键点置信度（sigmoid 后）
    pub landmark_scores: [f32; NUM_LANDMARKS],
    /// 检测置信度
    pub score: f32,
}

impl RawFace {
    fn iou(&self, other: &Self) -> f32 {
        let ax2 = self.bbox[0] + self.bbox[2];
        let ay2 = self.bbox[1] + self.bbox[3];
        let bx2 = other.bbox[0] + other.bbox[2];
        let by2 = other.bbox[1] + other.bbox[3];
        let ix1 = self.bbox[0].max(other.bbox[0]);
        let iy1 = self.bbox[1].max(other.bbox[1]);
        let ix2 = ax2.min(bx2);
        let iy2 = ay2.min(by2);
        let iw = (ix2 - ix1).max(0.0);
        let ih = (iy2 - iy1).max(0.0);
        let intersection = iw * ih;
        let union = self.bbox[2] * self.bbox[3] + other.bbox[2] * other.bbox[3] - intersection;
        if union > 0.0 {
            intersection / union
        } else {
            0.0
        }
    }
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// DFL 解码：16-bin softmax 加权求和 → 单个偏移值
///
/// 输入 16 个 logits，输出一个浮点偏移量。
fn compute_dfl(logits: &[f32]) -> Result<f32, AlgoError> {
    if logits.len() != DFL_LEN || logits.iter().any(|value| !value.is_finite()) {
        return Err(AlgoError::Inference {
            reason: "DFL logits 长度或数值非法".to_string(),
        });
    }
    let max_val = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let exp_sum: f32 = logits.iter().map(|&v| (v - max_val).exp()).sum();
    if !exp_sum.is_finite() || exp_sum <= f32::EPSILON {
        return Err(AlgoError::Inference {
            reason: "DFL softmax 分母非法".to_string(),
        });
    }
    Ok(logits
        .iter()
        .enumerate()
        .map(|(i, &v)| (v - max_val).exp() / exp_sum * i as f32)
        .sum())
}

/// 单尺度解码：遍历 grid，解码 box + cls + kpt
///
/// `box_tensor`: `[64, H, W]` 展平为一维，NCHW 布局
/// `score_sum`: `[1, H, W]` 展平
/// `cls_tensor`: `[1, H, W]` 展平
/// `kpt_tensor`: `[15, H, W]` 展平
#[allow(clippy::too_many_arguments)]
fn decode_scale(
    box_tensor: &[f32],
    score_sum: &[f32],
    cls_tensor: &[f32],
    kpt_tensor: &[f32],
    grid_h: usize,
    grid_w: usize,
    stride: usize,
    conf_threshold: f32,
) -> Result<Vec<RawFace>, AlgoError> {
    let grid_len = grid_h.checked_mul(grid_w).ok_or(AlgoError::OutOfMemory)?;
    let required_box = DFL_LEN
        .checked_mul(4)
        .and_then(|channels| channels.checked_mul(grid_len))
        .ok_or(AlgoError::OutOfMemory)?;
    let required_kpt = KPT_CHANNELS_PER_POINT
        .checked_mul(NUM_LANDMARKS)
        .and_then(|channels| channels.checked_mul(grid_len))
        .ok_or(AlgoError::OutOfMemory)?;
    if score_sum.len() < grid_len
        || cls_tensor.len() < grid_len
        || box_tensor.len() < required_box
        || kpt_tensor.len() < required_kpt
    {
        return Err(AlgoError::Inference {
            reason: format!(
                "YOLOv8-face 输出缓冲区不足: grid={grid_h}x{grid_w}, box={}, score={}, cls={}, kpt={}",
                box_tensor.len(),
                score_sum.len(),
                cls_tensor.len(),
                kpt_tensor.len()
            ),
        });
    }
    let mut faces = Vec::new();

    for gy in 0..grid_h {
        for gx in 0..grid_w {
            let offset = gy * grid_w + gx;

            let objectness = score_sum[offset];
            let cls_score = cls_tensor[offset];
            // Runtime 的 want_float 输出若出现 NaN/Inf，丢弃该候选而不是把非法值带入 NMS/JSON。
            if !objectness.is_finite() || !cls_score.is_finite() {
                continue;
            }

            // 快速过滤：score_sum < threshold 直接跳过
            if objectness < conf_threshold {
                continue;
            }

            // 检测置信度
            if cls_score < conf_threshold {
                continue;
            }

            // DFL 解码 bbox
            let mut box_offset = [0.0f32; 4];
            for (b, offset_val) in box_offset.iter_mut().enumerate() {
                let base = b * DFL_LEN * grid_len + offset;
                let mut logits = [0.0f32; DFL_LEN];
                for (k, logit) in logits.iter_mut().enumerate() {
                    *logit = box_tensor[base + k * grid_len];
                }
                *offset_val = compute_dfl(&logits)?;
            }

            // 还原为原图像素坐标
            let x1 = (-box_offset[0] + gx as f32 + 0.5) * stride as f32;
            let y1 = (-box_offset[1] + gy as f32 + 0.5) * stride as f32;
            let x2 = (box_offset[2] + gx as f32 + 0.5) * stride as f32;
            let y2 = (box_offset[3] + gy as f32 + 0.5) * stride as f32;
            let w = (x2 - x1).max(0.0);
            let h = (y2 - y1).max(0.0);

            if w <= 0.0 || h <= 0.0 {
                continue;
            }

            // 解码 5 个关键点
            let mut landmarks = [[0.0f32; 2]; NUM_LANDMARKS];
            let mut landmark_scores = [0.0f32; NUM_LANDMARKS];
            for p in 0..NUM_LANDMARKS {
                let kx_idx = p * KPT_CHANNELS_PER_POINT * grid_len + offset;
                let ky_idx = (p * KPT_CHANNELS_PER_POINT + 1) * grid_len + offset;
                let kc_idx = (p * KPT_CHANNELS_PER_POINT + 2) * grid_len + offset;

                let raw_x = kpt_tensor[kx_idx];
                let raw_y = kpt_tensor[ky_idx];
                let raw_conf = kpt_tensor[kc_idx];
                if !raw_x.is_finite() || !raw_y.is_finite() || !raw_conf.is_finite() {
                    landmarks = [[f32::NAN; 2]; NUM_LANDMARKS];
                    break;
                }

                // 关键点坐标映射：(grid + offset * 2.0 - 0.5) * stride
                landmarks[p][0] = (gx as f32 + raw_x * 2.0 - 0.5) * stride as f32;
                landmarks[p][1] = (gy as f32 + raw_y * 2.0 - 0.5) * stride as f32;
                landmark_scores[p] = sigmoid(raw_conf);
            }

            if landmarks
                .iter()
                .any(|point| point.iter().any(|value| !value.is_finite()))
            {
                continue;
            }
            faces.push(RawFace {
                bbox: [x1, y1, w, h],
                landmarks,
                landmark_scores,
                score: cls_score,
            });
        }
    }

    Ok(faces)
}

/// 对人脸候选执行类别无关 NMS
pub fn nms(faces: &mut Vec<RawFace>, iou_threshold: f32) {
    if faces.len() <= 1 {
        return;
    }
    faces.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut kept = Vec::with_capacity(faces.len());
    for face in faces.iter().cloned() {
        if kept
            .iter()
            .all(|previous: &RawFace| previous.iou(&face) < iou_threshold)
        {
            kept.push(face);
        }
    }
    *faces = kept;
}

/// 将 bbox 和 landmarks 从模型输入画布像素坐标根据 Letterbox 布局反算并归一化到原图 [0, 1]
pub fn normalize_to_relative(faces: &mut [RawFace], layout: &LetterboxLayout) {
    let eff_w = layout.scaled_w as f32;
    let eff_h = layout.scaled_h as f32;
    if eff_w <= 0.0 || eff_h <= 0.0 {
        return;
    }
    let pad_left = layout.pad_left as f32;
    let pad_top = layout.pad_top as f32;

    for face in faces {
        // bbox 反算黑边并归一化到原图
        let x = (face.bbox[0] - pad_left) / eff_w;
        let y = (face.bbox[1] - pad_top) / eff_h;
        let w = face.bbox[2] / eff_w;
        let h = face.bbox[3] / eff_h;

        let mut norm_box = algo_sdk::math::NormBox::new(x, y, w, h, face.score, 0);
        clamp_bbox(&mut norm_box);
        face.bbox = [norm_box.x, norm_box.y, norm_box.w, norm_box.h];

        // landmarks 反算黑边并归一化到原图
        for point in &mut face.landmarks {
            point[0] = ((point[0] - pad_left) / eff_w).clamp(0.0, 1.0);
            point[1] = ((point[1] - pad_top) / eff_h).clamp(0.0, 1.0);
        }
    }
}

/// 三尺度合并解码 YOLOv8n-face 输出
///
/// `float_outputs`: 来自 RKNN 的 12 个 float32 张量切片
/// `output_attrs`: 张量形状属性（NCHW: dims[2]=H, dims[3]=W）
/// `layout`: 预处理 Letterbox 几何布局（用于坐标反算与归一化）
/// `conf_threshold`: 置信度阈值
/// `nms_threshold`: NMS IoU 阈值
pub fn decode_yolov8_face(
    float_outputs: &[&[f32]],
    output_attrs: &[[u32; 4]],
    layout: &LetterboxLayout,
    conf_threshold: f32,
    nms_threshold: f32,
) -> Result<Vec<RawFace>, AlgoError> {
    if !conf_threshold.is_finite()
        || !(0.0..=1.0).contains(&conf_threshold)
        || !nms_threshold.is_finite()
        || !(0.0..=1.0).contains(&nms_threshold)
    {
        return Err(AlgoError::Inference {
            reason: "YOLOv8n-face 阈值必须是 [0, 1] 范围内的有限数".to_string(),
        });
    }
    if float_outputs.len() != 12 || output_attrs.len() != 12 {
        return Err(AlgoError::Inference {
            reason: format!(
                "YOLOv8n-face 输出数量必须为 12: tensors={}, attrs={}",
                float_outputs.len(),
                output_attrs.len()
            ),
        });
    }

    let expected_channels = [64usize, 1, 1, 15];
    let mut all_faces = Vec::new();
    for scale in 0..3 {
        let base = scale * BRANCHES_PER_SCALE;
        let grid_h = usize::try_from(output_attrs[base][2]).map_err(|_| AlgoError::OutOfMemory)?;
        let grid_w = usize::try_from(output_attrs[base][3]).map_err(|_| AlgoError::OutOfMemory)?;
        if grid_h == 0 || grid_w == 0 {
            return Err(AlgoError::Inference {
                reason: format!("YOLOv8n-face 输出 {base} 网格尺寸为 0"),
            });
        }
        for branch in 0..BRANCHES_PER_SCALE {
            let attr = output_attrs[base + branch];
            if attr[0] != 1
                || attr[1] as usize != expected_channels[branch]
                || attr[2] as usize != grid_h
                || attr[3] as usize != grid_w
            {
                return Err(AlgoError::Inference {
                    reason: format!("YOLOv8n-face 输出 {} 形状非法: {:?}", base + branch, attr),
                });
            }
        }
        let stride = 8 * (1 << scale);
        let faces = decode_scale(
            float_outputs[base],
            float_outputs[base + 1],
            float_outputs[base + 2],
            float_outputs[base + 3],
            grid_h,
            grid_w,
            stride,
            conf_threshold,
        )?;
        all_faces.extend(faces);
    }

    nms(&mut all_faces, nms_threshold);
    normalize_to_relative(&mut all_faces, layout);
    Ok(all_faces)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sigmoid() {
        assert!((sigmoid(0.0) - 0.5).abs() < 1e-6);
        assert!((sigmoid(100.0) - 1.0).abs() < 1e-6);
        assert!((sigmoid(-100.0) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn test_compute_dfl_uniform() {
        // 均匀分布 → 加权平均 = 7.5
        let logits = [0.0; DFL_LEN];
        let result = compute_dfl(&logits).expect("均匀 logits 应可解码");
        assert!((result - 7.5).abs() < 1e-5);
    }

    #[test]
    fn test_compute_dfl_peaked() {
        // 峰值在 index 3 → 结果接近 3.0
        let mut logits = [-10.0; DFL_LEN];
        logits[3] = 10.0;
        let result = compute_dfl(&logits).expect("尖峰 logits 应可解码");
        assert!((result - 3.0).abs() < 0.01);
    }

    #[test]
    fn test_nms_removes_overlapping() {
        let mut faces = vec![
            RawFace {
                bbox: [0.1, 0.1, 0.3, 0.3],
                landmarks: [[0.0; 2]; 5],
                landmark_scores: [0.0; 5],
                score: 0.9,
            },
            RawFace {
                bbox: [0.12, 0.12, 0.3, 0.3],
                landmarks: [[0.0; 2]; 5],
                landmark_scores: [0.0; 5],
                score: 0.8,
            },
        ];
        nms(&mut faces, 0.5);
        assert_eq!(faces.len(), 1);
        assert!((faces[0].score - 0.9).abs() < 1e-6);
    }

    #[test]
    fn rejects_short_output_buffers_and_invalid_thresholds() {
        let layout = LetterboxLayout {
            scale: 1.0,
            pad_left: 0,
            pad_top: 0,
            dst_w: 640,
            dst_h: 384,
            scaled_w: 640,
            scaled_h: 384,
        };
        let attrs = [[1, 64, 48, 80]; 12];
        let outputs: Vec<&[f32]> = vec![&[]; 12];
        assert!(decode_yolov8_face(&outputs, &attrs, &layout, 0.25, 0.45).is_err());
        assert!(decode_yolov8_face(&outputs, &attrs, &layout, f32::NAN, 0.45).is_err());
    }
    #[test]
    fn test_normalize_to_relative_with_padding() {
        let layout = LetterboxLayout {
            scale: 0.33333334,
            pad_left: 0,
            pad_top: 12,
            dst_w: 640,
            dst_h: 384,
            scaled_w: 640,
            scaled_h: 360,
        };
        let mut faces = vec![RawFace {
            bbox: [320.0, 192.0, 133.33334, 133.33334],
            landmarks: [[320.0, 192.0]; 5],
            landmark_scores: [0.9; 5],
            score: 0.95,
        }];
        normalize_to_relative(&mut faces, &layout);
        let f = &faces[0];
        assert!((f.bbox[0] - 0.5).abs() < 1e-4);
        assert!((f.bbox[1] - 0.5).abs() < 1e-4);
        assert!((f.landmarks[0][0] - 0.5).abs() < 1e-4);
        assert!((f.landmarks[0][1] - 0.5).abs() < 1e-4);
    }
}
