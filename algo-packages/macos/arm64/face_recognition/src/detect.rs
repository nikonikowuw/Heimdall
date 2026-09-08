use algo_sdk::cv::types::PreprocessMode;
use algo_sdk::math::clamp_bbox;

pub const YOLOV5_FACE_FIELDS: usize = 16;
pub const YOLOV8_FACE_FIELDS: usize = 20;
pub const PERSON_FIELDS: usize = 5;

pub use crate::association::PersonCandidate;

/// YOLO 单人脸候选框，坐标在解码阶段仍处于模型输入像素空间。
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

/// 人脸模型输出张量格式枚举。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaceTensorFormat {
    YoloV8,
    YoloV5,
    Unknown,
}

impl FaceTensorFormat {
    #[inline]
    pub fn detect(len: usize) -> Self {
        if len == 0 {
            Self::Unknown
        } else if len == 5040 * YOLOV8_FACE_FIELDS {
            Self::YoloV8
        } else {
            let is_v8 = len.is_multiple_of(YOLOV8_FACE_FIELDS);
            let is_v5 = len.is_multiple_of(YOLOV5_FACE_FIELDS);
            match (is_v8, is_v5) {
                (true, false) => Self::YoloV8,
                (false, true) => Self::YoloV5,
                (true, true) => Self::YoloV8, // 公倍数时优先采用 YOLOv8
                (false, false) => Self::Unknown,
            }
        }
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

/// 解码已经由导出模型展开的 `[1, N, 20]` YOLOv8-face 张量。
///
/// 每行依次为 `cx, cy, w, h, class_confidence, 5 * (x, y, landmark_confidence)`。
pub fn decode_yolov8_face(raw: &[f32], conf_threshold: f32) -> Vec<RawFace> {
    if raw.len() < YOLOV8_FACE_FIELDS || !raw.len().is_multiple_of(YOLOV8_FACE_FIELDS) {
        return Vec::new();
    }
    let threshold = conf_threshold.clamp(0.0, 1.0);
    let mut faces = Vec::with_capacity(raw.len() / YOLOV8_FACE_FIELDS);

    for row in raw.chunks_exact(YOLOV8_FACE_FIELDS) {
        let score = confidence_value(row[4]);
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
        let mut landmark_scores = [0.0; 5];
        let mut valid = true;
        for (index, point) in landmarks.iter_mut().enumerate() {
            let offset = 5 + index * 3;
            point[0] = row[offset];
            point[1] = row[offset + 1];
            landmark_scores[index] = confidence_value(row[offset + 2]);
            if !point[0].is_finite() || !point[1].is_finite() {
                valid = false;
                break;
            }
        }
        if !valid {
            continue;
        }

        faces.push(RawFace {
            bbox: [cx - width * 0.5, cy - height * 0.5, width, height],
            landmarks,
            landmark_scores,
            score,
        });
    }
    faces
}

/// 自适应解码 YOLO 人脸检测张量（自动识别 YOLOv8 20 维或 YOLOv5 16 维格式）。
pub fn decode_face_detections(raw: &[f32], conf_threshold: f32) -> Vec<RawFace> {
    match FaceTensorFormat::detect(raw.len()) {
        FaceTensorFormat::YoloV8 => decode_yolov8_face(raw, conf_threshold),
        FaceTensorFormat::YoloV5 => decode_yolov5_face(raw, conf_threshold),
        FaceTensorFormat::Unknown => Vec::new(),
    }
}

/// 解码 YOLO 人体检测张量（shape `[1, N, 5]`，每行依次为 `cx, cy, w, h, score`）。
pub fn decode_person_detections(raw: &[f32], conf_threshold: f32) -> Vec<PersonCandidate> {
    if raw.len() < PERSON_FIELDS || !raw.len().is_multiple_of(PERSON_FIELDS) {
        return Vec::new();
    }
    let threshold = conf_threshold.clamp(0.0, 1.0);
    let mut persons = Vec::with_capacity(raw.len() / PERSON_FIELDS);

    for row in raw.chunks_exact(PERSON_FIELDS) {
        let score = confidence_value(row[4]);
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

        persons.push(PersonCandidate {
            bbox: [cx - width * 0.5, cy - height * 0.5, width, height],
            score,
        });
    }
    persons
}

/// 对人体候选框执行类别无关 NMS。
pub fn nms_persons(persons: &mut Vec<PersonCandidate>, iou_threshold: f32) {
    if persons.len() <= 1 {
        return;
    }
    persons.sort_by(|left, right| right.score.total_cmp(&left.score));
    let mut kept = Vec::with_capacity(persons.len());
    for p in persons.iter().copied() {
        if kept.iter().all(|prev: &PersonCandidate| {
            let iou = crate::bytetrack::box_iou(&prev.bbox, &p.bbox);
            iou < iou_threshold
        }) {
            kept.push(p);
        }
    }
    *persons = kept;
}

/// 反算人体检测框至 `[0.0, 1.0]` 归一化全图空间。
pub fn unmap_persons_letterbox(
    persons: &mut [PersonCandidate],
    mode: &PreprocessMode,
    orig_width: u32,
    orig_height: u32,
) {
    if orig_width == 0 || orig_height == 0 {
        return;
    }
    let (orig_w, orig_h) = (orig_width as f32, orig_height as f32);
    for person in persons.iter_mut() {
        let x1 = person.bbox[0];
        let y1 = person.bbox[1];
        let x2 = x1 + person.bbox[2];
        let y2 = y1 + person.bbox[3];

        let p1 = map_point([x1, y1], mode, orig_w, orig_h);
        let p2 = map_point([x2, y2], mode, orig_w, orig_h);

        let unmapped_x = p1[0].min(p2[0]);
        let unmapped_y = p1[1].min(p2[1]);
        let unmapped_w = (p2[0] - p1[0]).abs();
        let unmapped_h = (p2[1] - p1[1]).abs();

        let mut normalized = algo_sdk::math::NormBox::new(
            unmapped_x,
            unmapped_y,
            unmapped_w,
            unmapped_h,
            person.score,
            0,
        );
        clamp_bbox(&mut normalized);
        person.bbox = [normalized.x, normalized.y, normalized.w, normalized.h];
    }
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
    fn decodes_yolov8_face_candidates_with_individual_landmark_scores() {
        let mut row = [0.0; YOLOV8_FACE_FIELDS];
        row[0] = 320.0;
        row[1] = 240.0;
        row[2] = 120.0;
        row[3] = 160.0;
        row[4] = 0.88; // cls_conf
        for k in 0..5 {
            row[5 + k * 3] = 300.0 + (k as f32) * 10.0;
            row[5 + k * 3 + 1] = 220.0 + (k as f32) * 10.0;
            row[5 + k * 3 + 2] = 0.95;
        }
        let faces = decode_face_detections(&row, 0.5);
        assert_eq!(faces.len(), 1);
        assert_eq!(faces[0].bbox, [260.0, 160.0, 120.0, 160.0]);
        assert!((faces[0].score - 0.88).abs() < 1e-5);
        assert!((faces[0].landmark_scores[0] - 0.95).abs() < 1e-5);
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
