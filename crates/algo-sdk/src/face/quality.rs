//! 人脸质量评估：偏航角/俯仰角估计、拓扑合理性检验与综合质量分

use serde::{Deserialize, Serialize};

/// 人脸质量评估结果，所有连续值均已限制在可传输范围内。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FaceQuality {
    pub score: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub blur: f32,
    pub face_size: u32,
}

/// 质量评估配置参数
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct QualityConfig {
    pub min_face_size: u32,
    pub max_yaw: f32,
    pub max_pitch: f32,
    pub min_quality_score: f32,
}

impl Default for QualityConfig {
    fn default() -> Self {
        Self {
            min_face_size: 48,
            max_yaw: 30.0,
            max_pitch: 30.0,
            min_quality_score: 0.40,
        }
    }
}

impl FaceQuality {
    #[inline]
    pub fn is_accepted(
        &self,
        min_score: f32,
        max_yaw: f32,
        max_pitch: f32,
        max_blur: f32,
        min_face_size: u32,
    ) -> bool {
        self.face_size >= min_face_size
            && self.score >= min_score
            && self.yaw.abs() <= max_yaw
            && self.pitch.abs() <= max_pitch
            && self.blur <= max_blur
    }
}

fn finite_or_zero(value: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        0.0
    }
}

/// 使用眼睛间距和鼻尖水平偏移估算偏航角 (Yaw)。
pub fn estimate_yaw(landmarks: &[[f32; 2]; 5]) -> f32 {
    let eye_dist = (landmarks[1][0] - landmarks[0][0]).hypot(landmarks[1][1] - landmarks[0][1]);
    if eye_dist <= f32::EPSILON {
        return 90.0;
    }
    let eye_center_x = (landmarks[0][0] + landmarks[1][0]) * 0.5;
    let offset = (landmarks[2][0] - eye_center_x) / eye_dist;
    finite_or_zero(offset.atan().to_degrees() * 2.0).clamp(-90.0, 90.0)
}

/// 使用鼻尖相对于眼线和嘴线的纵向位置估算俯仰角 (Pitch)。
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

/// 校验 5 点关键点拓扑几何合理性，拦截倒置、折叠或严重畸变的假人脸。
///
/// 基于 2D 仿射几何不变量（内在手征性叉积与面部纵向投影），支持人脸具有自然侧倾滚转角 (Roll)，
/// 严格拦截左右眼颠倒、上下镜像翻转、五官折叠退化等非生物形态伪影。
pub fn is_landmark_geometry_plausible(landmarks: &[[f32; 2]; 5]) -> bool {
    let [left_eye, right_eye, nose, left_mouth, right_mouth] = *landmarks;

    // 1. 眼向向量与眼距
    let eye_dx = right_eye[0] - left_eye[0];
    let eye_dy = right_eye[1] - left_eye[1];
    let eye_dist = eye_dx.hypot(eye_dy);
    if eye_dist <= 1e-4 {
        return false;
    }

    // 2. 嘴向向量与嘴宽
    let mouth_dx = right_mouth[0] - left_mouth[0];
    let mouth_dy = right_mouth[1] - left_mouth[1];
    let mouth_dist = mouth_dx.hypot(mouth_dy);
    if mouth_dist <= 1e-4 {
        return false;
    }

    // 3. 嘴宽 / 眼宽生物比例合理性 (0.3 ~ 1.8)
    let ratio = mouth_dist / eye_dist;
    if !(0.3..=1.8).contains(&ratio) {
        return false;
    }

    // 4. 眼嘴平行度点积约束 (夹角余弦 >= 0.35，即夹角不超过 ~70 度)
    let dot_eye_mouth = (eye_dx * mouth_dx + eye_dy * mouth_dy) / (eye_dist * mouth_dist);
    if dot_eye_mouth < 0.35 {
        return false;
    }

    // 5. 内在手征性检验 (Chirality Check):
    // 正立人脸中，向量 (左眼 -> 右眼) 顺时针旋转 90 度指向下方面部；
    // 叉积 cross(E, V) = Ex * Vy - Ey * Vx 必须严格大于 0。
    let eye_center = [
        (left_eye[0] + right_eye[0]) * 0.5,
        (left_eye[1] + right_eye[1]) * 0.5,
    ];
    let mouth_center = [
        (left_mouth[0] + right_mouth[0]) * 0.5,
        (left_mouth[1] + right_mouth[1]) * 0.5,
    ];

    let cross_em =
        eye_dx * (mouth_center[1] - eye_center[1]) - eye_dy * (mouth_center[0] - eye_center[0]);
    if cross_em <= 0.0 {
        return false;
    }

    let cross_en = eye_dx * (nose[1] - eye_center[1]) - eye_dy * (nose[0] - eye_center[0]);
    if cross_en <= 0.0 {
        return false;
    }

    // 6. 鼻子纵向投影检验：鼻子必须位于眼连线与嘴连线之间
    let face_height_proj = cross_em / eye_dist;
    let nose_proj = cross_en / eye_dist;
    if face_height_proj <= 1e-4 {
        return false;
    }

    let nose_relative_pos = nose_proj / face_height_proj;
    (0.05..=0.95).contains(&nose_relative_pos)
}

/// 计算人脸的姿态、清晰度与综合质量得分。
pub fn compute_quality(
    landmarks: &[[f32; 2]; 5],
    landmark_scores: &[f32; 5],
    face_width: f32,
    config: &QualityConfig,
) -> FaceQuality {
    if !is_landmark_geometry_plausible(landmarks) {
        return FaceQuality {
            score: 0.0,
            yaw: 90.0,
            pitch: 90.0,
            blur: 1.0,
            face_size: face_width.max(0.0).round() as u32,
        };
    }

    let yaw = estimate_yaw(landmarks);
    let pitch = estimate_pitch(landmarks);
    let average_landmark_score = landmark_scores
        .iter()
        .map(|score| finite_or_zero(*score).clamp(0.0, 1.0))
        .sum::<f32>()
        * 0.2;
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

#[cfg(test)]
mod tests {
    use super::*;

    const FRONTAL_LANDMARKS: [[f32; 2]; 5] = [
        [38.2946, 51.6963],
        [73.5318, 51.6963],
        [56.0252, 71.7366],
        [41.5493, 92.3655],
        [70.7299, 92.3655],
    ];

    #[test]
    fn frontal_face_has_near_zero_pose() {
        let config = QualityConfig::default();
        let scores = [0.99; 5];
        let quality = compute_quality(&FRONTAL_LANDMARKS, &scores, 120.0, &config);
        assert!(quality.yaw.abs() < 5.0);
        assert!(quality.pitch.abs() < 10.0);
        assert!(quality.score > 0.8);
        assert!(quality.blur < 0.1);
    }

    #[test]
    fn side_pose_and_small_face_are_rejected() {
        let config = QualityConfig::default();
        let scores = [0.8; 5];
        let landmarks = [
            [0.20, 0.35],
            [0.55, 0.35],
            [0.48, 0.50],
            [0.25, 0.68],
            [0.50, 0.68],
        ];
        let quality = compute_quality(&landmarks, &scores, 20.0, &config);
        assert!(quality.yaw.abs() > 20.0);
        assert!(!quality.is_accepted(
            config.min_quality_score,
            config.max_yaw,
            config.max_pitch,
            0.5,
            30
        ));
    }

    #[test]
    fn implausible_landmarks_are_rejected() {
        let config = QualityConfig::default();
        let scores = [0.99; 5];

        // 1. 上下颠倒的人脸
        let mut upside_down = FRONTAL_LANDMARKS;
        for pt in &mut upside_down {
            pt[1] = 150.0 - pt[1];
        }
        let q = compute_quality(&upside_down, &scores, 100.0, &config);
        assert_eq!(q.score, 0.0);

        // 2. 左右颠倒的人脸
        let mut mirrored = FRONTAL_LANDMARKS;
        mirrored.swap(0, 1);
        let q = compute_quality(&mirrored, &scores, 100.0, &config);
        assert_eq!(q.score, 0.0);

        // 3. 鼻子跑到嘴巴下方
        let mut nose_below = FRONTAL_LANDMARKS;
        nose_below[2][1] = 110.0;
        let q = compute_quality(&nose_below, &scores, 100.0, &config);
        assert_eq!(q.score, 0.0);
    }
}
