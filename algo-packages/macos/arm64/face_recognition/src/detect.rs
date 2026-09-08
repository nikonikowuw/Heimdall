use algo_sdk::cv::types::PreprocessMode;
use algo_sdk::math::clamp_bbox;

pub const YOLOV5_FACE_FIELDS: usize = 16;

/// YOLOv5-face 单候选框，坐标在解码阶段仍处于模型输入像素空间。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawFace {
    /// `[x, y, width, height]`，左上角坐标格式。
    pub bbox: [f32; 4],
    pub landmarks: [[f32; 2]; 5],
    pub landmark_scores: [f32; 5],
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

fn confidence_value(value: f32) -> f32 {
    if !value.is_finite() {
        return 0.0;
    }
    if (0.0..=1.0).contains(&value) {
        value
    } else {
        1.0 / (1.0 + (-value).exp())
    }
}

/// 解码已经由导出模型展开的 `[1, N, 16]` YOLOv5-face 张量。
///
/// 每行依次为 `cx, cy, w, h, objectness, class_confidence, 5 * (x, y)`。
/// 关键点分数在该模型格式中没有单独输出，因此先以检测分数作为保守代理。
pub fn decode_yolov5_face(raw: &[f32], conf_threshold: f32) -> Vec<RawFace> {
    if raw.len() < YOLOV5_FACE_FIELDS || !raw.len().is_multiple_of(YOLOV5_FACE_FIELDS) {
        return Vec::new();
    }
    let threshold = conf_threshold.clamp(0.0, 1.0);
    let mut faces = Vec::with_capacity(raw.len() / YOLOV5_FACE_FIELDS);

    for row in raw.chunks_exact(YOLOV5_FACE_FIELDS) {
        let score = confidence_value(row[4]) * confidence_value(row[5]);
        if score < threshold {
            continue;
        }
        let (cx, cy, width, height) = (row[0], row[1], row[2], row[3]);
        if !cx.is_finite()
            || !cy.is_finite()
            || !width.is_finite()
            || !height.is_finite()
            || width <= 0.0
            || height <= 0.0
        {
            continue;
        }

        let mut landmarks = [[0.0; 2]; 5];
        for (index, point) in landmarks.iter_mut().enumerate() {
            let offset = 6 + index * 2;
            point[0] = row[offset];
            point[1] = row[offset + 1];
            if !point[0].is_finite() || !point[1].is_finite() {
                landmarks = [[0.0; 2]; 5];
                break;
            }
        }

        faces.push(RawFace {
            bbox: [cx - width * 0.5, cy - height * 0.5, width, height],
            landmarks,
            landmark_scores: [score; 5],
            score,
        });
    }
    faces
}

/// 对同一张图的人脸候选执行类别无关 NMS。
pub fn nms(faces: &mut Vec<RawFace>, iou_threshold: f32) {
    if faces.len() <= 1 {
        return;
    }
    faces.sort_by(|left, right| right.score.total_cmp(&left.score));
    let mut kept = Vec::with_capacity(faces.len());
    for face in faces.iter().copied() {
        if kept
            .iter()
            .all(|previous: &RawFace| previous.iou(&face) < iou_threshold)
        {
            kept.push(face);
        }
    }
    *faces = kept;
}

fn model_coordinate(value: f32, extent: f32) -> f32 {
    if value.abs() <= 1.0 {
        value * extent
    } else {
        value
    }
}

fn map_point(point: [f32; 2], mode: &PreprocessMode, orig_w: f32, orig_h: f32) -> [f32; 2] {
    let (dst_w, dst_h, pad_left, pad_top, effective_w, effective_h) = match mode {
        PreprocessMode::Letterbox(layout) => (
            layout.dst_w as f32,
            layout.dst_h as f32,
            layout.pad_left as f32,
            layout.pad_top as f32,
            if layout.scale > 0.0 {
                orig_w * layout.scale
            } else {
                layout.scaled_w as f32
            },
            if layout.scale > 0.0 {
                orig_h * layout.scale
            } else {
                layout.scaled_h as f32
            },
        ),
        PreprocessMode::Resize => (1.0, 1.0, 0.0, 0.0, orig_w, orig_h),
    };

    let x = model_coordinate(point[0], dst_w);
    let y = model_coordinate(point[1], dst_h);
    if matches!(mode, PreprocessMode::Resize) {
        [x / dst_w.max(1.0), y / dst_h.max(1.0)]
    } else {
        [
            ((x - pad_left) / effective_w.max(1.0)).clamp(0.0, 1.0),
            ((y - pad_top) / effective_h.max(1.0)).clamp(0.0, 1.0),
        ]
    }
}

/// 把模型输入坐标扣除 Letterbox padding，转换为原图归一化坐标。
pub fn unmap_letterbox(faces: &mut [RawFace], mode: &PreprocessMode, orig_w: u32, orig_h: u32) {
    if orig_w == 0 || orig_h == 0 {
        faces.fill(RawFace {
            bbox: [0.0; 4],
            landmarks: [[0.0; 2]; 5],
            landmark_scores: [0.0; 5],
            score: 0.0,
        });
        return;
    }
    let width = orig_w as f32;
    let height = orig_h as f32;
    for face in faces {
        let x1 = face.bbox[0];
        let y1 = face.bbox[1];
        let x2 = x1 + face.bbox[2];
        let y2 = y1 + face.bbox[3];
        let p1 = map_point([x1, y1], mode, width, height);
        let p2 = map_point([x2, y2], mode, width, height);
        face.bbox = [
            p1[0],
            p1[1],
            (p2[0] - p1[0]).max(0.0),
            (p2[1] - p1[1]).max(0.0),
        ];
        for point in &mut face.landmarks {
            *point = map_point(*point, mode, width, height);
        }
        let mut normalized = algo_sdk::math::NormBox::new(
            face.bbox[0],
            face.bbox[1],
            face.bbox[2],
            face.bbox[3],
            face.score,
            0,
        );
        clamp_bbox(&mut normalized);
        face.bbox = [normalized.x, normalized.y, normalized.w, normalized.h];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use algo_sdk::cv::types::{LetterboxLayout, PreprocessMode};

    #[test]
    fn decodes_and_filters_face_candidates() {
        let mut row = [0.0; YOLOV5_FACE_FIELDS];
        row[0] = 320.0;
        row[1] = 240.0;
        row[2] = 120.0;
        row[3] = 160.0;
        row[4] = 0.9;
        row[5] = 0.95;
        row[6..16].fill(1.0);
        let faces = decode_yolov5_face(&row, 0.5);
        assert_eq!(faces.len(), 1);
        assert_eq!(faces[0].bbox, [260.0, 160.0, 120.0, 160.0]);
        assert!((faces[0].score - 0.855).abs() < 1e-5);
    }

    #[test]
    fn nms_removes_overlapping_lower_score_face() {
        let mut faces = vec![
            RawFace {
                bbox: [10.0, 10.0, 100.0, 100.0],
                landmarks: [[0.0; 2]; 5],
                landmark_scores: [1.0; 5],
                score: 0.9,
            },
            RawFace {
                bbox: [12.0, 12.0, 100.0, 100.0],
                landmarks: [[0.0; 2]; 5],
                landmark_scores: [1.0; 5],
                score: 0.8,
            },
        ];
        nms(&mut faces, 0.5);
        assert_eq!(faces.len(), 1);
        assert_eq!(faces[0].score, 0.9);
    }

    #[test]
    fn unmaps_non_square_letterbox_and_clamps_coordinates() {
        let mode = PreprocessMode::Letterbox(LetterboxLayout {
            scale: 0.5,
            pad_left: 0,
            pad_top: 80,
            dst_w: 640,
            dst_h: 640,
            scaled_w: 640,
            scaled_h: 360,
        });
        let mut faces = vec![RawFace {
            bbox: [0.0, 80.0, 640.0, 360.0],
            landmarks: [[320.0, 260.0]; 5],
            landmark_scores: [1.0; 5],
            score: 0.9,
        }];
        unmap_letterbox(&mut faces, &mode, 1280, 720);
        assert_eq!(faces[0].bbox, [0.0, 0.0, 1.0, 1.0]);
        assert!((faces[0].landmarks[0][0] - 0.5).abs() < 1e-5);
        assert!((faces[0].landmarks[0][1] - 0.5).abs() < 1e-5);
    }
}
