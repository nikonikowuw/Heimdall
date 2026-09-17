//! 人脸仿射对齐与预处理
//!
//! 核心 Umeyama 求解与图像处理算法已下沉至 [`algo_sdk::face::align`]。
//! 本模块保留 macOS CoreImage 所需的 `face_alignment_matrix` 及签名以维持向后兼容性。

pub use algo_sdk::face::align::*;

/// 将归一化/像素关键点转换为 source-top-left -> 112x112 target-top-left 仿射矩阵。
///
/// 矩阵布局为 `[a, b, tx, c, d, ty]`，对应 `x' = ax + by + tx`、
/// `y' = cx + dy + ty`。该矩阵可直接交给设备侧 Core Image 预处理。
pub fn face_alignment_matrix(
    width: u32,
    height: u32,
    landmarks: &[[f32; 2]; 5],
) -> Result<[f64; 6], &'static str> {
    if width == 0 || height == 0 {
        return Err("人脸图像尺寸不能为 0");
    }
    let mut source = *landmarks;
    for point in &mut source {
        if !point[0].is_finite() || !point[1].is_finite() {
            return Err("人脸关键点包含非有限值");
        }
    }
    let is_normalized = source
        .iter()
        .all(|point| (-0.5..=2.0).contains(&point[0]) && (-0.5..=2.0).contains(&point[1]));
    if is_normalized {
        for point in &mut source {
            point[0] *= width as f32;
            point[1] *= height as f32;
        }
    }
    let matrix = estimate_similarity_checked(&source, &ARC_FACE_TEMPLATE)
        .map_err(|_| "关键点几何退化，无法估计仿射矩阵")?;
    let coefficients = matrix.coefficients();
    if coefficients.iter().any(|value| !value.is_finite()) {
        return Err("人脸对齐矩阵包含非有限值");
    }
    Ok(coefficients)
}

/// 将归一化/像素关键点转换为 112x112 对齐图像。
pub fn align_face(
    image: &[u8],
    width: u32,
    height: u32,
    landmarks: &[[f32; 2]; 5],
) -> Result<Vec<u8>, &'static str> {
    let matrix = face_alignment_matrix(width, height, landmarks)?;
    let aligned = apply_affine(image, width, height, AffineMatrix2D(matrix), ALIGNED_SIZE);
    if aligned.len() == (ALIGNED_SIZE * ALIGNED_SIZE * 3) as usize {
        Ok(aligned)
    } else {
        Err("仿射对齐输出为空或尺寸错误")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_constant_is_finite() {
        assert!(ARC_FACE_TEMPLATE
            .iter()
            .flatten()
            .all(|value| value.is_finite()));
    }

    #[test]
    fn identity_points_produce_identity_matrix() {
        let src = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0], [0.5, 0.5]];
        let dst = src.map(|point| [point[0] as f64, point[1] as f64]);
        let matrix = estimate_affine(&src, &dst);
        for (actual, expected) in matrix.0.iter().zip(AffineMatrix2D::IDENTITY.0.iter()) {
            assert!((actual - expected).abs() < 1e-6);
        }
    }

    #[test]
    fn degenerate_landmarks_fall_back_without_panic() {
        let source = [[1.0, 1.0]; 5];
        let matrix = estimate_affine(&source, &ARC_FACE_TEMPLATE);
        assert_eq!(matrix, AffineMatrix2D::IDENTITY);
    }

    #[test]
    fn bilinear_affine_keeps_identity_image_values() {
        let image = vec![10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120];
        let output = apply_affine(&image, 2, 2, AffineMatrix2D::IDENTITY, 2);
        assert_eq!(output, image);
    }

    #[test]
    fn align_face_returns_fixed_size_rgb() {
        let image = vec![128u8; 32 * 32 * 3];
        let landmarks = [
            [0.3, 0.35],
            [0.7, 0.35],
            [0.5, 0.5],
            [0.35, 0.7],
            [0.65, 0.7],
        ];
        let aligned = align_face(&image, 32, 32, &landmarks).expect("对齐应成功");
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
