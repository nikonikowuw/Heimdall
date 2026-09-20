//! 检测后处理：SCRFD 人脸解码与 YOLOv8n 人体解码
//!
//! - `decode_scrfd_face`：9 个 float32 张量（3 尺度 × score/bbox/kps）→ `Vec<RawFace>`；
//! - `decode_yolov8_person_multi_int8`：9 个 INT8 张量（3 尺度 × box/cls/score），
//!   仅取 COCO class 0 → `Vec<PersonCandidate>`。
//!
//! bbox 与 landmarks 统一归一化到 [0, 1]。

use std::sync::atomic::{AtomicBool, Ordering};

use algo_sdk::cv::LetterboxLayout;
use algo_sdk::error::AlgoError;

pub use crate::association::PersonCandidate;

/// box DFL 的 bin 数量（每条边 16 个分布）
const DFL_LEN: usize = 16;
/// 人脸关键点数量
pub const NUM_LANDMARKS: usize = 5;
/// 判定 cls 分支为「概率输出」时的取值窗口，必须开得远宽于 [0, 1]。
///
/// 两个方向的误判代价**不对称**，窗口要为最坏情况留余量：
/// - 概率被误判为 logits：会对概率再做一次 sigmoid，背景格 0 被抬到 0.5，
///   在 0.4 的人物门限下就是全屏假阳性，且现场难以归因；
/// - logits 被误判为概率：背景负值仍被门控过滤，仅强检出的置信度偏高，可逆。
///
/// 因此窗口取 ±2.0：概率（含校准外扩）不可能接近 2.0，而真实人像的 logits 通常远超 2，
/// 背景 logits 远低于 -2。
const PROBABILITY_MIN_SCORE: f32 = -2.0;
const PROBABILITY_MAX_SCORE: f32 = 2.0;
/// 激活模式只上报一次：现场需要知道上报分数是概率还是 sigmoid(logit)。
static CLS_MODE_LOGGED: AtomicBool = AtomicBool::new(false);

/// 人脸单候选框
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawFace {
    /// `[x, y, width, height]` 坐标格式，左上角原点，归一化到 [0, 1]
    pub bbox: [f32; 4],
    /// 5 个关键点坐标，归一化到 [0, 1]
    pub landmarks: [[f32; 2]; NUM_LANDMARKS],
    /// 关键点位置的置信度代理量。
    ///
    /// 交付的 SCRFD 模型只输出 10 通道关键点偏移、没有关键点分数分支，因此解码时
    /// 填的是**检测置信度**（见 [`decode_scrfd_face`]）。后果：下游质量模块的
    /// `blur = 1 - 关键点置信度均值` 实际退化为 `1 - 检测置信度`，`quality_max_blur`
    /// 等价于一条检测分下限（与 `min_score` 权重叠加，检测分在综合分里被计两次）。
    /// 若后续改用真实清晰度度量（如图像梯度），需同步重新标定现场阈值。
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
        crate::bytetrack::box_iou(&self.bbox, &other.bbox)
    }
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// 通用原地 NMS 贪婪抑制算法
#[inline]
fn run_nms<T>(
    items: &mut Vec<T>,
    iou_threshold: f32,
    score_fn: impl FnMut(&T) -> f32,
    iou_fn: impl FnMut(&T, &T) -> f32,
) {
    algo_sdk::math::run_nms_by(items, iou_threshold, score_fn, iou_fn);
}

/// 对人脸候选执行类别无关 NMS（零额外堆内存分配原地抑制）
pub fn nms(faces: &mut Vec<RawFace>, iou_threshold: f32) {
    run_nms(faces, iou_threshold, |f| f.score, |a, b| a.iou(b));
}

/// 人体检测候选 NMS 抑制
pub fn nms_persons(persons: &mut Vec<PersonCandidate>, iou_threshold: f32) {
    run_nms(
        persons,
        iou_threshold,
        |p| p.score,
        |a, b| crate::bytetrack::box_iou(&a.bbox, &b.bbox),
    );
}

pub const COCO_PERSON_CLASS_ID: usize = 0;

fn matches_channel_count(attr: &crate::rknn::RknnTensorAttr, expected: u32) -> bool {
    match attr.fmt {
        crate::rknn::RknnTensorFormat::Nchw | crate::rknn::RknnTensorFormat::Undefined => {
            attr.dims[1] == expected
        }
        crate::rknn::RknnTensorFormat::Nc1hwc2 => attr.dims[1] * 16 >= expected,
        crate::rknn::RknnTensorFormat::Nhwc => false,
    }
}

/// 解码标准 YOLOv8n 640x384 多张量 INT8 输出，算法与参考 C++ 后处理保持一致。
///
/// 契约固定为 3 尺度 × 3 分支 (9 个 NCHW INT8 张量)：
/// - 尺度 1 (stride 8):  grid 80x48, box 64ch, cls 80ch, score 1ch
/// - 尺度 2 (stride 16): grid 40x24, box 64ch, cls 80ch, score 1ch
/// - 尺度 3 (stride 32): grid 20x12, box 64ch, cls 80ch, score 1ch
///
/// 支持 NCHW 和 RKNN 的 NC1HWC2 物理通道布局；
/// 严格只提取 COCO class 0 (person) 进行反量化与 Sigmoid 激活。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PersonBoxDecoderKind {
    Yolov8Dfl,
    Yolov6Direct,
}

fn decode_person_multi_int8_impl(
    int8_outputs: &[&[i8]],
    output_attrs: &[crate::rknn::RknnTensorAttr],
    layout: &LetterboxLayout,
    conf_threshold: f32,
    nms_threshold: f32,
    kind: PersonBoxDecoderKind,
) -> Result<Vec<PersonCandidate>, AlgoError> {
    let expected_box_channels = match kind {
        PersonBoxDecoderKind::Yolov8Dfl => 64,
        PersonBoxDecoderKind::Yolov6Direct => 4,
    };
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
        if cls_idx == 0 || !matches_channel_count(cls_attr, 80) {
            continue;
        }
        let box_idx = cls_idx - 1;
        let box_attr = &output_attrs[box_idx];
        if !matches_channel_count(box_attr, expected_box_channels)
            || box_idx >= int8_outputs.len()
            || cls_idx >= int8_outputs.len()
        {
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
        // 激活模式按整张量判定一次：逐格猜会让同一张量内的分数在「直通」与「sigmoid」之间
        // 跳变（logit 1.0 直通为 1.0，logit 1.2 却被压成 0.77），同帧分数失去单调性，
        // NMS 排序与人工阈值都不可信。
        let cls_probability_mode = cls_scores_are_probabilities(cls_data, cls_attr, grid_h, grid_w);
        if !CLS_MODE_LOGGED.swap(true, Ordering::Relaxed) {
            let model_tag = match kind {
                PersonBoxDecoderKind::Yolov8Dfl => "YOLOv8n",
                PersonBoxDecoderKind::Yolov6Direct => "YOLOv6n",
            };
            tracing::info!(
                probability_mode = cls_probability_mode,
                grid_w,
                grid_h,
                "{model_tag} 人体检测 cls 分支激活模式判定"
            );
        }

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
                let score = activate_person_score(cls_float, cls_probability_mode);
                if !score.is_finite() || score < threshold {
                    continue;
                }

                let (left, top, right, bottom) = match kind {
                    PersonBoxDecoderKind::Yolov8Dfl => {
                        let l =
                            decode_dfl_i8(box_data, box_attr, 0, offset, grid_len, grid_h, grid_w);
                        let t =
                            decode_dfl_i8(box_data, box_attr, 1, offset, grid_len, grid_h, grid_w);
                        let r =
                            decode_dfl_i8(box_data, box_attr, 2, offset, grid_len, grid_h, grid_w);
                        let b =
                            decode_dfl_i8(box_data, box_attr, 3, offset, grid_len, grid_h, grid_w);
                        (l, t, r, b)
                    }
                    PersonBoxDecoderKind::Yolov6Direct => {
                        let (Some(l_raw), Some(t_raw), Some(r_raw), Some(b_raw)) = (
                            tensor_i8_value(box_data, box_attr.fmt, 0, grid_h, grid_w, gy, gx),
                            tensor_i8_value(box_data, box_attr.fmt, 1, grid_h, grid_w, gy, gx),
                            tensor_i8_value(box_data, box_attr.fmt, 2, grid_h, grid_w, gy, gx),
                            tensor_i8_value(box_data, box_attr.fmt, 3, grid_h, grid_w, gy, gx),
                        ) else {
                            continue;
                        };
                        (
                            dequant_i8(l_raw, box_attr.zp, box_attr.scale),
                            dequant_i8(t_raw, box_attr.zp, box_attr.scale),
                            dequant_i8(r_raw, box_attr.zp, box_attr.scale),
                            dequant_i8(b_raw, box_attr.zp, box_attr.scale),
                        )
                    }
                };

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
    decode_person_multi_int8_impl(
        int8_outputs,
        output_attrs,
        layout,
        conf_threshold,
        nms_threshold,
        PersonBoxDecoderKind::Yolov8Dfl,
    )
}

/// 解码标准 YOLOv6n 640x384 (或 384x640) 多张量 INT8 输出。
///
/// 契约固定为 3 尺度 × 3 分支 (9 个 NCHW INT8 张量)：
/// - 尺度 1 (stride 8):  grid 80x48, box 4ch (left, top, right, bottom), cls 80ch, score 1ch
/// - 尺度 2 (stride 16): grid 40x24, box 4ch, cls 80ch, score 1ch
/// - 尺度 3 (stride 32): grid 20x12, box 4ch, cls 80ch, score 1ch
///
/// 支持 NCHW 和 RKNN 的 NC1HWC2 物理通道布局；
/// 严格只提取 COCO class 0 (person) 进行反量化与 Sigmoid 激活。
pub fn decode_yolov6_person_multi_int8(
    int8_outputs: &[&[i8]],
    output_attrs: &[crate::rknn::RknnTensorAttr],
    layout: &LetterboxLayout,
    conf_threshold: f32,
    nms_threshold: f32,
) -> Result<Vec<PersonCandidate>, AlgoError> {
    decode_person_multi_int8_impl(
        int8_outputs,
        output_attrs,
        layout,
        conf_threshold,
        nms_threshold,
        PersonBoxDecoderKind::Yolov6Direct,
    )
}

/// 统一人体检测后处理入口：自动按输出张量通道数分发至 YOLOv6 或 YOLOv8 解码器。
pub fn decode_person_multi_int8(
    int8_outputs: &[&[i8]],
    output_attrs: &[crate::rknn::RknnTensorAttr],
    layout: &LetterboxLayout,
    conf_threshold: f32,
    nms_threshold: f32,
) -> Result<Vec<PersonCandidate>, AlgoError> {
    let is_yolov6 = output_attrs
        .first()
        .map(|a| matches_channel_count(a, 4))
        .unwrap_or(false);
    if is_yolov6 {
        decode_yolov6_person_multi_int8(
            int8_outputs,
            output_attrs,
            layout,
            conf_threshold,
            nms_threshold,
        )
    } else {
        decode_yolov8_person_multi_int8(
            int8_outputs,
            output_attrs,
            layout,
            conf_threshold,
            nms_threshold,
        )
    }
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

/// 判定 cls 分支的输出是否已经是概率而非 raw logits。
///
/// 依据是模型量化后的实际取值：概率输出被校准时落在 [0, 1] 内；raw logits 无界，
/// 且 BCE 训练下背景格的 logit 是强负值。两个方向都必须查：
/// - 只看上界：背景 logit 强负的张量在满幅人像场景下其最大值可能仍 ≤ 1，
///   会被误判为概率而整张直通；
/// - 只看下界：校准区间略宽于 [0, 1] 的概率张量会被误判为 logits，
///   等于给概率又做一次 sigmoid（背景 0 会被抬到 0.5，直接制造假阳性）。
///
/// 窗口宽度按「误判代价不对称」定，见 `PROBABILITY_MIN_SCORE`。
fn cls_scores_are_probabilities(
    cls_data: &[i8],
    cls_attr: &crate::rknn::RknnTensorAttr,
    grid_h: usize,
    grid_w: usize,
) -> bool {
    for gy in 0..grid_h {
        for gx in 0..grid_w {
            let Some(value) = tensor_i8_value(
                cls_data,
                cls_attr.fmt,
                COCO_PERSON_CLASS_ID,
                grid_h,
                grid_w,
                gy,
                gx,
            ) else {
                return false;
            };
            let score = dequant_i8(value, cls_attr.zp, cls_attr.scale);
            if !(PROBABILITY_MIN_SCORE..=PROBABILITY_MAX_SCORE).contains(&score) {
                return false;
            }
        }
    }
    true
}

fn activate_person_score(value: f32, probability_mode: bool) -> f32 {
    if probability_mode {
        value.clamp(0.0, 1.0)
    } else {
        sigmoid(value)
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
    let (exp_sum, weighted_sum) =
        logits
            .into_iter()
            .enumerate()
            .fold((0.0f32, 0.0f32), |(e_acc, w_acc), (bin, logit)| {
                let exp = (logit - max_logit).exp();
                (e_acc + exp, w_acc + exp * bin as f32)
            });
    if exp_sum > 0.0 {
        weighted_sum / exp_sum
    } else {
        0.0
    }
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

        // landmarks 反算黑边并归一化到原图相对坐标系（允许轻微跨界以保护 5 点几何拓扑不失真）
        for point in &mut face.landmarks {
            point[0] = ((point[0] - pad_left) * inv_w).clamp(-0.2, 1.2);
            point[1] = ((point[1] - pad_top) * inv_h).clamp(-0.2, 1.2);
        }
    }
}

/// 三尺度合并解码 SCRFD 人脸检测模型（如 SCRFD-2.5G BNKPS）输出
///
/// 契约说明：
/// `float_outputs`: 9 个 float32 张量切片：
/// - 0..3: 3 个尺度的 score: score_8, score_16, score_32 (已由模型 Sigmoid 激活)
/// - 3..6: 3 个尺度的 bbox: bbox_8, bbox_16, bbox_32 (每个 anchor 4 通道: l, t, r, b 偏移)
/// - 6..9: 3 个尺度的 kps: kps_8, kps_16, kps_32 (每个 anchor 10 通道: 5 点 x, y 偏移)
///
/// 每个 grid cell 预设 2 个 anchors。
pub fn decode_scrfd_face(
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
            reason: "SCRFD 阈值必须是 [0, 1] 范围内的有限数".to_string(),
        });
    }
    if float_outputs.len() != 9 || output_attrs.len() != 9 {
        return Err(AlgoError::Inference {
            reason: format!(
                "SCRFD 输出数量必须为 9: tensors={}, attrs={}",
                float_outputs.len(),
                output_attrs.len()
            ),
        });
    }

    let strides = [8u32, 16, 32];
    let num_anchors = 2usize;
    let mut all_faces = Vec::new();

    for (scale_idx, &stride) in strides.iter().enumerate() {
        let grid_w = usize::try_from(layout.dst_w / stride).map_err(|_| AlgoError::OutOfMemory)?;
        let grid_h = usize::try_from(layout.dst_h / stride).map_err(|_| AlgoError::OutOfMemory)?;
        if grid_w == 0 || grid_h == 0 {
            return Err(AlgoError::Inference {
                reason: format!("SCRFD 尺度 {scale_idx} (stride={stride}) 网格尺寸为 0"),
            });
        }
        let total_anchors = grid_h * grid_w * num_anchors;

        let score_slice = float_outputs[scale_idx];
        let bbox_slice = float_outputs[scale_idx + 3];
        let kps_slice = float_outputs[scale_idx + 6];

        if score_slice.len() < total_anchors
            || bbox_slice.len() < total_anchors * 4
            || kps_slice.len() < total_anchors * 10
        {
            return Err(AlgoError::Inference {
                reason: format!(
                    "SCRFD 尺度 {scale_idx} 张量长度不足: score={}, bbox={}, kps={}, expected_anchors={}",
                    score_slice.len(),
                    bbox_slice.len(),
                    kps_slice.len(),
                    total_anchors
                ),
            });
        }

        let stride_f32 = stride as f32;

        for gy in 0..grid_h {
            let center_y = (gy as f32) * stride_f32;
            for gx in 0..grid_w {
                let center_x = (gx as f32) * stride_f32;
                for a in 0..num_anchors {
                    let anchor_idx = (gy * grid_w + gx) * num_anchors + a;
                    let score = score_slice[anchor_idx];
                    if !score.is_finite() || score < conf_threshold {
                        continue;
                    }

                    let b_offset = anchor_idx * 4;
                    let dx1 = bbox_slice[b_offset] * stride_f32;
                    let dy1 = bbox_slice[b_offset + 1] * stride_f32;
                    let dx2 = bbox_slice[b_offset + 2] * stride_f32;
                    let dy2 = bbox_slice[b_offset + 3] * stride_f32;

                    let x1 = center_x - dx1;
                    let y1 = center_y - dy1;
                    let x2 = center_x + dx2;
                    let y2 = center_y + dy2;

                    let w = x2 - x1;
                    let h = y2 - y1;
                    if w <= 0.0 || h <= 0.0 {
                        continue;
                    }

                    let k_offset = anchor_idx * 10;
                    let mut landmarks = [[0.0f32; 2]; NUM_LANDMARKS];
                    for (p, point) in landmarks.iter_mut().enumerate() {
                        let px = center_x + kps_slice[k_offset + p * 2] * stride_f32;
                        let py = center_y + kps_slice[k_offset + p * 2 + 1] * stride_f32;
                        *point = [px, py];
                    }

                    all_faces.push(RawFace {
                        bbox: [x1, y1, w, h],
                        landmarks,
                        // SCRFD 无关键点分数分支：用检测置信度作代理，语义见字段文档。
                        landmark_scores: [score; NUM_LANDMARKS],
                        score,
                    });
                }
            }
        }
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

    /// cls 分支的激活模式必须按整张量统一判定。
    ///
    /// 旧实现对每个 grid 单元按数值猜激活函数（值落在 [-0.1, 1.0] 就直通），同一张量内
    /// 会出现 logit 1.0 上报 1.0、logit 1.2 上报 0.77 的分数倒挂，NMS 排序随之翻转。
    #[test]
    fn person_cls_activation_is_monotonic_per_tensor() {
        // 概率张量：直通并钳位到 [0, 1]
        assert!((activate_person_score(0.42, true) - 0.42).abs() < 1e-6);
        assert!((activate_person_score(1.4, true) - 1.0).abs() < 1e-6);
        // logits 张量：sigmoid 激活且严格单调
        assert!((activate_person_score(0.0, false) - 0.5).abs() < 1e-6);
        assert!(activate_person_score(1.2, false) > activate_person_score(1.0, false));
        assert!(activate_person_score(1.0, false) < 1.0);
    }

    /// 激活模式判定只看量化上界：概率张量不得因校准区间略宽而被二次激活。
    #[test]
    fn cls_activation_mode_follows_quantized_value_range() {
        let make_attr = |scale: f32, zp: i32| crate::rknn::RknnTensorAttr {
            index: 1,
            n_dims: 4,
            dims: [1, 80, 1, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            n_elems: 160,
            fmt: crate::rknn::RknnTensorFormat::Nchw,
            qnt_type: crate::rknn::RknnTensorQntType::AsymmetricChar,
            scale,
            zp,
            ..Default::default()
        };
        // 概率输出：反量化区间恰好钳在 [0, 1]
        let probability = make_attr(1.0 / 255.0, -128);
        let saturated = vec![i8::MAX; 160];
        assert!(cls_scores_are_probabilities(&saturated, &probability, 1, 2));
        // 概率输出：背景为 0 也不得判成 logits（否则 sigmoid(0)=0.5 制造假阳性）
        let mut probability_background = vec![0i8; 160];
        probability_background[0] = i8::MAX;
        assert!(cls_scores_are_probabilities(
            &probability_background,
            &probability,
            1,
            2
        ));
        // raw logits：取值远高于概率窗口
        let logits = make_attr(0.05, -20);
        assert!(!cls_scores_are_probabilities(&saturated, &logits, 1, 2));
        // raw logits：最大值仍落在概率窗口内，靠强负背景识别（满幅人像场景）
        let narrow_logits = make_attr(0.02, 0);
        let mut logits_background = vec![i8::MIN; 160];
        // 75 * 0.02 = 1.5，落在概率窗口内
        logits_background[0] = 75;
        assert!(!cls_scores_are_probabilities(
            &logits_background,
            &narrow_logits,
            1,
            2
        ));
    }

    /// NC1HWC2 物理通道布局的索引契约（组内通道数固定 16）。
    ///
    /// 该分支在 RKNN 报告 NC1HWC2 时启用（当前交付模型是 NCHW，属保底路径）：
    /// `group = channel / 16`、组内偏移 `= channel % 16`、`(y, x)` 通道步长为 16。
    /// 越界必须返回 `None`，不得回绕到同组其它通道。
    #[test]
    fn nc1hwc2_channel_indexing_matches_rknn_layout() {
        let (grid_h, grid_w, channels) = (2usize, 3usize, 32usize);
        let mut data = vec![0i8; channels * grid_h * grid_w];
        let index = |channel: usize, y: usize, x: usize| {
            (channel / 16) * grid_h * grid_w * 16 + y * grid_w * 16 + x * 16 + channel % 16
        };
        data[index(17, 1, 2)] = 42;
        assert_eq!(
            tensor_i8_value(
                &data,
                crate::rknn::RknnTensorFormat::Nc1hwc2,
                17,
                grid_h,
                grid_w,
                1,
                2
            ),
            Some(42)
        );
        assert_eq!(
            tensor_i8_value(
                &data,
                crate::rknn::RknnTensorFormat::Nc1hwc2,
                channels,
                grid_h,
                grid_w,
                1,
                2
            ),
            None
        );
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
        let scrfd_attrs = [[7680, 1, 1, 1]; 9];
        let scrfd_outputs: Vec<&[f32]> = vec![&[]; 9];
        assert!(decode_scrfd_face(&scrfd_outputs, &scrfd_attrs, &layout, 0.25, 0.45).is_err());
        assert!(decode_scrfd_face(&scrfd_outputs, &scrfd_attrs, &layout, f32::NAN, 0.45).is_err());
    }

    #[test]
    fn test_decode_scrfd_face_synthetic() {
        let layout = LetterboxLayout {
            scale: 1.0,
            pad_left: 0,
            pad_top: 0,
            dst_w: 640,
            dst_h: 384,
            scaled_w: 640,
            scaled_h: 384,
        };
        let attrs = crate::manifest::DETECTOR_OUTPUT_SHAPES;
        let mut score_8 = vec![0.0f32; 7680];
        let score_16 = vec![0.0f32; 1920];
        let score_32 = vec![0.0f32; 480];
        let mut bbox_8 = vec![0.0f32; 7680 * 4];
        let bbox_16 = vec![0.0f32; 1920 * 4];
        let bbox_32 = vec![0.0f32; 480 * 4];
        let mut kps_8 = vec![0.0f32; 7680 * 10];
        let kps_16 = vec![0.0f32; 1920 * 10];
        let kps_32 = vec![0.0f32; 480 * 10];

        // 在 (gx=10, gy=10), anchor=0 构造一个人脸候选
        // stride = 8, center_x = 80.0, center_y = 80.0
        let anchor_idx = (10 * 80 + 10) * 2;
        score_8[anchor_idx] = 0.95;
        // dx1 = 2.0 (16px), dy1 = 2.0 (16px), dx2 = 3.0 (24px), dy2 = 3.0 (24px)
        // x1 = 64.0, y1 = 64.0, x2 = 104.0, y2 = 104.0, w = 40.0, h = 40.0
        bbox_8[anchor_idx * 4] = 2.0;
        bbox_8[anchor_idx * 4 + 1] = 2.0;
        bbox_8[anchor_idx * 4 + 2] = 3.0;
        bbox_8[anchor_idx * 4 + 3] = 3.0;

        // kps: 5 点相对 center 的 offset
        for p in 0..5 {
            kps_8[anchor_idx * 10 + p * 2] = (p as f32) * 0.5;
            kps_8[anchor_idx * 10 + p * 2 + 1] = (p as f32) * 0.5;
        }

        let outputs: [&[f32]; 9] = [
            &score_8, &score_16, &score_32, &bbox_8, &bbox_16, &bbox_32, &kps_8, &kps_16, &kps_32,
        ];

        let faces =
            decode_scrfd_face(&outputs, &attrs, &layout, 0.5, 0.45).expect("SCRFD 解码应成功");
        assert_eq!(faces.len(), 1);
        let f = &faces[0];
        assert!((f.score - 0.95).abs() < 1e-5);
        // 归一化后的 bbox: x = 64/640 = 0.1, y = 64/384 = 0.166667, w = 40/640 = 0.0625, h = 40/384 = 0.104167
        assert!((f.bbox[0] - 0.1).abs() < 1e-4);
        assert!((f.bbox[1] - (64.0 / 384.0)).abs() < 1e-4);
        assert!((f.bbox[2] - 0.0625).abs() < 1e-4);
        assert!((f.bbox[3] - (40.0 / 384.0)).abs() < 1e-4);
    }

    #[test]
    fn test_decode_yolov8_person_multi_int8_non_square_three_scales() {
        let grid_h = 48usize;
        let grid_w = 80usize;
        let grid_len = grid_h * grid_w;
        let anchor = 10 * grid_w + 10;
        // cls / score 张量填充强负背景：模型输出的是 raw logits，
        // logit 0 经 sigmoid 即为 50% 置信度，不能当作「无目标」。
        let mut output_storage = [
            vec![-10i8; 64 * grid_len],
            vec![-10i8; 80 * grid_len],
            vec![-10i8; grid_len],
            vec![-10i8; 64 * (24 * 40)],
            vec![-10i8; 80 * (24 * 40)],
            vec![-10i8; 24 * 40],
            vec![-10i8; 64 * (12 * 20)],
            vec![-10i8; 80 * (12 * 20)],
            vec![-10i8; 12 * 20],
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
        let mut non_person_class_data = vec![-10i8; 80 * grid_len];
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
    fn test_decode_yolov6_person_multi_int8_non_square_three_scales() {
        let grid_h = 48usize;
        let grid_w = 80usize;
        let grid_len = grid_h * grid_w;
        let anchor = 10 * grid_w + 10;
        let mut output_storage = [
            vec![-10i8; 4 * grid_len],
            vec![-10i8; 80 * grid_len],
            vec![-10i8; grid_len],
            vec![-10i8; 4 * (24 * 40)],
            vec![-10i8; 80 * (24 * 40)],
            vec![-10i8; 24 * 40],
            vec![-10i8; 4 * (12 * 20)],
            vec![-10i8; 80 * (12 * 20)],
            vec![-10i8; 12 * 20],
        ];
        // left = 1, top = 2, right = 3, bottom = 4
        for (side, val) in [1i8, 2, 3, 4].into_iter().enumerate() {
            output_storage[0][side * grid_len + anchor] = val;
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
                zp: 0,
                ..Default::default()
            };
        let attrs = [
            make_attr(0, 4, 48, 80),
            make_attr(1, 80, 48, 80),
            make_attr(2, 1, 48, 80),
            make_attr(3, 4, 24, 40),
            make_attr(4, 80, 24, 40),
            make_attr(5, 1, 24, 40),
            make_attr(6, 4, 12, 20),
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

        let persons = decode_person_multi_int8(&outputs, &attrs, &layout, 0.5, 0.45)
            .expect("非方形三尺度 INT8 YOLOv6 解码应成功");
        assert_eq!(persons.len(), 1);
        assert!(persons[0].score > 0.99);
        assert!((persons[0].bbox[0] - 76.0 / 640.0).abs() < 0.01);
        assert!((persons[0].bbox[1] - 68.0 / 384.0).abs() < 0.01);
        assert!((persons[0].bbox[2] - 32.0 / 640.0).abs() < 0.01);
        assert!((persons[0].bbox[3] - 48.0 / 384.0).abs() < 0.01);
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
