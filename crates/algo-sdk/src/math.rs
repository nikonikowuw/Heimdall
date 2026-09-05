//! 向量化几何后处理 (math)
//! 提供 IoU、NMS、Letterbox 坐标反算去黑边与边界安全截断。

use crate::cv::types::PreprocessMode;

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
pub fn calculate_iou(a: &NormBox, b: &NormBox) -> f32 {
    let a_x2 = a.x + a.w;
    let a_y2 = a.y + a.h;
    let b_x2 = b.x + b.w;
    let b_y2 = b.y + b.h;

    let inter_x1 = a.x.max(b.x);
    let inter_y1 = a.y.max(b.y);
    let inter_x2 = a_x2.min(b_x2);
    let inter_y2 = a_y2.min(b_y2);

    let inter_w = (inter_x2 - inter_x1).max(0.0);
    let inter_h = (inter_y2 - inter_y1).max(0.0);
    let inter_area = inter_w * inter_h;

    let area_a = (a.w * a.h).max(0.0);
    let area_b = (b.w * b.h).max(0.0);
    let union_area = area_a + area_b - inter_area;

    if union_area <= 0.0 {
        0.0
    } else {
        inter_area / union_area
    }
}

/// 高性能非极大值抑制 (NMS)，默认类别相关抑制 (Class-aware)
pub fn fast_nms(boxes: &mut Vec<NormBox>, iou_threshold: f32) {
    if boxes.len() <= 1 {
        return;
    }

    // 按置信度降序排序
    boxes.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));

    let mut keep = Vec::with_capacity(boxes.len());
    let mut suppressed = vec![false; boxes.len()];

    for i in 0..boxes.len() {
        if suppressed[i] {
            continue;
        }
        keep.push(boxes[i]);

        for j in (i + 1)..boxes.len() {
            if suppressed[j] {
                continue;
            }
            // 相同类别才进行抑制
            if boxes[i].class_id == boxes[j].class_id {
                let iou = calculate_iou(&boxes[i], &boxes[j]);
                if iou >= iou_threshold {
                    suppressed[j] = true;
                }
            }
        }
    }

    *boxes = keep;
}

/// 类别无关非极大值抑制 (Class-agnostic NMS)
pub fn fast_nms_agnostic(boxes: &mut Vec<NormBox>, iou_threshold: f32) {
    if boxes.len() <= 1 {
        return;
    }

    boxes.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));

    let mut keep = Vec::with_capacity(boxes.len());
    let mut suppressed = vec![false; boxes.len()];

    for i in 0..boxes.len() {
        if suppressed[i] {
            continue;
        }
        keep.push(boxes[i]);

        for j in (i + 1)..boxes.len() {
            if suppressed[j] {
                continue;
            }
            let iou = calculate_iou(&boxes[i], &boxes[j]);
            if iou >= iou_threshold {
                suppressed[j] = true;
            }
        }
    }

    *boxes = keep;
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
}
