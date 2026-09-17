//! 5 点人脸对齐：基于 Umeyama 相似变换与双线性插值的 112×112 RGB 裁剪
//!
//! 已下沉至 [`algo_sdk::face::align`]。本模块保留 re-export 以维持向后兼容性。

pub use algo_sdk::face::align::{
    align_face_pixels as align_face, apply_affine, estimate_affine, estimate_similarity_checked,
    ALIGNED_SIZE, ARC_FACE_TEMPLATE,
};

pub use algo_sdk::face::{enhance_face_details_inplace, normalize_illumination_inplace};

pub use algo_sdk::face::AffineMatrix2D;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_constant_is_finite() {
        for row in &ARC_FACE_TEMPLATE {
            assert!(row[0].is_finite());
            assert!(row[1].is_finite());
        }
    }

    #[test]
    fn identity_points_produce_identity_matrix() {
        let pts = [
            [38.2946f32, 51.6963],
            [73.5318, 51.6963],
            [56.0252, 71.7366],
            [41.5493, 92.3655],
            [70.7299, 92.3655],
        ];
        let mat = estimate_similarity_checked(&pts, &ARC_FACE_TEMPLATE).expect("invertible");
        assert!((mat.0[0] - 1.0).abs() < 1e-4);
        assert!(mat.0[1].abs() < 1e-4);
        assert!(mat.0[2].abs() < 1e-4);
        assert!(mat.0[3].abs() < 1e-4);
        assert!((mat.0[4] - 1.0).abs() < 1e-4);
        assert!(mat.0[5].abs() < 1e-4);
    }

    #[test]
    fn degenerate_landmarks_fall_back_without_panic() {
        let pts = [[10.0f32, 10.0]; 5];
        assert!(estimate_similarity_checked(&pts, &ARC_FACE_TEMPLATE).is_err());
        let mat = estimate_affine(&pts, &ARC_FACE_TEMPLATE);
        assert!(mat.0.iter().all(|value| value.is_finite()));
    }

    #[test]
    fn bilinear_affine_keeps_identity_image_values() {
        let image = vec![10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120];
        let output = apply_affine(&image, 2, 2, AffineMatrix2D::IDENTITY, 2);
        assert_eq!(output, image);
    }

    #[test]
    fn align_face_returns_fixed_size_rgb() {
        let dummy = vec![100u8; 200 * 200 * 3];
        let landmarks = [
            [60.0f32, 60.0],
            [140.0, 60.0],
            [100.0, 100.0],
            [70.0, 140.0],
            [130.0, 140.0],
        ];
        let aligned = align_face(&dummy, 200, 200, &landmarks).expect("对齐应当正常返回");
        assert_eq!(aligned.len(), 112 * 112 * 3);
    }

    #[test]
    fn test_normalize_illumination_inplace() {
        let mut dark_image = vec![30u8; 112 * 112 * 3];
        normalize_illumination_inplace(&mut dark_image);
        assert!(dark_image[0] > 30);

        let mut bright_image = vec![230u8; 112 * 112 * 3];
        normalize_illumination_inplace(&mut bright_image);
        assert!(bright_image[0] < 230);
    }

    #[test]
    fn test_enhance_face_details_inplace() {
        let mut image = vec![128u8; 112 * 112 * 3];
        image[56 * 112 * 3 + 56 * 3] = 200;
        enhance_face_details_inplace(&mut image);
        assert_eq!(image.len(), 112 * 112 * 3);
    }
}
