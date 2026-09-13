//! YOLOv8 RKNN 多分支 INT8 解析器
//!
//! 组合 `quantize` 与 `dfl` 原语，解析 RKNN 优化版 YOLOv8 模型的多分支输出。
//! 支持 9-tensor（含 score_sum 快筛）与 6-tensor（无 score_sum）两种输出结构。

use crate::cv::types::PreprocessMode;
use crate::math::{fast_nms, unmap_box, NormBox};

use super::quantize::{dequant_i8, quant_f32};

/// 单个 INT8 量化张量的元数据与数据视图
#[derive(Debug, Clone)]
pub struct RknnTensorOutput<'a> {
    pub index: u32,
    pub dims: [u32; 4],
    pub scale: f32,
    pub zp: i32,
    pub data: &'a [i8],
}

/// YOLOv8 RKNN 后处理配置
#[derive(Debug, Clone)]
pub struct Yolov8RknnConfig {
    /// 模型输入宽度（像素），如 640
    pub model_input_w: f32,
    /// 模型输入高度（像素），如 384
    pub model_input_h: f32,
    /// DFL bins 数量（YOLOv8 标准为 16）
    pub dfl_bins: usize,
    /// 类别数（如 2 = fire/smoke，80 = COCO）
    pub num_classes: usize,
    /// 是否启用 score_sum 快速过滤（true = 9-tensor 优化版，false = 6-tensor 标准版）
    pub use_score_sum: bool,
}

/// 后处理上下文，聚合推理输出解析所需的全部参数
pub struct Yolov8ParseContext<'a> {
    /// RKNN 模型输出张量切片
    pub branches: &'a [RknnTensorOutput<'a>],
    /// 后处理配置
    pub config: &'a Yolov8RknnConfig,
    /// 置信度过滤阈值
    pub conf_threshold: f32,
    /// NMS IoU 阈值
    pub iou_threshold: f32,
    /// 类别标签表（长度须 >= `config.num_classes`）
    pub labels: &'a [&'static str],
    /// 自定义标签覆盖函数；返回 `Some(label)` 时优先使用
    pub label_fn: Option<&'a dyn Fn(usize) -> Option<&'static str>>,
    /// 预处理模式（Letterbox / Resize），用于坐标反算
    pub mode: &'a PreprocessMode,
    /// 原始帧宽度
    pub orig_w: u32,
    /// 原始帧高度
    pub orig_h: u32,
}

impl<'a> std::fmt::Debug for Yolov8ParseContext<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Yolov8ParseContext")
            .field("branches_len", &self.branches.len())
            .field("config", &self.config)
            .field("conf_threshold", &self.conf_threshold)
            .field("iou_threshold", &self.iou_threshold)
            .field("labels_len", &self.labels.len())
            .field("has_label_fn", &self.label_fn.is_some())
            .field("mode", &self.mode)
            .field("orig_w", &self.orig_w)
            .field("orig_h", &self.orig_h)
            .finish()
    }
}

/// 从 RKNN 多分支 INT8 输出解析检测框的统一入口
///
/// 自动根据 `config.use_score_sum` 选择 9-tensor 或 6-tensor 解析路径。
/// 支持任意类别数、任意 DFL bins、任意输入分辨率。
pub fn parse_yolov8_int8(ctx: &Yolov8ParseContext<'_>) -> Vec<NormBox> {
    if ctx.orig_w == 0
        || ctx.orig_h == 0
        || ctx.config.model_input_w <= 0.0
        || ctx.config.model_input_h <= 0.0
    {
        return Vec::new();
    }

    if ctx.config.use_score_sum {
        parse_multi_branch::<true>(ctx)
    } else {
        parse_multi_branch::<false>(ctx)
    }
}

// ---------------------------------------------------------------------------
// 多分支路径：每个分支包含 (box, cls)，可选 score_sum
// ---------------------------------------------------------------------------

fn parse_multi_branch<const WITH_SCORE_SUM: bool>(ctx: &Yolov8ParseContext<'_>) -> Vec<NormBox> {
    let outputs_per_branch = if WITH_SCORE_SUM { 3 } else { 2 };
    let total_branches = ctx.branches.len() / outputs_per_branch;
    if total_branches == 0 {
        return Vec::new();
    }

    let box_channels = ctx.config.dfl_bins * 4;
    let mut candidates = Vec::with_capacity(32);

    for branch_idx in 0..total_branches {
        let base = branch_idx * outputs_per_branch;
        let box_out = &ctx.branches[base];
        let cls_out = &ctx.branches[base + 1];
        let score_sum_out = if WITH_SCORE_SUM {
            Some(&ctx.branches[base + 2])
        } else {
            None
        };

        let stride = stride_for_branch(branch_idx, total_branches);
        let grid_w = (ctx.config.model_input_w as usize) / stride;
        let grid_h = (ctx.config.model_input_h as usize) / stride;
        let grid_len = grid_w * grid_h;

        if box_out.data.len() < box_channels * grid_len
            || cls_out.data.len() < ctx.config.num_classes * grid_len
            || score_sum_out.is_some_and(|output| output.data.len() < grid_len)
        {
            continue;
        }

        let cls_th_i8 = quant_f32(ctx.conf_threshold, cls_out.zp, cls_out.scale);
        let score_sum_pass = score_sum_out.map_or([true; SCORE_SUM_LUT_LEN], |output| {
            score_sum_pass_lut(output, ctx.conf_threshold)
        });
        let stride_f = stride as f32;

        for row in 0..grid_h {
            let row_offset = row * grid_w;
            let cy = (row as f32 + 0.5) * stride_f;

            for column in 0..grid_w {
                let offset = row_offset + column;

                if let Some(score_sum_out) = score_sum_out {
                    // SAFETY: 已断言 score_sum_out.data.len() >= grid_len。
                    let value = unsafe { *score_sum_out.data.get_unchecked(offset) };
                    if !score_sum_pass[value as u8 as usize] {
                        continue;
                    }
                }

                let mut max_class_id = 0usize;
                let mut max_score_i8 = i8::MIN;
                let mut class_offset = offset;
                for class_id in 0..ctx.config.num_classes {
                    // SAFETY: 已断言 cls_out.data.len() >= num_classes * grid_len。
                    let value = unsafe { *cls_out.data.get_unchecked(class_offset) };
                    if value > cls_th_i8 && value > max_score_i8 {
                        max_score_i8 = value;
                        max_class_id = class_id;
                    }
                    class_offset += grid_len;
                }

                if max_score_i8 <= cls_th_i8 {
                    continue;
                }

                push_candidate(
                    ctx,
                    box_out,
                    cls_out,
                    CandidateGeometry {
                        offset,
                        grid_len,
                        cx: (column as f32 + 0.5) * stride_f,
                        cy,
                        stride: stride_f,
                    },
                    max_class_id,
                    max_score_i8,
                    &mut candidates,
                );
            }
        }
    }

    if candidates.is_empty() {
        return Vec::new();
    }
    fast_nms(&mut candidates, ctx.iou_threshold);
    candidates
}

const SCORE_SUM_LUT_LEN: usize = 256;

/// 为所有可能的 INT8 score_sum 值预计算原始浮点判定，避免逐网格重复反量化。
#[inline]
fn score_sum_pass_lut(output: &RknnTensorOutput<'_>, threshold: f32) -> [bool; SCORE_SUM_LUT_LEN] {
    std::array::from_fn(|raw| {
        let value = raw as u8 as i8;
        !matches!(
            dequant_i8(value, output.zp, output.scale).partial_cmp(&threshold),
            Some(std::cmp::Ordering::Less)
        )
    })
}

#[derive(Debug, Clone, Copy)]
struct CandidateGeometry {
    offset: usize,
    grid_len: usize,
    cx: f32,
    cy: f32,
    stride: f32,
}

/// 将单个网格候选解码、反算坐标并追加到结果集。
#[inline]
fn push_candidate(
    ctx: &Yolov8ParseContext<'_>,
    box_out: &RknnTensorOutput<'_>,
    cls_out: &RknnTensorOutput<'_>,
    geometry: CandidateGeometry,
    class_id: usize,
    score_i8: i8,
    candidates: &mut Vec<NormBox>,
) {
    let dfl_box = decode_anchor_box(
        box_out,
        geometry.offset,
        geometry.grid_len,
        ctx.config.dfl_bins,
    );
    let x1 = -dfl_box[0] * geometry.stride + geometry.cx;
    let y1 = -dfl_box[1] * geometry.stride + geometry.cy;
    let x2 = dfl_box[2] * geometry.stride + geometry.cx;
    let y2 = dfl_box[3] * geometry.stride + geometry.cy;

    let raw = NormBox {
        x: (x1 / ctx.config.model_input_w).clamp(0.0, 1.0),
        y: (y1 / ctx.config.model_input_h).clamp(0.0, 1.0),
        w: ((x2 - x1) / ctx.config.model_input_w).clamp(0.0, 1.0),
        h: ((y2 - y1) / ctx.config.model_input_h).clamp(0.0, 1.0),
        confidence: dequant_i8(score_i8, cls_out.zp, cls_out.scale),
        class_id: class_id as u32,
        label: resolve_label(class_id, ctx.labels, ctx.label_fn),
    };
    candidates.push(unmap_box(&raw, ctx.mode, ctx.orig_w, ctx.orig_h));
}

// ---------------------------------------------------------------------------
// 内部工具函数
// ---------------------------------------------------------------------------

/// 根据分支索引计算 stride（适配 P3/P4/P5 三检测头）
fn stride_for_branch(branch_idx: usize, total_branches: usize) -> usize {
    match total_branches {
        3 => [8, 16, 32][branch_idx.min(2)],
        _ => 8usize << branch_idx,
    }
}

/// 从 box 张量中解码单个 anchor 的 DFL 坐标 → [x1, y1, x2, y2]
///
/// 利用 `(val - zp) * scale` 在原生 `i8` 上的单调性直接进行极值比较，
/// 并将 `exp()` 超越函数计算合并至单次加权循环，大幅提升向量化吞吐。
#[inline]
fn decode_anchor_box(
    box_out: &RknnTensorOutput<'_>,
    offset: usize,
    grid_len: usize,
    dfl_bins: usize,
) -> [f32; 4] {
    if dfl_bins == 0 {
        return [0.0; 4];
    }

    let scale = if box_out.scale.is_finite() && box_out.scale > 0.0 {
        box_out.scale
    } else {
        0.0
    };

    let mut dfl_box = [0.0f32; 4];
    for (side, distance) in dfl_box.iter_mut().enumerate() {
        let side_offset = offset + side * dfl_bins * grid_len;

        // ① 纯整数比较求极值，零浮点开销
        let mut max_i8 = i8::MIN;
        for bin in 0..dfl_bins {
            // SAFETY: 调用方已断言 box_out.data.len() >= dfl_bins * 4 * grid_len。
            let value = unsafe { *box_out.data.get_unchecked(side_offset + bin * grid_len) };
            if value > max_i8 {
                max_i8 = value;
            }
        }

        // ② 单次 exp() 计算与加权累加
        let mut exp_sum = 0.0f32;
        let mut weighted_sum = 0.0f32;
        for bin in 0..dfl_bins {
            // SAFETY: 与上面的边界证明相同。
            let value = unsafe { *box_out.data.get_unchecked(side_offset + bin * grid_len) };
            let diff = (value as i32 - max_i8 as i32) as f32 * scale;
            let exp_v = diff.exp();
            exp_sum += exp_v;
            weighted_sum += exp_v * (bin as f32);
        }
        *distance = if exp_sum > 0.0 {
            weighted_sum / exp_sum
        } else {
            0.0
        };
    }
    dfl_box
}

/// 解析标签：优先使用自定义标签函数，其次从标签表查询
fn resolve_label<'a>(
    class_id: usize,
    labels: &[&'a str],
    label_fn: Option<&dyn Fn(usize) -> Option<&'a str>>,
) -> Option<&'a str> {
    if let Some(f) = label_fn {
        if let Some(l) = f(class_id) {
            return Some(l);
        }
    }
    labels.get(class_id).copied()
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cv::types::LetterboxLayout;

    fn config_9tensor() -> Yolov8RknnConfig {
        Yolov8RknnConfig {
            model_input_w: 640.0,
            model_input_h: 384.0,
            dfl_bins: 16,
            num_classes: 2,
            use_score_sum: true,
        }
    }

    fn mode() -> PreprocessMode {
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
    fn test_resolve_label_from_table() {
        let labels = ["fire", "smoke"];
        assert_eq!(resolve_label(0, &labels, None), Some("fire"));
        assert_eq!(resolve_label(1, &labels, None), Some("smoke"));
        assert_eq!(resolve_label(5, &labels, None), None);
    }

    #[test]
    fn test_resolve_label_custom_fn() {
        let labels = ["fire", "smoke"];
        let custom: &dyn Fn(usize) -> Option<&'static str> = &|id| {
            if id == 0 {
                Some("火焰告警")
            } else {
                None
            }
        };
        assert_eq!(resolve_label(0, &labels, Some(custom)), Some("火焰告警"));
        assert_eq!(resolve_label(1, &labels, Some(custom)), Some("smoke"));
    }

    #[test]
    fn test_stride_standard_3branch() {
        assert_eq!(stride_for_branch(0, 3), 8);
        assert_eq!(stride_for_branch(1, 3), 16);
        assert_eq!(stride_for_branch(2, 3), 32);
    }

    #[test]
    fn test_stride_generic() {
        assert_eq!(stride_for_branch(0, 5), 8);
        assert_eq!(stride_for_branch(3, 5), 64);
    }

    #[test]
    fn test_decode_anchor_box_reads_strided_channels_without_allocation() {
        let dfl_bins = 4;
        let grid_len = 2;
        let offset = 1;
        let mut data = vec![-10i8; dfl_bins * 4 * grid_len];
        for (side, peak_bin) in [0usize, 1, 2, 3].into_iter().enumerate() {
            data[(side * dfl_bins + peak_bin) * grid_len + offset] = 10;
        }
        let output = RknnTensorOutput {
            index: 0,
            dims: [1, 16, 1, 2],
            scale: 1.0,
            zp: 0,
            data: &data,
        };

        let decoded = decode_anchor_box(&output, offset, grid_len, dfl_bins);
        for (actual, expected) in decoded.into_iter().zip([0.0, 1.0, 2.0, 3.0]) {
            assert!((actual - expected).abs() < 0.01);
        }
    }

    #[test]
    fn test_score_sum_lut_matches_float_threshold_boundary() {
        let data = [];
        let output = RknnTensorOutput {
            index: 0,
            dims: [1, 1, 1, 1],
            scale: 0.1,
            zp: 0,
            data: &data,
        };
        let pass = score_sum_pass_lut(&output, 0.21);

        assert!(!pass[2_i8 as u8 as usize]);
        assert!(pass[3_i8 as u8 as usize]);
    }

    #[test]
    fn test_parse_single_branch_with_score_sum() {
        let box_data = [0i8; 4];
        let class_data = [10i8];
        let score_sum_data = [10i8];
        let branches = [
            RknnTensorOutput {
                index: 0,
                dims: [1, 4, 1, 1],
                scale: 1.0,
                zp: 0,
                data: &box_data,
            },
            RknnTensorOutput {
                index: 1,
                dims: [1, 1, 1, 1],
                scale: 0.1,
                zp: 0,
                data: &class_data,
            },
            RknnTensorOutput {
                index: 2,
                dims: [1, 1, 1, 1],
                scale: 0.01,
                zp: 0,
                data: &score_sum_data,
            },
        ];
        let config = Yolov8RknnConfig {
            model_input_w: 8.0,
            model_input_h: 8.0,
            dfl_bins: 1,
            num_classes: 1,
            use_score_sum: true,
        };
        let mode = PreprocessMode::Resize;
        let context = Yolov8ParseContext {
            branches: &branches,
            config: &config,
            conf_threshold: 0.05,
            iou_threshold: 0.45,
            labels: &["object"],
            label_fn: None,
            mode: &mode,
            orig_w: 8,
            orig_h: 8,
        };

        let boxes = parse_yolov8_int8(&context);
        assert_eq!(boxes.len(), 1);
        assert_eq!(boxes[0].label, Some("object"));
        assert!((boxes[0].confidence - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_parse_single_branch_without_score_sum() {
        let box_data = [0i8; 4];
        let class_data = [10i8];
        let branches = [
            RknnTensorOutput {
                index: 0,
                dims: [1, 4, 1, 1],
                scale: 1.0,
                zp: 0,
                data: &box_data,
            },
            RknnTensorOutput {
                index: 1,
                dims: [1, 1, 1, 1],
                scale: 0.1,
                zp: 0,
                data: &class_data,
            },
        ];
        let config = Yolov8RknnConfig {
            model_input_w: 8.0,
            model_input_h: 8.0,
            dfl_bins: 1,
            num_classes: 1,
            use_score_sum: false,
        };
        let mode = PreprocessMode::Resize;
        let context = Yolov8ParseContext {
            branches: &branches,
            config: &config,
            conf_threshold: 0.05,
            iou_threshold: 0.45,
            labels: &["object"],
            label_fn: None,
            mode: &mode,
            orig_w: 8,
            orig_h: 8,
        };

        let boxes = parse_yolov8_int8(&context);
        assert_eq!(boxes.len(), 1);
        assert!((boxes[0].confidence - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_empty_branches() {
        let config = config_9tensor();
        let ctx = Yolov8ParseContext {
            branches: &[],
            config: &config,
            conf_threshold: 0.25,
            iou_threshold: 0.45,
            labels: &["fire", "smoke"],
            label_fn: None,
            mode: &mode(),
            orig_w: 1920,
            orig_h: 1080,
        };
        assert!(parse_yolov8_int8(&ctx).is_empty());
    }

    #[test]
    fn test_invalid_dimensions() {
        let config = config_9tensor();
        let ctx = Yolov8ParseContext {
            branches: &[],
            config: &config,
            conf_threshold: 0.25,
            iou_threshold: 0.45,
            labels: &["fire", "smoke"],
            label_fn: None,
            mode: &mode(),
            orig_w: 0,
            orig_h: 0,
        };
        assert!(parse_yolov8_int8(&ctx).is_empty());
    }
}
