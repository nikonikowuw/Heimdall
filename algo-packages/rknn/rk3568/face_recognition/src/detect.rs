//! YOLOv8n-face 12 张量解码器
//!
//! 输入：RKNN 模型输出的 12 个 float32 张量（3 尺度 × 4 分支：box, score_sum, cls, kpt）
//! 输出：`Vec<RawFace>`，bbox 和 landmarks 归一化到 [0, 1]

use algo_sdk::cv::LetterboxLayout;
use algo_sdk::error::AlgoError;

pub use crate::association::PersonCandidate;

/// 每个尺度的输出分支数
const BRANCHES_PER_SCALE: usize = 4;
/// box DFL 的 bin 数量（每条边 16 个分布）
const DFL_LEN: usize = 16;
/// 每个关键点的通道数 (x_offset, y_offset, visibility_conf)
const KPT_CHANNELS_PER_POINT: usize = 3;
/// 人脸关键点数量
pub const NUM_LANDMARKS: usize = 5;

/// YOLOv8n-face 单候选框
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawFace {
    /// `[x, y, width, height]` 坐标格式，左上角原点，归一化到 [0, 1]
    pub bbox: [f32; 4],
    /// 5 个关键点坐标，归一化到 [0, 1]
    pub landmarks: [[f32; 2]; NUM_LANDMARKS],
    /// 5 个关键点置信度（sigmoid 后）
    pub landmark_scores: [f32; NUM_LANDMARKS],
    /// 检测置信度
    pub score: f32,
}

impl RawFace {
    #[inline]
    pub fn width(&self) -> f32 {
        self.bbox[2].max(0.0)
    }

    #[inline]
    pub fn height(&self) -> f32 {
        self.bbox[3].max(0.0)
    }

    #[inline]
    pub fn area(&self) -> f32 {
        self.width() * self.height()
    }

    pub fn iou(&self, other: &Self) -> f32 {
        let ax2 = self.bbox[0] + self.bbox[2];
        let ay2 = self.bbox[1] + self.bbox[3];
        let bx2 = other.bbox[0] + other.bbox[2];
        let by2 = other.bbox[1] + other.bbox[3];
        let ix1 = self.bbox[0].max(other.bbox[0]);
        let iy1 = self.bbox[1].max(other.bbox[1]);
        let ix2 = ax2.min(bx2);
        let iy2 = ay2.min(by2);
        if ix2 <= ix1 || iy2 <= iy1 {
            return 0.0;
        }
        let iw = ix2 - ix1;
        let ih = iy2 - iy1;
        let intersection = iw * ih;
        let union = self.area() + other.area() - intersection;
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
    let mut exp_sum = 0.0f32;
    let mut weighted_sum = 0.0f32;
    for (i, &v) in logits.iter().enumerate() {
        let e = (v - max_val).exp();
        exp_sum += e;
        weighted_sum += e * i as f32;
    }
    if !exp_sum.is_finite() || exp_sum <= f32::EPSILON {
        return Err(AlgoError::Inference {
            reason: "DFL softmax 分母非法".to_string(),
        });
    }
    Ok(weighted_sum / exp_sum)
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

            // 快速过滤：低于阈值直接跳过
            if objectness < conf_threshold || cls_score < conf_threshold {
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

            // 还原为原图像素坐标 (x1, y1, x2, y2)
            let cx = gx as f32 + 0.5;
            let cy = gy as f32 + 0.5;
            let stride_f = stride as f32;
            let x1 = (cx - box_offset[0]) * stride_f;
            let y1 = (cy - box_offset[1]) * stride_f;
            let x2 = (cx + box_offset[2]) * stride_f;
            let y2 = (cy + box_offset[3]) * stride_f;

            if x2 <= x1 || y2 <= y1 {
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
                bbox: [x1, y1, x2 - x1, y2 - y1],
                landmarks,
                landmark_scores,
                score: cls_score,
            });
        }
    }

    Ok(faces)
}

/// 对人脸候选执行类别无关 NMS（零额外堆内存分配原地抑制）
pub fn nms(faces: &mut Vec<RawFace>, iou_threshold: f32) {
    if faces.len() <= 1 {
        return;
    }
    faces.sort_unstable_by(|a, b| b.score.total_cmp(&a.score));
    let mut kept_len = 0;
    for i in 0..faces.len() {
        let overlaps = (0..kept_len).any(|j| faces[j].iou(&faces[i]) >= iou_threshold);
        if !overlaps {
            faces.swap(kept_len, i);
            kept_len += 1;
        }
    }
    faces.truncate(kept_len);
}

pub const COCO_PERSON_CLASS_ID: usize = 0;

/// 解码标准 YOLOv8n 640x384 多张量 INT8 输出，算法与参考 C++ 后处理保持一致。
///
/// 契约固定为 3 尺度 × 3 分支 (9 个 NCHW INT8 张量)：
/// - 尺度 1 (stride 8):  grid 80x48, box 64ch, cls 80ch, score 1ch
/// - 尺度 2 (stride 16): grid 40x24, box 64ch, cls 80ch, score 1ch
/// - 尺度 3 (stride 32): grid 20x12, box 64ch, cls 80ch, score 1ch
///
/// 支持 NCHW 和 RKNN 的 NC1HWC2 物理通道布局；
/// 严格只提取 COCO class 0 (person) 进行反量化与 Sigmoid 激活。
pub fn decode_yolov8_person_multi_int8(
    int8_outputs: &[&[i8]],
    output_attrs: &[crate::rknn::RknnTensorAttr],
    layout: &LetterboxLayout,
    conf_threshold: f32,
    nms_threshold: f32,
) -> Result<Vec<PersonCandidate>, AlgoError> {
    let threshold = conf_threshold.clamp(0.0, 1.0);
    let mut candidates = Vec::new();
    let eff_w = layout.scaled_w as f32;
    let eff_h = layout.scaled_h as f32;
    if eff_w <= 0.0 || eff_h <= 0.0 {
        return Ok(candidates);
    }
    let inv_eff_w = 1.0 / eff_w;
    let inv_eff_h = 1.0 / eff_h;
    let pad_left = layout.pad_left as f32;
    let pad_top = layout.pad_top as f32;

    for (cls_idx, cls_attr) in output_attrs.iter().enumerate() {
        let cls_channels = cls_attr.dims[1];
        let is_cls = match cls_attr.fmt {
            crate::rknn::RknnTensorFormat::Nchw | crate::rknn::RknnTensorFormat::Undefined => {
                cls_channels == 80
            }
            crate::rknn::RknnTensorFormat::Nc1hwc2 => cls_channels * 16 >= 80,
            crate::rknn::RknnTensorFormat::Nhwc => false,
        };
        if !is_cls || cls_idx == 0 {
            continue;
        }
        let box_idx = cls_idx - 1;
        let box_attr = &output_attrs[box_idx];
        let is_box = match box_attr.fmt {
            crate::rknn::RknnTensorFormat::Nchw | crate::rknn::RknnTensorFormat::Undefined => {
                box_attr.dims[1] == 64
            }
            crate::rknn::RknnTensorFormat::Nc1hwc2 => box_attr.dims[1] * 16 >= 64,
            crate::rknn::RknnTensorFormat::Nhwc => false,
        };
        if !is_box || box_idx >= int8_outputs.len() || cls_idx >= int8_outputs.len() {
            continue;
        }

        let grid_h = usize::try_from(cls_attr.dims[2]).map_err(|_| AlgoError::OutOfMemory)?;
        let grid_w = usize::try_from(cls_attr.dims[3]).map_err(|_| AlgoError::OutOfMemory)?;
        let grid_len = grid_h.checked_mul(grid_w).ok_or(AlgoError::OutOfMemory)?;
        let (stride_x, stride_y) = match (grid_w, grid_h) {
            (80, 48) => (8.0, 8.0),
            (40, 24) => (16.0, 16.0),
            (20, 12) => (32.0, 32.0),
            _ => continue,
        };
        let cls_data = int8_outputs[cls_idx];
        let box_data = int8_outputs[box_idx];

        for gy in 0..grid_h {
            for gx in 0..grid_w {
                let offset = gy * grid_w + gx;
                let Some(person_value) = tensor_i8_value(
                    cls_data,
                    cls_attr.fmt,
                    COCO_PERSON_CLASS_ID,
                    grid_h,
                    grid_w,
                    gy,
                    gx,
                ) else {
                    continue;
                };
                let cls_float = dequant_i8(person_value, cls_attr.zp, cls_attr.scale);
                let score = activate_yolov8_score(cls_float);
                if !score.is_finite() || score < threshold {
                    continue;
                }

                let left = decode_dfl_i8(box_data, box_attr, 0, offset, grid_len, grid_h, grid_w);
                let top = decode_dfl_i8(box_data, box_attr, 1, offset, grid_len, grid_h, grid_w);
                let right = decode_dfl_i8(box_data, box_attr, 2, offset, grid_len, grid_h, grid_w);
                let bottom = decode_dfl_i8(box_data, box_attr, 3, offset, grid_len, grid_h, grid_w);
                let cx = (gx as f32 + 0.5) * stride_x;
                let cy = (gy as f32 + 0.5) * stride_y;
                let x1_px = cx - left * stride_x;
                let y1_px = cy - top * stride_y;
                let x2_px = cx + right * stride_x;
                let y2_px = cy + bottom * stride_y;

                let x1 = ((x1_px - pad_left) * inv_eff_w).clamp(0.0, 1.0);
                let y1 = ((y1_px - pad_top) * inv_eff_h).clamp(0.0, 1.0);
                let x2 = ((x2_px - pad_left) * inv_eff_w).clamp(0.0, 1.0);
                let y2 = ((y2_px - pad_top) * inv_eff_h).clamp(0.0, 1.0);
                if x2 > x1 && y2 > y1 {
                    candidates.push(PersonCandidate {
                        bbox: [x1, y1, x2 - x1, y2 - y1],
                        score,
                    });
                }
            }
        }
    }

    nms_persons(&mut candidates, nms_threshold);
    Ok(candidates)
}

fn tensor_i8_value(
    data: &[i8],
    format: crate::rknn::RknnTensorFormat,
    channel: usize,
    grid_h: usize,
    grid_w: usize,
    y: usize,
    x: usize,
) -> Option<i8> {
    let index = match format {
        crate::rknn::RknnTensorFormat::Nchw | crate::rknn::RknnTensorFormat::Undefined => channel
            .checked_mul(grid_h.checked_mul(grid_w)?)?
            .checked_add(y.checked_mul(grid_w)?.checked_add(x)?)?,
        crate::rknn::RknnTensorFormat::Nc1hwc2 => {
            let channel_group = channel / 16;
            let channel_in_group = channel % 16;
            channel_group
                .checked_mul(grid_h.checked_mul(grid_w)?.checked_mul(16)?)?
                .checked_add(y.checked_mul(grid_w)?.checked_mul(16)?)?
                .checked_add(x.checked_mul(16)?)?
                .checked_add(channel_in_group)?
        }
        _ => return None,
    };
    data.get(index).copied()
}

fn dequant_i8(value: i8, zero_point: i32, scale: f32) -> f32 {
    (value as f32 - zero_point as f32) * scale
}

fn activate_yolov8_score(value: f32) -> f32 {
    if value > 1.0 || value < -0.1 {
        1.0 / (1.0 + (-value).exp())
    } else {
        value
    }
}

fn decode_dfl_i8(
    data: &[i8],
    attr: &crate::rknn::RknnTensorAttr,
    side: usize,
    offset: usize,
    grid_len: usize,
    grid_h: usize,
    grid_w: usize,
) -> f32 {
    debug_assert!(offset < grid_len);
    let gy = offset / grid_w;
    let gx = offset % grid_w;
    let base_channel = side * DFL_LEN;

    let mut logits = [0.0f32; DFL_LEN];
    let mut max_logit = f32::NEG_INFINITY;
    for (bin, logit) in logits.iter_mut().enumerate() {
        let channel = base_channel + bin;
        let Some(value) = tensor_i8_value(data, attr.fmt, channel, grid_h, grid_w, gy, gx) else {
            return 0.0;
        };
        *logit = dequant_i8(value, attr.zp, attr.scale);
        max_logit = max_logit.max(*logit);
    }
    let mut exp_sum = 0.0f32;
    let mut weighted_sum = 0.0f32;
    for (bin, logit) in logits.into_iter().enumerate() {
        let exp_value = (logit - max_logit).exp();
        exp_sum += exp_value;
        weighted_sum += exp_value * bin as f32;
    }
    if exp_sum > 0.0 {
        weighted_sum / exp_sum
    } else {
        0.0
    }
}
/// 人体检测候选 NMS 抑制
pub fn nms_persons(persons: &mut Vec<PersonCandidate>, iou_threshold: f32) {
    if persons.len() <= 1 {
        return;
    }
    persons.sort_unstable_by(|a, b| b.score.total_cmp(&a.score));
    let mut kept_len = 0;
    for i in 0..persons.len() {
        let overlaps = (0..kept_len).any(|j| {
            crate::bytetrack::box_iou(&persons[j].bbox, &persons[i].bbox) >= iou_threshold
        });
        if !overlaps {
            persons.swap(kept_len, i);
            kept_len += 1;
        }
    }
    persons.truncate(kept_len);
}

/// 将 bbox 和 landmarks 从模型输入画布像素坐标根据 Letterbox 布局反算并归一化到原图 [0, 1]
/// 全系统统一遵循 [x, y, w, h] 规范
pub fn normalize_to_relative(faces: &mut [RawFace], layout: &LetterboxLayout) {
    let eff_w = layout.scaled_w as f32;
    let eff_h = layout.scaled_h as f32;
    if eff_w <= 0.0 || eff_h <= 0.0 {
        return;
    }
    let inv_w = 1.0 / eff_w;
    let inv_h = 1.0 / eff_h;
    let pad_left = layout.pad_left as f32;
    let pad_top = layout.pad_top as f32;

    for face in faces {
        // bbox 反算黑边并归一化到原图 [0, 1] (x, y, w, h)
        let x1 = ((face.bbox[0] - pad_left) * inv_w).clamp(0.0, 1.0);
        let y1 = ((face.bbox[1] - pad_top) * inv_h).clamp(0.0, 1.0);
        let x2 = ((face.bbox[0] + face.bbox[2] - pad_left) * inv_w).clamp(0.0, 1.0);
        let y2 = ((face.bbox[1] + face.bbox[3] - pad_top) * inv_h).clamp(0.0, 1.0);

        face.bbox = [x1, y1, (x2 - x1).max(0.0), (y2 - y1).max(0.0)];

        // landmarks 反算黑边并归一化到原图
        for point in &mut face.landmarks {
            point[0] = ((point[0] - pad_left) * inv_w).clamp(0.0, 1.0);
            point[1] = ((point[1] - pad_top) * inv_h).clamp(0.0, 1.0);
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
                bbox: [0.1, 0.1, 0.4, 0.4],
                landmarks: [[0.0; 2]; 5],
                landmark_scores: [0.0; 5],
                score: 0.9,
            },
            RawFace {
                bbox: [0.12, 0.12, 0.42, 0.42],
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
    fn test_decode_yolov8_person_multi_int8_non_square_three_scales() {
        let grid_h = 48usize;
        let grid_w = 80usize;
        let grid_len = grid_h * grid_w;
        let anchor = 10 * grid_w + 10;
        let mut output_storage = [
            vec![-10i8; 64 * grid_len],
            vec![0i8; 80 * grid_len],
            vec![0i8; grid_len],
            vec![-10i8; 64 * (24 * 40)],
            vec![0i8; 80 * (24 * 40)],
            vec![0i8; 24 * 40],
            vec![-10i8; 64 * (12 * 20)],
            vec![0i8; 80 * (12 * 20)],
            vec![0i8; 12 * 20],
        ];
        for (side, bin) in [1usize, 2, 3, 4].into_iter().enumerate() {
            output_storage[0][(side * DFL_LEN + bin) * grid_len + anchor] = 10;
        }
        output_storage[1][anchor] = 10;

        let make_attr =
            |index: u32, channels: u32, height: u32, width: u32| crate::rknn::RknnTensorAttr {
                index,
                n_dims: 4,
                dims: [
                    1, channels, height, width, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                ],
                n_elems: channels * height * width,
                fmt: crate::rknn::RknnTensorFormat::Nchw,
                qnt_type: crate::rknn::RknnTensorQntType::AsymmetricChar,
                scale: 1.0,
                ..Default::default()
            };
        let attrs = [
            make_attr(0, 64, 48, 80),
            make_attr(1, 80, 48, 80),
            make_attr(2, 1, 48, 80),
            make_attr(3, 64, 24, 40),
            make_attr(4, 80, 24, 40),
            make_attr(5, 1, 24, 40),
            make_attr(6, 64, 12, 20),
            make_attr(7, 80, 12, 20),
            make_attr(8, 1, 12, 20),
        ];
        let outputs: Vec<&[i8]> = output_storage.iter().map(Vec::as_slice).collect();
        let layout = LetterboxLayout {
            scale: 1.0,
            pad_left: 0,
            pad_top: 0,
            dst_w: 640,
            dst_h: 384,
            scaled_w: 640,
            scaled_h: 384,
        };

        let persons = decode_yolov8_person_multi_int8(&outputs, &attrs, &layout, 0.5, 0.45)
            .expect("非方形三尺度 INT8 YOLOv8 解码应成功");
        assert_eq!(persons.len(), 1);
        assert!(persons[0].score > 0.99);
        assert!((persons[0].bbox[0] - 76.0 / 640.0).abs() < 0.01);
        assert!((persons[0].bbox[1] - 68.0 / 384.0).abs() < 0.01);
        assert!((persons[0].bbox[2] - 32.0 / 640.0).abs() < 0.01);
        assert!((persons[0].bbox[3] - 48.0 / 384.0).abs() < 0.01);
        let mut non_person_class_data = vec![0i8; 80 * grid_len];
        non_person_class_data[grid_len + anchor] = 10;
        let non_person_outputs = [
            &output_storage[0][..],
            &non_person_class_data[..],
            &output_storage[2][..],
            &output_storage[3][..],
            &output_storage[4][..],
            &output_storage[5][..],
            &output_storage[6][..],
            &output_storage[7][..],
            &output_storage[8][..],
        ];
        let non_persons =
            decode_yolov8_person_multi_int8(&non_person_outputs, &attrs, &layout, 0.5, 0.45)
                .expect("非 person 类别不应导致人体候选");
        assert!(non_persons.is_empty());
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
        assert!((f.bbox[2] - 0.20833).abs() < 1e-4);
        assert!((f.bbox[3] - 0.37037).abs() < 1e-4);
        assert!((f.landmarks[0][0] - 0.5).abs() < 1e-4);
        assert!((f.landmarks[0][1] - 0.5).abs() < 1e-4);
    }
}
