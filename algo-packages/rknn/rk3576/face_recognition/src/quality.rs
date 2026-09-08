use crate::config::QualityThresholds;

/// 人脸质量评估结果，所有连续值均已限制在可传输范围内。
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct FaceQuality {
    pub score: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub blur: f32,
    pub face_size: u32,
}

fn finite_or_zero(value: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        0.0
    }
}

/// 使用眼睛间距和鼻尖水平偏移估算偏航角。
pub fn estimate_yaw(landmarks: &[[f32; 2]; 5]) -> f32 {
    let eye_dist = (landmarks[1][0] - landmarks[0][0]).hypot(landmarks[1][1] - landmarks[0][1]);
    if eye_dist <= f32::EPSILON {
        return 90.0;
    }
    let eye_center = [
        (landmarks[0][0] + landmarks[1][0]) * 0.5,
        (landmarks[0][1] + landmarks[1][1]) * 0.5,
    ];
    let offset = (landmarks[2][0] - eye_center[0]) / eye_dist;
    finite_or_zero(offset.atan().to_degrees() * 2.0).clamp(-90.0, 90.0)
}

/// 使用鼻尖相对于眼线和嘴线的纵向位置估算俯仰角。
pub fn estimate_pitch(landmarks: &[[f32; 2]; 5]) -> f32 {
    let eye_y = (landmarks[0][1] + landmarks[1][1]) * 0.5;
    let mouth_y = (landmarks[3][1] + landmarks[4][1]) * 0.5;
    let face_axis = mouth_y - eye_y;
    if face_axis.abs() <= f32::EPSILON {
        return 90.0;
    }
    let expected_nose_y = eye_y + face_axis * 0.5;
    let offset = (landmarks[2][1] - expected_nose_y) / face_axis;
    finite_or_zero(offset.atan().to_degrees() * 2.0).clamp(-90.0, 90.0)
}

/// 汇总尺寸、姿态、关键点稳定性和模糊代理分数。
pub fn compute_quality(
    landmarks: &[[f32; 2]; 5],
    landmark_scores: &[f32; 5],
    face_width: f32,
    config: &QualityThresholds,
) -> FaceQuality {
    let yaw = estimate_yaw(landmarks);
    let pitch = estimate_pitch(landmarks);
    let average_landmark_score = landmark_scores
        .iter()
        .map(|score| finite_or_zero(*score).clamp(0.0, 1.0))
        .sum::<f32>()
        / landmark_scores.len() as f32;
    // YOLOv5-face 没有独立 Laplacian 输出，关键点置信度是 fast path 上的低成本模糊代理。
    let blur = (1.0 - average_landmark_score).clamp(0.0, 1.0);
    let size = face_width.max(0.0);
    let size_quality = (size / 120.0).clamp(0.0, 1.0);
    let yaw_quality = 1.0 - (yaw.abs() / config.max_yaw.max(1.0)).clamp(0.0, 1.0);
    let pitch_quality = 1.0 - (pitch.abs() / config.max_pitch.max(1.0)).clamp(0.0, 1.0);
    let pose_quality = (yaw_quality + pitch_quality) * 0.5;
    let sharpness_quality = 1.0 - blur;
    let score = (size_quality * 0.35
        + pose_quality * 0.3
        + average_landmark_score * 0.2
        + sharpness_quality * 0.15)
        .clamp(0.0, 1.0);

    FaceQuality {
        score,
        yaw,
        pitch,
        blur,
        face_size: size.round().clamp(0.0, u32::MAX as f32) as u32,
    }
}

impl FaceQuality {
    pub fn accepted(self, thresholds: &QualityThresholds, min_face_size: u32) -> bool {
        self.face_size >= min_face_size
            && self.score >= thresholds.min_score
            && self.yaw.abs() <= thresholds.max_yaw
            && self.pitch.abs() <= thresholds.max_pitch
            && self.blur <= thresholds.max_blur
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::QualityThresholds;

    fn frontal_landmarks() -> [[f32; 2]; 5] {
        [
            [0.35, 0.35],
            [0.65, 0.35],
            [0.50, 0.515],
            [0.40, 0.68],
            [0.60, 0.68],
        ]
    }

    #[test]
    fn frontal_face_has_near_zero_pose() {
        let landmarks = frontal_landmarks();
        assert!(estimate_yaw(&landmarks).abs() < 1e-5);
        assert!(estimate_pitch(&landmarks).abs() < 1e-5);
        let quality = compute_quality(&landmarks, &[1.0; 5], 140.0, &QualityThresholds::default());
        assert!(quality.score > 0.8);
        assert!(quality.accepted(&QualityThresholds::default(), 30));
    }

    #[test]
    fn side_pose_and_small_face_are_rejected() {
        let landmarks = [
            [0.30, 0.35],
            [0.60, 0.35],
            [0.78, 0.50],
            [0.38, 0.68],
            [0.58, 0.68],
        ];
        let quality = compute_quality(&landmarks, &[0.8; 5], 20.0, &QualityThresholds::default());
        assert!(quality.yaw > 20.0);
        assert!(!quality.accepted(&QualityThresholds::default(), 30));
    }
}
