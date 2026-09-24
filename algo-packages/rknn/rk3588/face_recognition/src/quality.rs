//! 人脸质量评估：偏航角/俯仰角估计、拓扑合理性检验与综合质量分
//!
//! 核心算法已下沉至 [`algo_sdk::face::quality`]。本模块桥接算法包配置与质量判定。

use crate::config::QualityThresholds;

pub use algo_sdk::face::quality::{
    compute_quality as compute_sdk_quality, estimate_pitch, estimate_yaw,
    is_landmark_geometry_plausible, FaceQuality, QualityConfig,
};

/// 扩展 trait：桥接本地 QualityThresholds
pub trait FaceQualityExt {
    fn accepted(&self, thresholds: &QualityThresholds, min_face_size: u32) -> bool;
}

impl FaceQualityExt for FaceQuality {
    #[inline]
    fn accepted(&self, thresholds: &QualityThresholds, min_face_size: u32) -> bool {
        self.is_accepted(
            thresholds.min_score,
            thresholds.max_yaw,
            thresholds.max_pitch,
            thresholds.max_blur,
            min_face_size,
        )
    }
}

/// 汇总尺寸、姿态、关键点稳定性和模糊代理分数。
pub fn compute_quality(
    landmarks: &[[f32; 2]; 5],
    landmark_scores: &[f32; 5],
    face_width: f32,
    thresholds: &QualityThresholds,
) -> FaceQuality {
    let config = QualityConfig {
        min_face_size: 48,
        max_yaw: thresholds.max_yaw,
        max_pitch: thresholds.max_pitch,
        min_quality_score: thresholds.min_score,
    };
    compute_sdk_quality(landmarks, landmark_scores, face_width, &config)
}

#[cfg(test)]
mod tests {
    use super::*;

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
            [0.20, 0.35],
            [0.55, 0.35],
            [0.48, 0.50],
            [0.25, 0.68],
            [0.50, 0.68],
        ];
        let quality = compute_quality(&landmarks, &[0.8; 5], 20.0, &QualityThresholds::default());
        assert!(quality.yaw.abs() > 20.0);
        assert!(!quality.accepted(&QualityThresholds::default(), 30));
    }

    #[test]
    fn implausible_landmarks_are_rejected() {
        let inverted_eyes = [
            [0.65, 0.35],
            [0.35, 0.35],
            [0.50, 0.50],
            [0.40, 0.68],
            [0.60, 0.68],
        ];
        assert!(!is_landmark_geometry_plausible(&inverted_eyes));
        let q = compute_quality(
            &inverted_eyes,
            &[1.0; 5],
            100.0,
            &QualityThresholds::default(),
        );
        assert_eq!(q.score, 0.0);
    }
}
