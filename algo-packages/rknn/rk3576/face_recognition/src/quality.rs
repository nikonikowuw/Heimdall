//! 人脸姿态评估与质量评估
//!
//! 基础评估算法已下沉至 [`algo_sdk::face::quality`]。本模块保留与本地配置绑定的胶水代码以维持向后兼容性。

use crate::config::QualityThresholds;

pub use algo_sdk::face::quality::{
    estimate_pitch, estimate_yaw, is_landmark_geometry_plausible, FaceQuality, QualityConfig,
};

/// 汇总尺寸、姿态、关键点稳定性和模糊代理分数。
pub fn compute_quality(
    landmarks: &[[f32; 2]; 5],
    landmark_scores: &[f32; 5],
    face_width: f32,
    config: &QualityThresholds,
) -> FaceQuality {
    algo_sdk::face::quality::compute_quality(
        landmarks,
        landmark_scores,
        face_width,
        &QualityConfig {
            min_quality_score: config.min_score,
            max_yaw: config.max_yaw,
            max_pitch: config.max_pitch,
            min_face_size: 0,
        },
    )
}

/// 判定人脸质量是否满足门禁阈值
pub fn is_accepted(
    quality: &FaceQuality,
    thresholds: &QualityThresholds,
    min_face_size: u32,
) -> bool {
    quality.is_accepted(
        thresholds.min_score,
        thresholds.max_yaw,
        thresholds.max_pitch,
        thresholds.max_blur,
        min_face_size,
    )
}

pub trait FaceQualityExt {
    fn accepted(&self, thresholds: &QualityThresholds, min_face_size: u32) -> bool;
}

impl FaceQualityExt for FaceQuality {
    fn accepted(&self, thresholds: &QualityThresholds, min_face_size: u32) -> bool {
        is_accepted(self, thresholds, min_face_size)
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
            [0.50, 0.50],
            [0.40, 0.65],
            [0.60, 0.65],
        ]
    }

    #[test]
    fn frontal_face_has_near_zero_pose() {
        let landmarks = frontal_landmarks();
        let yaw = estimate_yaw(&landmarks);
        let pitch = estimate_pitch(&landmarks);
        assert!(yaw.abs() < 1.0);
        assert!(pitch.abs() < 5.0);
    }

    #[test]
    fn side_pose_and_small_face_are_rejected() {
        let turned_landmarks = [
            [0.45, 0.35],
            [0.55, 0.35],
            [0.54, 0.50],
            [0.48, 0.65],
            [0.53, 0.65],
        ];
        let thresholds = QualityThresholds {
            min_score: 0.55,
            max_yaw: 30.0,
            max_pitch: 30.0,
            max_blur: 0.7,
        };
        let quality = compute_quality(&turned_landmarks, &[0.8; 5], 40.0, &thresholds);
        assert!(!quality.accepted(&thresholds, 80));
    }

    #[test]
    fn implausible_landmarks_are_rejected() {
        let inverted = [
            [0.40, 0.65],
            [0.60, 0.65],
            [0.50, 0.50],
            [0.35, 0.35],
            [0.65, 0.35],
        ];
        assert!(!is_landmark_geometry_plausible(&inverted));

        let thresholds = QualityThresholds {
            min_score: 0.55,
            max_yaw: 30.0,
            max_pitch: 30.0,
            max_blur: 0.7,
        };
        let quality = compute_quality(&inverted, &[0.99; 5], 160.0, &thresholds);
        assert_eq!(quality.score, 0.0);
        assert!(!quality.accepted(&thresholds, 40));
    }
}
