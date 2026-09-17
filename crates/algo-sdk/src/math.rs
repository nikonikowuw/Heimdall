//! 向量化几何后处理 (math)
//! 提供 IoU、NMS、Letterbox 坐标反算去黑边与边界安全截断。

use crate::cv::types::PreprocessMode;
use crate::error::AlgoError;
use base64::Engine;

/// 归一化检测框描述符（坐标与尺寸均归一化至 `[0.0, 1.0]`）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NormBox {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub confidence: f32,
    pub class_id: u32,
    pub label: Option<&'static str>,
}

impl NormBox {
    pub fn new(x: f32, y: f32, w: f32, h: f32, confidence: f32, class_id: u32) -> Self {
        Self {
            x,
            y,
            w,
            h,
            confidence,
            class_id,
            label: None,
        }
    }

    pub fn with_label(mut self, label: &'static str) -> Self {
        self.label = Some(label);
        self
    }
}

/// 计算两个归一化框的交并比 (IoU)
#[inline(always)]
pub fn calculate_iou(a: &NormBox, b: &NormBox) -> f32 {
    let a_x2 = a.x + a.w;
    let a_y2 = a.y + a.h;
    let area_a = (a.w * a.h).max(0.0);
    calculate_iou_with_precomputed(a_x2, a_y2, area_a, a, b)
}

#[inline(always)]
fn calculate_iou_with_precomputed(
    a_x2: f32,
    a_y2: f32,
    area_a: f32,
    a: &NormBox,
    b: &NormBox,
) -> f32 {
    let b_x2 = b.x + b.w;
    let b_y2 = b.y + b.h;

    let inter_x1 = a.x.max(b.x);
    let inter_y1 = a.y.max(b.y);
    let inter_x2 = a_x2.min(b_x2);
    let inter_y2 = a_y2.min(b_y2);

    let inter_w = (inter_x2 - inter_x1).max(0.0);
    let inter_h = (inter_y2 - inter_y1).max(0.0);
    let inter_area = inter_w * inter_h;

    let area_b = (b.w * b.h).max(0.0);
    let union_area = area_a + area_b - inter_area;

    if union_area <= 0.0 {
        0.0
    } else {
        inter_area / union_area
    }
}

const STACK_MASK_WORDS: usize = 16; // 16 * 64 = 1024 框以内保持栈上零分配

enum BitMask {
    Stack([u64; STACK_MASK_WORDS]),
    Heap(Vec<u64>),
}

impl BitMask {
    #[inline]
    fn new(len: usize) -> Self {
        let words = len.div_ceil(64);
        if words <= STACK_MASK_WORDS {
            BitMask::Stack([0; STACK_MASK_WORDS])
        } else {
            BitMask::Heap(vec![0; words])
        }
    }

    #[inline(always)]
    fn is_set(&self, idx: usize) -> bool {
        let word = idx >> 6;
        let bit = idx & 63;
        match self {
            BitMask::Stack(arr) => (arr[word] & (1u64 << bit)) != 0,
            BitMask::Heap(vec) => (vec[word] & (1u64 << bit)) != 0,
        }
    }

    #[inline(always)]
    fn set(&mut self, idx: usize) {
        let word = idx >> 6;
        let bit = idx & 63;
        match self {
            BitMask::Stack(arr) => arr[word] |= 1u64 << bit,
            BitMask::Heap(vec) => vec[word] |= 1u64 << bit,
        }
    }
}

/// 高性能非极大值抑制 (NMS)，默认类别相关抑制 (Class-aware)
///
/// 采用位图掩码与就地双指针压缩，避免额外的 keep/suppressed 工作数组分配。
#[inline]
pub fn fast_nms(boxes: &mut Vec<NormBox>, iou_threshold: f32) {
    fast_nms_impl::<false>(boxes, iou_threshold);
}

/// 类别无关非极大值抑制 (Class-agnostic NMS)
///
/// 采用位图掩码与就地双指针压缩，避免额外的 keep/suppressed 工作数组分配。
#[inline]
pub fn fast_nms_agnostic(boxes: &mut Vec<NormBox>, iou_threshold: f32) {
    fast_nms_impl::<true>(boxes, iou_threshold);
}

#[inline]
fn fast_nms_impl<const AGNOSTIC: bool>(boxes: &mut Vec<NormBox>, iou_threshold: f32) {
    let n = boxes.len();
    if n <= 1 {
        return;
    }

    // 按置信度降序排序
    boxes.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));

    let mut mask = BitMask::new(n);
    let mut write_idx = 0;

    for i in 0..n {
        if mask.is_set(i) {
            continue;
        }

        let a = boxes[i];
        let a_x2 = a.x + a.w;
        let a_y2 = a.y + a.h;
        let area_a = (a.w * a.h).max(0.0);

        for (j, b) in boxes.iter().enumerate().skip(i + 1) {
            if mask.is_set(j) {
                continue;
            }
            if AGNOSTIC || a.class_id == b.class_id {
                let iou = calculate_iou_with_precomputed(a_x2, a_y2, area_a, &a, b);
                if iou >= iou_threshold {
                    mask.set(j);
                }
            }
        }

        if write_idx != i {
            boxes[write_idx] = a;
        }
        write_idx += 1;
    }

    boxes.truncate(write_idx);
}

/// 坐标反算：将模型输出空间的检测框映射回原始视频帧的归一化空间 `[0.0, 1.0]`
///
/// 若预处理为 `Letterbox`，根据原图尺寸与变换布局扣除四周黑边偏移并还原真实比例；
/// 若预处理为 `Resize`，则直接做坐标截断钳位。
pub fn unmap_box(b: &NormBox, mode: &PreprocessMode, src_w: u32, src_h: u32) -> NormBox {
    let mut out = *b;
    match mode {
        PreprocessMode::Letterbox(layout) => {
            let eff_w = if src_w > 0 && layout.scale > 0.0 {
                src_w as f32 * layout.scale
            } else {
                layout.scaled_w as f32
            };
            let eff_h = if src_h > 0 && layout.scale > 0.0 {
                src_h as f32 * layout.scale
            } else {
                layout.scaled_h as f32
            };

            if eff_w <= 0.0 || eff_h <= 0.0 {
                clamp_bbox(&mut out);
                return out;
            }

            // 模型输入像素坐标
            let abs_x = b.x * layout.dst_w as f32;
            let abs_y = b.y * layout.dst_h as f32;
            let abs_w = b.w * layout.dst_w as f32;
            let abs_h = b.h * layout.dst_h as f32;

            // 扣除黑边偏移并以实际缩放有效尺寸归一化到原图 [0.0, 1.0]
            out.x = (abs_x - layout.pad_left as f32) / eff_w;
            out.y = (abs_y - layout.pad_top as f32) / eff_h;
            out.w = abs_w / eff_w;
            out.h = abs_h / eff_h;
        }
        PreprocessMode::Resize => {
            // Resize 模式下模型输出空间归一化坐标线性映射到原图 [0.0, 1.0]
        }
    }

    clamp_bbox(&mut out);
    out
}

/// 将检测框的坐标和尺寸严格截断钳位在 `[0.0, 1.0]` 合法区间内
pub fn clamp_bbox(b: &mut NormBox) {
    b.x = b.x.clamp(0.0, 1.0);
    b.y = b.y.clamp(0.0, 1.0);
    let max_w = (1.0 - b.x).max(0.0);
    let max_h = (1.0 - b.y).max(0.0);
    b.w = b.w.clamp(0.0, max_w);
    b.h = b.h.clamp(0.0, max_h);
}

/// 坐标格式转换：归一化 `[x, y, w, h]` (左上宽高) -> `[x1, y1, x2, y2]` (左上右下角点)
///
/// 输出保证合法钳位在 `[0.0, 1.0]`，且满足 `x2 >= x1` 与 `y2 >= y1`。
#[inline]
pub fn box_xywh_to_xyxy(bbox: [f32; 4]) -> [f32; 4] {
    let x1 = bbox[0].clamp(0.0, 1.0);
    let y1 = bbox[1].clamp(0.0, 1.0);
    let x2 = (bbox[0] + bbox[2].max(0.0)).clamp(x1, 1.0);
    let y2 = (bbox[1] + bbox[3].max(0.0)).clamp(y1, 1.0);
    [x1, y1, x2, y2]
}

/// 坐标格式转换：归一化 `[x1, y1, x2, y2]` (左上右下角点) -> `[x, y, w, h]` (左上宽高)
#[inline]
pub fn box_xyxy_to_xywh(bbox: [f32; 4]) -> [f32; 4] {
    let x1 = bbox[0].clamp(0.0, 1.0);
    let y1 = bbox[1].clamp(0.0, 1.0);
    let x2 = bbox[2].clamp(x1, 1.0);
    let y2 = bbox[3].clamp(y1, 1.0);
    [x1, y1, x2 - x1, y2 - y1]
}

/// 安全计算两个特征向量的余弦相似度 (Cosine Similarity)
///
/// 特性与安全边界：
/// - 要求两向量长度相同且非空；
/// - 自动过滤非有限浮点数（NaN / Inf）；
/// - 任一向量 L2 模长平方小于等于 1e-12 时安全返回 0.0，杜绝除零崩溃；
/// - 输出数值严格截断在 `[-1.0, 1.0]` 闭区间内。
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || a.len() != b.len() {
        return 0.0;
    }

    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;

    for (&va, &vb) in a.iter().zip(b.iter()) {
        if !va.is_finite() || !vb.is_finite() {
            return 0.0;
        }
        dot += va * vb;
        norm_a += va * va;
        norm_b += vb * vb;
    }

    if norm_a <= 1e-12 || norm_b <= 1e-12 {
        return 0.0;
    }

    (dot / (norm_a.sqrt() * norm_b.sqrt())).clamp(-1.0, 1.0)
}

/// 特征向量 L2 范数归一化
///
/// 将任意非零浮点切片投影到单位超球面上。
/// 若包含 NaN / Inf 或模长退化 (<= 1e-12)，返回 `AlgoError::Preprocess`。
pub fn l2_normalize(values: &[f32]) -> Result<Vec<f32>, AlgoError> {
    if values.is_empty() {
        return Err(AlgoError::Preprocess {
            reason: "待归一化向量为空".to_string(),
        });
    }

    let mut norm_sq = 0.0f32;
    for &v in values {
        if !v.is_finite() {
            return Err(AlgoError::Preprocess {
                reason: "向量中存在非有限浮点数 (NaN/Inf)".to_string(),
            });
        }
        norm_sq += v * v;
    }

    if norm_sq <= 1e-12 {
        return Err(AlgoError::Preprocess {
            reason: "向量模长趋近于零，无法进行超球面投影".to_string(),
        });
    }

    let inv_norm = 1.0 / norm_sq.sqrt();
    Ok(values.iter().map(|&v| v * inv_norm).collect())
}

/// 将 f32 特征向量按显式小端序 (Little-Endian) 二进制编码为标准 Base64 字符串
///
/// 与宿主契约完全对齐，杜绝由于跨架构大端字节序导致的特征对账损坏。
pub fn encode_embedding_base64_le(embedding: &[f32]) -> Result<String, AlgoError> {
    if embedding.is_empty() {
        return Err(AlgoError::Preprocess {
            reason: "特征向量为空，拒绝编码".to_string(),
        });
    }

    let mut bytes = Vec::with_capacity(embedding.len() * 4);
    for &val in embedding {
        if !val.is_finite() {
            return Err(AlgoError::Preprocess {
                reason: "特征向量包含非法数值 (NaN/Inf)".to_string(),
            });
        }
        bytes.extend_from_slice(&val.to_le_bytes());
    }

    Ok(base64::prelude::BASE64_STANDARD.encode(&bytes))
}

/// 通用原地 NMS 算子
///
/// 适用于任意业务候选框集合，支持自定义置信度与 IoU 计算逻辑，
/// 内部通过位图掩码实现零多余堆内存分配过滤。
pub fn run_nms_by<T>(
    items: &mut Vec<T>,
    iou_threshold: f32,
    mut score_fn: impl FnMut(&T) -> f32,
    mut iou_fn: impl FnMut(&T, &T) -> f32,
) {
    let n = items.len();
    if n <= 1 {
        return;
    }

    // 按置信度降序排序
    items.sort_by(|a, b| score_fn(b).total_cmp(&score_fn(a)));

    let mut mask = BitMask::new(n);
    let mut write_idx = 0;

    for i in 0..n {
        if mask.is_set(i) {
            continue;
        }

        for j in (i + 1)..n {
            if mask.is_set(j) {
                continue;
            }
            if iou_fn(&items[i], &items[j]) >= iou_threshold {
                mask.set(j);
            }
        }

        if write_idx != i {
            items.swap(write_idx, i);
        }
        write_idx += 1;
    }

    items.truncate(write_idx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cv::types::LetterboxLayout;

    #[test]
    fn test_calculate_iou() {
        let b1 = NormBox::new(0.0, 0.0, 1.0, 1.0, 0.9, 0);
        let b2 = NormBox::new(0.0, 0.0, 1.0, 1.0, 0.8, 0);
        assert!((calculate_iou(&b1, &b2) - 1.0).abs() < 1e-5);

        let b3 = NormBox::new(2.0, 2.0, 1.0, 1.0, 0.7, 0);
        assert_eq!(calculate_iou(&b1, &b3), 0.0);

        // 50% 重叠
        let b4 = NormBox::new(0.0, 0.0, 1.0, 0.5, 0.9, 0);
        let b5 = NormBox::new(0.0, 0.0, 1.0, 1.0, 0.8, 0);
        assert!((calculate_iou(&b4, &b5) - 0.5).abs() < 1e-5);
    }

    #[test]
    fn test_fast_nms_suppression() {
        let mut boxes = vec![
            NormBox::new(0.1, 0.1, 0.2, 0.2, 0.9, 0),
            NormBox::new(0.11, 0.11, 0.2, 0.2, 0.85, 0), // 高度重叠同类，应被抑制
            NormBox::new(0.11, 0.11, 0.2, 0.2, 0.88, 1), // 高度重叠不同类，保留
            NormBox::new(0.6, 0.6, 0.2, 0.2, 0.7, 0),    // 无重叠，保留
        ];

        fast_nms(&mut boxes, 0.5);
        assert_eq!(boxes.len(), 3);
        assert_eq!(boxes[0].confidence, 0.9);
        assert_eq!(boxes[1].confidence, 0.88);
        assert_eq!(boxes[2].confidence, 0.7);
    }

    #[test]
    fn test_unmap_box_letterbox() {
        // 原始帧 1920x1080 目标 640x640: scaled_w=640, scaled_h=360, pad_top=140, pad_left=0
        let layout = LetterboxLayout {
            scale: 640.0 / 1920.0,
            pad_left: 0,
            pad_top: 140,
            dst_w: 640,
            dst_h: 640,
            scaled_w: 640,
            scaled_h: 360,
        };
        let mode = PreprocessMode::Letterbox(layout);

        // 假设模型检测出位于居中内容区域正中央的框：
        // 模型像素: x=0, y=140, w=640, h=360 -> 对应原图满屏
        let norm_in_model = NormBox::new(0.0, 140.0 / 640.0, 1.0, 360.0 / 640.0, 0.9, 0);
        let unmapped = unmap_box(&norm_in_model, &mode, 1920, 1080);

        assert!((unmapped.x - 0.0).abs() < 1e-4);
        assert!((unmapped.y - 0.0).abs() < 1e-4);
        assert!((unmapped.w - 1.0).abs() < 1e-4);
        assert!((unmapped.h - 1.0).abs() < 1e-4);
    }

    #[test]
    fn test_cosine_similarity_properties() {
        let v1 = [1.0, 0.0, 0.0];
        let v2 = [1.0, 0.0, 0.0];
        assert!((cosine_similarity(&v1, &v2) - 1.0).abs() < 1e-6);

        let v3 = [0.0, 1.0, 0.0];
        assert!((cosine_similarity(&v1, &v3) - 0.0).abs() < 1e-6);

        let v4 = [-1.0, 0.0, 0.0];
        assert!((cosine_similarity(&v1, &v4) - (-1.0)).abs() < 1e-6);

        // 零向量与非有限值安全保护
        let zero = [0.0, 0.0, 0.0];
        assert_eq!(cosine_similarity(&v1, &zero), 0.0);
        let nan = [f32::NAN, 0.0, 0.0];
        assert_eq!(cosine_similarity(&v1, &nan), 0.0);
        assert_eq!(cosine_similarity(&v1, &[1.0, 0.0]), 0.0);
    }

    #[test]
    fn test_l2_normalize() {
        let raw = [3.0, 4.0];
        let norm = l2_normalize(&raw).expect("valid norm");
        assert!((norm[0] - 0.6).abs() < 1e-6);
        assert!((norm[1] - 0.8).abs() < 1e-6);

        // 模长平方为 1.0
        let len_sq = norm[0] * norm[0] + norm[1] * norm[1];
        assert!((len_sq - 1.0).abs() < 1e-6);

        // 退化与非法输入
        assert!(l2_normalize(&[0.0, 0.0]).is_err());
        assert!(l2_normalize(&[f32::NAN, 1.0]).is_err());
        assert!(l2_normalize(&[]).is_err());
    }

    #[test]
    fn test_encode_embedding_base64_le() {
        let emb = [1.0f32, 2.0f32];
        let b64 = encode_embedding_base64_le(&emb).expect("valid encode");
        let decoded = base64::prelude::BASE64_STANDARD
            .decode(&b64)
            .expect("valid decode");
        assert_eq!(decoded.len(), 8);

        let v0 = f32::from_le_bytes(decoded[0..4].try_into().expect("valid slice"));
        let v1 = f32::from_le_bytes(decoded[4..8].try_into().expect("valid slice"));
        assert_eq!(v0, 1.0);
        assert_eq!(v1, 2.0);

        assert!(encode_embedding_base64_le(&[]).is_err());
        assert!(encode_embedding_base64_le(&[f32::NAN]).is_err());
    }

    #[test]
    fn test_box_xywh_and_xyxy_conversion() {
        let xywh = [0.1, 0.2, 0.3, 0.4];
        let xyxy = box_xywh_to_xyxy(xywh);
        assert_eq!(xyxy, [0.1, 0.2, 0.4, 0.6]);

        let back = box_xyxy_to_xywh(xyxy);
        assert!((back[0] - xywh[0]).abs() < 1e-6);
        assert!((back[1] - xywh[1]).abs() < 1e-6);
        assert!((back[2] - xywh[2]).abs() < 1e-6);
        assert!((back[3] - xywh[3]).abs() < 1e-6);

        // 越界截断
        let clamped = box_xywh_to_xyxy([-0.5, 0.8, 2.0, 0.5]);
        assert_eq!(clamped, [0.0, 0.8, 1.0, 1.0]);
    }

    #[test]
    fn test_run_nms_by() {
        #[derive(Debug, Clone, PartialEq)]
        struct BoxItem {
            rect: [f32; 4],
            score: f32,
        }

        let mut items = vec![
            BoxItem {
                rect: [0.0, 0.0, 1.0, 1.0],
                score: 0.9,
            },
            BoxItem {
                rect: [0.05, 0.05, 0.95, 0.95],
                score: 0.8,
            },
            BoxItem {
                rect: [0.8, 0.8, 0.2, 0.2],
                score: 0.7,
            },
        ];

        run_nms_by(
            &mut items,
            0.5,
            |b| b.score,
            |a, b| {
                let na = NormBox::new(a.rect[0], a.rect[1], a.rect[2], a.rect[3], a.score, 0);
                let nb = NormBox::new(b.rect[0], b.rect[1], b.rect[2], b.rect[3], b.score, 0);
                calculate_iou(&na, &nb)
            },
        );

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].score, 0.9);
        assert_eq!(items[1].score, 0.7);
    }
}
