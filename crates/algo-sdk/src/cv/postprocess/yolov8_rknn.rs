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
        parse_multi_branch_with_score_sum(ctx)
    } else {
        parse_multi_branch_without_score_sum(ctx)
    }
}

// ---------------------------------------------------------------------------
// 9-tensor 路径：每 3 个一组 (box, cls, score_sum)
// ---------------------------------------------------------------------------

fn parse_multi_branch_with_score_sum(ctx: &Yolov8ParseContext<'_>) -> Vec<NormBox> {
    let outputs_per_branch = 3;
    let total_branches = ctx.branches.len() / outputs_per_branch;
    if total_branches == 0 {
        return Vec::new();
    }

    let box_channels = ctx.config.dfl_bins * 4;
    let mut candidates = Vec::with_capacity(32);

    for s in 0..total_branches {
        let base = s * outputs_per_branch;
        let box_out = &ctx.branches[base];
        let cls_out = &ctx.branches[base + 1];
        let score_sum_out = &ctx.branches[base + 2];

        let stride = stride_for_branch(s, total_branches);
        let grid_w = (ctx.config.model_input_w as usize) / stride;
        let grid_h = (ctx.config.model_input_h as usize) / stride;
        let grid_len = grid_w * grid_h;

        if box_out.data.len() < box_channels * grid_len
            || cls_out.data.len() < ctx.config.num_classes * grid_len
            || score_sum_out.data.len() < grid_len
        {
            continue;
        }

        let cls_th_i8 = quant_f32(ctx.conf_threshold, cls_out.zp, cls_out.scale);

        for offset in 0..grid_len {
            // ① score_sum 快速过滤
            // SAFETY: 已断言 score_sum_out.data.len() >= grid_len
            let ss_val = dequant_i8(
                unsafe { *score_sum_out.data.get_unchecked(offset) },
                score_sum_out.zp,
                score_sum_out.scale,
            );
            if ss_val < ctx.conf_threshold {
                continue;
            }

            // ② 遍历类别找最高分
            let mut max_class_id = 0usize;
            let mut max_score_i8 = i8::MIN;
            let mut c_offset = offset;
            for c in 0..ctx.config.num_classes {
                // SAFETY: 已断言 cls_out.data.len() >= num_classes * grid_len
                let val = unsafe { *cls_out.data.get_unchecked(c_offset) };
                if val > cls_th_i8 && val > max_score_i8 {
                    max_score_i8 = val;
                    max_class_id = c;
                }
                c_offset += grid_len;
            }

            if max_score_i8 <= cls_th_i8 {
                continue;
            }

            // ③ DFL 解码 box 坐标
            let i = offset / grid_w;
            let j = offset % grid_w;
            let dfl_box = decode_anchor_box(box_out, offset, grid_len, ctx.config.dfl_bins);

            let x1 = (-dfl_box[0] + j as f32 + 0.5) * stride as f32;
            let y1 = (-dfl_box[1] + i as f32 + 0.5) * stride as f32;
            let x2 = (dfl_box[2] + j as f32 + 0.5) * stride as f32;
            let y2 = (dfl_box[3] + i as f32 + 0.5) * stride as f32;

            let raw = NormBox {
                x: (x1 / ctx.config.model_input_w).clamp(0.0, 1.0),
                y: (y1 / ctx.config.model_input_h).clamp(0.0, 1.0),
                w: ((x2 - x1) / ctx.config.model_input_w).clamp(0.0, 1.0),
                h: ((y2 - y1) / ctx.config.model_input_h).clamp(0.0, 1.0),
                confidence: dequant_i8(max_score_i8, cls_out.zp, cls_out.scale),
                class_id: max_class_id as u32,
                label: resolve_label(max_class_id, ctx.labels, ctx.label_fn),
            };
            candidates.push(unmap_box(&raw, ctx.mode, ctx.orig_w, ctx.orig_h));
        }
    }

    if candidates.is_empty() {
        return Vec::new();
    }
    fast_nms(&mut candidates, ctx.iou_threshold);
    candidates
}

// ---------------------------------------------------------------------------
// 6-tensor 路径：每 2 个一组 (box, cls)，无 score_sum
// ---------------------------------------------------------------------------

fn parse_multi_branch_without_score_sum(ctx: &Yolov8ParseContext<'_>) -> Vec<NormBox> {
    let outputs_per_branch = 2;
    let total_branches = ctx.branches.len() / outputs_per_branch;
    if total_branches == 0 {
        return Vec::new();
    }

    let box_channels = ctx.config.dfl_bins * 4;
    let mut candidates = Vec::with_capacity(32);

    for s in 0..total_branches {
        let base = s * outputs_per_branch;
        let box_out = &ctx.branches[base];
        let cls_out = &ctx.branches[base + 1];

        let stride = stride_for_branch(s, total_branches);
        let grid_w = (ctx.config.model_input_w as usize) / stride;
        let grid_h = (ctx.config.model_input_h as usize) / stride;
        let grid_len = grid_w * grid_h;

        if box_out.data.len() < box_channels * grid_len
            || cls_out.data.len() < ctx.config.num_classes * grid_len
        {
            continue;
        }

        let cls_th_i8 = quant_f32(ctx.conf_threshold, cls_out.zp, cls_out.scale);

        for offset in 0..grid_len {
            let mut max_class_id = 0usize;
            let mut max_score_i8 = i8::MIN;
            let mut c_offset = offset;
            for c in 0..ctx.config.num_classes {
                // SAFETY: 已断言 cls_out.data.len() >= num_classes * grid_len
                let val = unsafe { *cls_out.data.get_unchecked(c_offset) };
                if val > cls_th_i8 && val > max_score_i8 {
                    max_score_i8 = val;
                    max_class_id = c;
                }
                c_offset += grid_len;
            }

            if max_score_i8 <= cls_th_i8 {
                continue;
            }

            let i = offset / grid_w;
            let j = offset % grid_w;
            let dfl_box = decode_anchor_box(box_out, offset, grid_len, ctx.config.dfl_bins);

            let x1 = (-dfl_box[0] + j as f32 + 0.5) * stride as f32;
            let y1 = (-dfl_box[1] + i as f32 + 0.5) * stride as f32;
            let x2 = (dfl_box[2] + j as f32 + 0.5) * stride as f32;
            let y2 = (dfl_box[3] + i as f32 + 0.5) * stride as f32;

            let raw = NormBox {
                x: (x1 / ctx.config.model_input_w).clamp(0.0, 1.0),
                y: (y1 / ctx.config.model_input_h).clamp(0.0, 1.0),
                w: ((x2 - x1) / ctx.config.model_input_w).clamp(0.0, 1.0),
                h: ((y2 - y1) / ctx.config.model_input_h).clamp(0.0, 1.0),
                confidence: dequant_i8(max_score_i8, cls_out.zp, cls_out.scale),
                class_id: max_class_id as u32,
                label: resolve_label(max_class_id, ctx.labels, ctx.label_fn),
            };
            candidates.push(unmap_box(&raw, ctx.mode, ctx.orig_w, ctx.orig_h));
        }
    }

    if candidates.is_empty() {
        return Vec::new();
    }
    fast_nms(&mut candidates, ctx.iou_threshold);
    candidates
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
fn decode_anchor_box(
    box_out: &RknnTensorOutput<'_>,
    offset: usize,
    grid_len: usize,
    dfl_bins: usize,
) -> [f32; 4] {
    if dfl_bins == 0 {
        return [0.0; 4];
    }

    let mut dfl_box = [0.0f32; 4];
    for (side, distance) in dfl_box.iter_mut().enumerate() {
        let side_offset = offset + side * dfl_bins * grid_len;
        let mut max_value = f32::NEG_INFINITY;
        for bin in 0..dfl_bins {
            // SAFETY: 调用方已断言 box_out.data.len() >= dfl_bins * 4 * grid_len。
            let value = unsafe { *box_out.data.get_unchecked(side_offset + bin * grid_len) };
            max_value = max_value.max(dequant_i8(value, box_out.zp, box_out.scale));
        }

        let mut exp_sum = 0.0f32;
        let mut weighted_sum = 0.0f32;
        for bin in 0..dfl_bins {
            // SAFETY: 与上面的边界证明相同。
            let value = unsafe { *box_out.data.get_unchecked(side_offset + bin * grid_len) };
            let exp = (dequant_i8(value, box_out.zp, box_out.scale) - max_value).exp();
            exp_sum += exp;
            weighted_sum += exp * bin as f32;
        }
        *distance = weighted_sum / exp_sum;
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
