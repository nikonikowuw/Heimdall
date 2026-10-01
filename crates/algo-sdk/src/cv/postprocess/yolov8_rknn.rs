//! YOLOv8 RKNN 多分支 INT8 解析器
//!
//! 组合 `quantize` 与 `dfl` 原语，解析 RKNN 优化版 YOLOv8 模型的多分支输出。
//! 支持 9-tensor（含 score_sum 快筛）与 6-tensor（无 score_sum）两种输出结构。

use crate::cv::types::PreprocessMode;
use crate::error::AlgoError;
use crate::math::{fast_nms, unmap_box, NormBox};

use super::quantize::dequant_i8;

/// 单个 INT8 量化张量的元数据与数据视图
#[derive(Debug, Clone)]
pub struct RknnTensorOutput<'a> {
    pub index: u32,
    pub dims: [u32; 4],
    pub scale: f32,
    pub zp: i32,
    pub data: &'a [i8],
}

/// 分类分支的激活语义
///
/// 与 `Yolov8RknnConfig::use_score_sum` 组合时存在硬约束：
/// score_sum 预筛依赖 `score_sum >= max_class_score >= 阈值`，该不等式在
/// **概率语义**下成立，在 **logits 语义**下不成立（其余类别的负 logit 会把
/// 总和压低）。因此“9-tensor + logits”是无法正确解码的组合，
/// 通过 [`Yolov8RknnConfig::from_spec`] 在构造期拒绝，使其无法被表达。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassActivation {
    /// 图中已含 sigmoid：原始值即概率，直接与阈值比较
    Probability,
    /// sigmoid 已移出计算图：原始值含负值，需还原并换算阈值
    Logits,
}

impl ClassActivation {
    /// 判定条件是否为“阈值以上”
    ///
    /// `Logits` 语义下等价换算到 logit 空间（`sigmoid` 单调）：
    /// `sigmoid(x) > p ⟺ x > ln(p/(1-p))`。
    #[inline]
    pub fn effective_threshold(self, p: f32) -> f32 {
        match self {
            Self::Probability => p,
            Self::Logits => logit(p),
        }
    }

    /// 将分支原始值还原为概率置信度
    #[inline]
    pub fn to_confidence(self, raw: f32) -> f32 {
        match self {
            Self::Probability => raw,
            Self::Logits => sigmoid(raw),
        }
    }
}

/// YOLOv8 RKNN 后处理配置
///
/// 字段全部私有：唯一构造入口是 [`Yolov8RknnConfig::from_spec`]，
/// 使“9-tensor + logits”这类无法正确解码的组合在类型层面不可表达。
#[derive(Debug, Clone)]
pub struct Yolov8RknnConfig {
    /// 模型输入宽度（像素），如 640
    model_input_w: f32,
    /// 模型输入高度（像素），如 384
    model_input_h: f32,
    /// DFL bins 数量（YOLOv8 标准为 16）
    dfl_bins: usize,
    /// 类别数（如 2 = fire/smoke，80 = COCO）
    num_classes: usize,
    /// 是否启用 score_sum 快速过滤（true = 9-tensor 优化版，false = 6-tensor 标准版）
    use_score_sum: bool,
    /// 分类分支的激活语义
    class_activation: ClassActivation,
}

impl Yolov8RknnConfig {
    /// 由模型规格构造解码配置，守卫“9-tensor + logits”非法组合
    ///
    /// `use_score_sum = true` 时要求分类分支为概率语义。若模型导出时 sigmoid 已被
    /// 移出计算图（`cls_is_logits = true`），score_sum 预筛会过滤掉本该通过的网格，
    /// 导致静默漏检；此处拒绝而不是静默降级。
    pub fn from_spec(
        model_input_w: f32,
        model_input_h: f32,
        dfl_bins: usize,
        num_classes: usize,
        use_score_sum: bool,
        cls_is_logits: bool,
    ) -> Result<Self, AlgoError> {
        let class_activation = if cls_is_logits {
            ClassActivation::Logits
        } else {
            ClassActivation::Probability
        };

        if use_score_sum && class_activation == ClassActivation::Logits {
            return Err(AlgoError::ConfigParse {
                reason: "score_sum 预筛仅适用于概率语义的分类分支；\
                         sigmoid 已移出计算图（logits）的模型必须使用 6-tensor 输出结构"
                    .to_string(),
            });
        }

        Ok(Self {
            model_input_w,
            model_input_h,
            dfl_bins,
            num_classes,
            use_score_sum,
            class_activation,
        })
    }
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

        let cls_th_i8 = threshold_i8(
            ctx.config
                .class_activation
                .effective_threshold(ctx.conf_threshold),
            cls_out.zp,
            cls_out.scale,
        );
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

/// 概率 → logit：`ln(p / (1 - p))`
///
/// 端点做防溢出夹逼：`p <= 0` 视为接受任意正 logit，`p >= 1` 视为拒绝全部。
#[inline]
fn logit(p: f32) -> f32 {
    let p = p.clamp(f32::MIN_POSITIVE, 1.0 - f32::EPSILON);
    (p / (1.0 - p)).ln()
}

/// 数值稳定的 sigmoid（负半轴直接求 `e^x`，避免 `exp` 溢出）
#[inline]
fn sigmoid(x: f32) -> f32 {
    if x >= 0.0 {
        1.0 / (1.0 + (-x).exp())
    } else {
        let e = x.exp();
        e / (1.0 + e)
    }
}

/// 将实数阈值换算为「严格大于」比较所需的 INT8 下界
///
/// 判定目标是 `dequant_i8(v) > threshold`，即 `v > threshold / scale + zp`。
/// 因 `v` 为整数，该条件**严格等价**于 `v > floor(threshold / scale + zp)`。
///
/// 不得改用四舍五入（`quant_f32`）：舍入会把阈值抬高最多半个量化步长，
/// 从而丢弃本应通过的边界网格——例如 `scale = 1.0` 下 logit 阈值 `-0.2007`
/// 被舍入为 `0`，使 `logit = 0`（sigmoid = 0.5 > 0.45）的网格被误删。
#[inline]
fn threshold_i8(val: f32, zp: i32, scale: f32) -> i8 {
    // 与 `dequant_i8` / `quant_f32` 保持一致的退化输入处理：元数据非法时
    // `dequant_i8` 恒返回 0.0，此处以 zp 为界保守比较。
    if !scale.is_finite() || scale <= 0.0 || !val.is_finite() {
        return zp.clamp(-128, 127) as i8;
    }
    let v = val / scale + zp as f32;
    if !v.is_finite() {
        return zp.clamp(-128, 127) as i8;
    }
    v.floor().clamp(-128.0, 127.0) as i8
}

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

    let raw_score = dequant_i8(score_i8, cls_out.zp, cls_out.scale);
    let raw = NormBox {
        x: (x1 / ctx.config.model_input_w).clamp(0.0, 1.0),
        y: (y1 / ctx.config.model_input_h).clamp(0.0, 1.0),
        w: ((x2 - x1) / ctx.config.model_input_w).clamp(0.0, 1.0),
        h: ((y2 - y1) / ctx.config.model_input_h).clamp(0.0, 1.0),
        confidence: ctx.config.class_activation.to_confidence(raw_score),
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
            class_activation: ClassActivation::Probability,
        }
    }

    /// 官方 6-tensor 结构（无 score_sum），sigmoid 保留在图内
    fn config_6tensor_probability() -> Yolov8RknnConfig {
        Yolov8RknnConfig {
            model_input_w: 8.0,
            model_input_h: 8.0,
            dfl_bins: 1,
            num_classes: 1,
            use_score_sum: false,
            class_activation: ClassActivation::Probability,
        }
    }

    /// 官方 6-tensor 结构，sigmoid 已移出计算图
    fn config_6tensor_logits() -> Yolov8RknnConfig {
        Yolov8RknnConfig {
            class_activation: ClassActivation::Logits,
            ..config_6tensor_probability()
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
            class_activation: ClassActivation::Probability,
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
            class_activation: ClassActivation::Probability,
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

    /// 阈值必须向下取整，使 `v > threshold_i8` 与实数比较 `dequant(v) > t` 严格等价。
    ///
    /// 若改回四舍五入，阈值会被抬高最多半个量化步长，丢弃边界网格：
    /// `scale = 1.0` 时 logit 阈值 `-0.2007` 会被舍入为 `0`，导致 `logit = 0`
    /// （sigmoid 0.5）的网格被误删；改用 `floor` 得 `-1`，判定正确。
    #[test]
    fn test_threshold_quantization_uses_floor_for_strict_comparison() {
        // 概率阈值 0.45 换算到 logit 空间
        let t = ClassActivation::Logits.effective_threshold(0.45);
        assert!(t < 0.0 && t > -1.0, "预期阈值落在 (-1, 0)，实际 {t}");
        assert_eq!(threshold_i8(t, 0, 1.0), -1, "阈值必须向下取整为 -1");

        // 边界网格：logit = 0 应通过（sigmoid(0)=0.5 > 0.45）
        let th = threshold_i8(t, 0, 1.0);
        assert!(0i8 > th, "logit 0 必须先于阈值比较通过");

        // 整阈值不得被 floor 降低：t = 2.0 应仍为 2（v > 2 与 v > 2.0 等价）
        assert_eq!(threshold_i8(2.0, 0, 1.0), 2);
        // zp 偏移必须计入：t = 0.0, zp = 10 → 边界 10
        assert_eq!(threshold_i8(0.0, 10, 1.0), 10);
        // 典型量化步长下的等价性抽样校验
        for raw in -128i8..=127 {
            let dequantized = dequant_i8(raw, 3, 0.0378);
            let th = threshold_i8(-0.2, 3, 0.0378);
            assert_eq!(
                raw > th,
                dequantized > -0.2,
                "raw={raw} 下整数比较与实数比较不等价"
            );
        }
    }

    /// `use_score_sum` 与 logits 语义是互斥的：score_sum 预筛依赖
    /// `score_sum >= max_class_score >= 阈值`，该不等式在 logits 语义下不成立。
    /// 构造期必须直接拒绝，否则会静默漏检。
    #[test]
    fn test_from_spec_rejects_score_sum_with_logits() {
        let error = Yolov8RknnConfig::from_spec(640.0, 384.0, 16, 2, true, true)
            .expect_err("9-tensor + logits 必须在构造期被拒绝");
        assert!(
            matches!(error, AlgoError::ConfigParse { .. }),
            "应为配置类错误，实际 {error:?}"
        );

        // 合法组合均放行
        assert!(Yolov8RknnConfig::from_spec(640.0, 384.0, 16, 2, true, false).is_ok());
        assert!(Yolov8RknnConfig::from_spec(640.0, 384.0, 16, 2, false, true).is_ok());
        assert!(Yolov8RknnConfig::from_spec(640.0, 384.0, 16, 2, false, false).is_ok());
    }

    /// logit 换算必须与 `sigmoid(x) > p` 单调等价（阈值比较的正确性根基）。
    #[test]
    fn test_logit_threshold_equivalence() {
        for p in [0.05f32, 0.25, 0.45, 0.5, 0.8, 0.95] {
            let th = logit(p);
            // 阈值点两侧的判定必须与直接比较 sigmoid 一致
            for delta in [-0.5f32, -0.01, 0.01, 0.5] {
                let x = th + delta;
                assert_eq!(
                    x > th,
                    sigmoid(x) > p,
                    "p={p} x={x} 下 logit 阈值比较与 sigmoid 直接比较不一致"
                );
            }
        }
    }

    #[test]
    fn test_sigmoid_matches_reference_and_is_stable() {
        assert!((sigmoid(0.0) - 0.5).abs() < 1e-6);
        assert!((sigmoid(1.0) - 0.7310586).abs() < 1e-5);
        assert!((sigmoid(-1.0) - 0.26894143).abs() < 1e-5);
        // 极端输入不得溢出为 NaN/Inf
        assert_eq!(sigmoid(200.0), 1.0);
        assert!(sigmoid(-200.0).is_finite());
        assert!(sigmoid(-200.0).abs() < 1e-6);
    }

    /// 负 logits 是 logits 分支的核心特征：未经还原时置信度会系统性错误。
    #[test]
    fn test_negative_logits_are_sigmoid_decoded() {
        // score 原始值 -4.0 对应 logits（旧路径会当作置信度直接输出负值）
        let box_data = [0i8; 4];
        let class_data = [-4i8];
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
                scale: 1.0,
                zp: 0,
                data: &class_data,
            },
        ];
        let mode = PreprocessMode::Resize;
        let labels = ["object"];

        // (a) 已激活模型语义：-4.0 低于阈值，应被过滤
        let cfg_prob = config_6tensor_probability();
        let ctx_prob = Yolov8ParseContext {
            branches: &branches,
            config: &cfg_prob,
            conf_threshold: 0.01,
            iou_threshold: 0.45,
            labels: &labels,
            label_fn: None,
            mode: &mode,
            orig_w: 8,
            orig_h: 8,
        };
        assert!(
            parse_yolov8_int8(&ctx_prob).is_empty(),
            "已激活语义下 -4.0 应被阈值过滤"
        );

        // (b) logits 模型语义：sigmoid(-4.0)=0.0180，高于 0.01 阈值，应保留
        let cfg_logit = config_6tensor_logits();
        let ctx_logit = Yolov8ParseContext {
            branches: &branches,
            config: &cfg_logit,
            conf_threshold: 0.01,
            iou_threshold: 0.45,
            labels: &labels,
            label_fn: None,
            mode: &mode,
            orig_w: 8,
            orig_h: 8,
        };
        let boxes = parse_yolov8_int8(&ctx_logit);
        assert_eq!(boxes.len(), 1, "logits 语义下应还原并保留该候选");
        assert!(
            (boxes[0].confidence - sigmoid(-4.0)).abs() < 1e-5,
            "置信度必须为 sigmoid 还原值，实际 {}",
            boxes[0].confidence
        );
        assert!(
            boxes[0].confidence > 0.0 && boxes[0].confidence < 0.5,
            "负 logits 还原后应落在 (0, 0.5)，实际 {}",
            boxes[0].confidence
        );
    }

    /// 端到端锁住新模型契约：官方 6-tensor 分层输出 + logits 分类分支。
    ///
    /// 对应 `yolov8_hard_hat.rknn` 的真实结构：
    /// 分支顺序 `box_8, score_8, box_16, score_16, box_32, score_32`，
    /// 且 score 分支为未激活 logits。若 `USE_SCORE_SUM` 或 `CLS_IS_LOGITS`
    /// 任一被改错，本用例必须失败。
    #[test]
    fn test_six_tensor_logits_pipeline_decodes_expected_box() {
        const BINS: usize = 16;
        const CLASSES: usize = 2;
        // 以 stride=32 分支（grid 12x20=240）避免构造全尺寸张量
        let grid_len = 12 * 20;

        // box_32: 让 anchor(0,0) 的四个方向 DFL 峰值落在已知 bin 上
        let mut box_data = vec![0i8; BINS * 4 * grid_len];
        for (side, peak) in [4usize, 4, 4, 4].into_iter().enumerate() {
            box_data[(side * BINS + peak) * grid_len] = 100;
        }
        // score_32: class 1 (NO-Hardhat) 给正 logit，其余锚点给低于阈值的负 logit。
        // 注意 logits 语义下 sigmoid(0)=0.5 会超过 0.25 阈值，因此不能用 0 做「不命中」值。
        let mut score_data = vec![-120i8; CLASSES * grid_len];
        score_data[grid_len] = 40;
        score_data[0] = -120;

        let branches = [
            // stride 8 / 16 分支给空张量：它们在长度校验处被安全跳过
            RknnTensorOutput {
                index: 0,
                dims: [1, 64, 48, 80],
                scale: 1.0,
                zp: 0,
                data: &[],
            },
            RknnTensorOutput {
                index: 1,
                dims: [1, 2, 48, 80],
                scale: 1.0,
                zp: 0,
                data: &[],
            },
            RknnTensorOutput {
                index: 2,
                dims: [1, 64, 24, 40],
                scale: 1.0,
                zp: 0,
                data: &[],
            },
            RknnTensorOutput {
                index: 3,
                dims: [1, 2, 24, 40],
                scale: 1.0,
                zp: 0,
                data: &[],
            },
            RknnTensorOutput {
                index: 4,
                dims: [1, 64, 12, 20],
                scale: 0.1,
                zp: 0,
                data: &box_data,
            },
            RknnTensorOutput {
                index: 5,
                dims: [1, 2, 12, 20],
                scale: 0.01,
                zp: 0,
                data: &score_data,
            },
        ];

        let config = Yolov8RknnConfig {
            model_input_w: 640.0,
            model_input_h: 384.0,
            dfl_bins: BINS,
            num_classes: CLASSES,
            use_score_sum: false,
            class_activation: ClassActivation::Logits,
        };
        let mode = PreprocessMode::Resize;
        let labels = ["Hardhat", "NO-Hardhat"];
        let context = Yolov8ParseContext {
            branches: &branches,
            config: &config,
            conf_threshold: 0.25,
            iou_threshold: 0.45,
            labels: &labels,
            label_fn: None,
            mode: &mode,
            orig_w: 640,
            orig_h: 384,
        };

        let boxes = parse_yolov8_int8(&context);
        assert_eq!(boxes.len(), 1, "6-tensor 结构应解出单框");
        assert_eq!(boxes[0].class_id, 1, "应选中 logit 更高的 class 1");
        assert_eq!(boxes[0].label, Some("NO-Hardhat"));
        // logit = 40 * 0.01 = 0.4 → sigmoid(0.4) ≈ 0.5987
        let expected = sigmoid(0.4);
        assert!(
            (boxes[0].confidence - expected).abs() < 1e-4,
            "置信度应为 sigmoid(0.4)={expected}，实际 {}",
            boxes[0].confidence
        );
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
