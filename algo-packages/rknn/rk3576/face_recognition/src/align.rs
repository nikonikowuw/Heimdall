//! 5 点人脸对齐：基于 Umeyama 相似变换与双线性插值的 112×112 RGB 裁剪
//!
//! 使用标准 ArcFace 5 关键点模板，通过最小二乘求解 2D 相似变换矩阵（保角无剪切：缩放、旋转、平移），
//! 并使用双线性插值执行逆仿射采样，确保送入 EdgeFace 特征提取的人脸图像不失真且无混叠。

use algo_sdk::error::AlgoError;

/// ArcFace/InsightFace 常用的 112×112 五点对齐模板。
pub const ARC_FACE_TEMPLATE: [[f64; 2]; 5] = [
    [38.2946, 51.6963],
    [73.5318, 51.6963],
    [56.0252, 71.7366],
    [41.5493, 92.3655],
    [70.7299, 92.3655],
];

/// 目标对齐尺寸
pub const ALIGNED_SIZE: u32 = 112;

/// 2D 仿射变换矩阵，按行优先存储为 `[a, b, tx, c, d, ty]`，对应变换公式：
///   x' = a * x + b * y + tx
///   y' = c * x + d * y + ty
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AffineMatrix2D(pub [f64; 6]);

impl AffineMatrix2D {
    pub const IDENTITY: Self = Self([1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);

    #[inline]
    pub const fn new(coeffs: [f64; 6]) -> Self {
        Self(coeffs)
    }

    #[inline]
    pub fn determinant(&self) -> f64 {
        self.0[0] * self.0[4] - self.0[1] * self.0[3]
    }
}

impl Default for AffineMatrix2D {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl AsRef<[f64; 6]> for AffineMatrix2D {
    fn as_ref(&self) -> &[f64; 6] {
        &self.0
    }
}

/// 使用 Umeyama 算法闭式求解 2D 相似变换矩阵（保角无剪切，4 自由度：等比缩放、旋转、平移）。
pub fn estimate_similarity_checked(
    src: &[[f32; 2]; 5],
    dst: &[[f64; 2]; 5],
) -> Result<AffineMatrix2D, &'static str> {
    let n = src.len() as f64;
    if n == 0.0 {
        return Err("输入点集为空");
    }

    // 1. 计算源点集与目标点集的质心 (Centroids)
    let (mut src_cx, mut src_cy) = (0.0f64, 0.0f64);
    let (mut dst_cx, mut dst_cy) = (0.0f64, 0.0f64);
    for (s, d) in src.iter().zip(dst.iter()) {
        src_cx += s[0] as f64;
        src_cy += s[1] as f64;
        dst_cx += d[0];
        dst_cy += d[1];
    }
    src_cx /= n;
    src_cy /= n;
    dst_cx /= n;
    dst_cy /= n;

    // 2. 中心化并计算方差与协方差
    let mut src_var = 0.0f64;
    let mut s_xx = 0.0f64;
    let mut s_xy = 0.0f64;
    let mut s_yx = 0.0f64;
    let mut s_yy = 0.0f64;

    for (s, d) in src.iter().zip(dst.iter()) {
        let sx = s[0] as f64 - src_cx;
        let sy = s[1] as f64 - src_cy;
        let dx = d[0] - dst_cx;
        let dy = d[1] - dst_cy;

        src_var += sx * sx + sy * sy;
        s_xx += sx * dx;
        s_xy += sx * dy;
        s_yx += sy * dx;
        s_yy += sy * dy;
    }

    if src_var <= 1e-8 || !src_var.is_finite() {
        return Err("关键点几何退化，无法估计仿射矩阵");
    }

    // 3. 求解相似变换参数 a = s*cos(theta), b = s*sin(theta)
    let a = (s_xx + s_yy) / src_var;
    let b = (s_xy - s_yx) / src_var;

    if !a.is_finite() || !b.is_finite() {
        return Err("变换系数计算溢出");
    }

    // 4. 求解平移量
    let tx = dst_cx - (a * src_cx - b * src_cy);
    let ty = dst_cy - (b * src_cx + a * src_cy);

    Ok(AffineMatrix2D([a, -b, tx, b, a, ty]))
}

/// 兼容接口：使用带保角相似变换约束的最小二乘估计二维变换矩阵。
pub fn estimate_affine(src: &[[f32; 2]; 5], dst: &[[f64; 2]; 5]) -> AffineMatrix2D {
    estimate_similarity_checked(src, dst).unwrap_or(AffineMatrix2D::IDENTITY)
}

/// 使用双线性插值把 RGB 图像按 `src -> dst` 仿射矩阵逆采样到正方形输出。
pub fn apply_affine(
    image: &[u8],
    width: u32,
    height: u32,
    matrix: impl AsRef<[f64; 6]>,
    out_size: u32,
) -> Vec<u8> {
    let width = width as usize;
    let height = height as usize;
    let out_size = out_size as usize;
    let source_len = width.checked_mul(height).and_then(|v| v.checked_mul(3));
    let output_len = out_size
        .checked_mul(out_size)
        .and_then(|v| v.checked_mul(3));
    let (Some(source_len), Some(output_len)) = (source_len, output_len) else {
        return Vec::new();
    };
    if width == 0 || height == 0 || out_size == 0 || image.len() < source_len {
        return Vec::new();
    }

    let [a, b, tx, c, d, ty] = *matrix.as_ref();
    let determinant = a * d - b * c;
    if !determinant.is_finite() || determinant.abs() <= 1e-12 {
        return Vec::new();
    }
    let inv = 1.0 / determinant;

    // 预先计算逆变换仿射矩阵系数 (转换为 f32 向量化单精度浮点)
    let m00 = (d * inv) as f32;
    let m01 = (-b * inv) as f32;
    let m02 = ((-d * tx + b * ty) * inv) as f32;

    let m10 = (-c * inv) as f32;
    let m11 = (a * inv) as f32;
    let m12 = ((c * tx - a * ty) * inv) as f32;

    let stride = width * 3;
    let max_x = (width - 1) as f32;
    let max_y = (height - 1) as f32;
    let max_x_idx = width - 1;
    let max_y_idx = height - 1;

    let mut output = vec![0u8; output_len];

    for y in 0..out_size {
        let y_f = y as f32;
        let mut sx = m01 * y_f + m02;
        let mut sy = m11 * y_f + m12;
        let out_row_offset = y * out_size * 3;

        for x in 0..out_size {
            let out_offset = out_row_offset + x * 3;

            if sx >= 0.0 && sy >= 0.0 && sx <= max_x && sy <= max_y {
                let x0 = sx as usize;
                let y0 = sy as usize;
                let x1 = (x0 + 1).min(max_x_idx);
                let y1 = (y0 + 1).min(max_y_idx);

                let fx = sx - x0 as f32;
                let fy = sy - y0 as f32;

                let row0 = y0 * stride;
                let row1 = y1 * stride;
                let col0 = x0 * 3;
                let col1 = x1 * 3;

                let idx00 = row0 + col0;
                let idx10 = row0 + col1;
                let idx01 = row1 + col0;
                let idx11 = row1 + col1;

                let p00 = &image[idx00..idx00 + 3];
                let p10 = &image[idx10..idx10 + 3];
                let p01 = &image[idx01..idx01 + 3];
                let p11 = &image[idx11..idx11 + 3];
                let out_pixel = &mut output[out_offset..out_offset + 3];

                for ch in 0..3 {
                    let c00 = p00[ch] as f32;
                    let c10 = p10[ch] as f32;
                    let c01 = p01[ch] as f32;
                    let c11 = p11[ch] as f32;

                    let top = c00 + (c10 - c00) * fx;
                    let bottom = c01 + (c11 - c01) * fx;
                    out_pixel[ch] = (top + (bottom - top) * fy + 0.5f32) as u8;
                }
            }

            sx += m00;
            sy += m10;
        }
    }
    output
}

/// 将归一化/像素关键点转换为 112×112 对齐人脸 RGB 图像。
///
/// `image`: 原图连续 RGB8 字节流
/// `width`, `height`: 原图尺寸
/// `landmarks`: 5 点关键点坐标（支持 [0, 1] 归一化或像素绝对坐标）
pub fn align_face(
    image: &[u8],
    width: u32,
    height: u32,
    landmarks: &[[f32; 2]; 5],
) -> Result<Vec<u8>, AlgoError> {
    if width == 0 || height == 0 {
        return Err(AlgoError::Preprocess {
            reason: "人脸图像尺寸不能为 0".to_string(),
        });
    }
    let mut source = *landmarks;
    for point in &mut source {
        if point[0].abs() <= 1.0 && point[1].abs() <= 1.0 {
            point[0] *= width as f32;
            point[1] *= height as f32;
        }
    }
    let matrix = estimate_affine(&source, &ARC_FACE_TEMPLATE);
    let aligned = apply_affine(image, width, height, matrix, ALIGNED_SIZE);
    if aligned.len() == (ALIGNED_SIZE * ALIGNED_SIZE * 3) as usize {
        Ok(aligned)
    } else {
        Err(AlgoError::Preprocess {
            reason: "仿射对齐输出为空或尺寸错误".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_points_produce_identity_matrix() {
        let src = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0], [0.5, 0.5]];
        let dst = src.map(|point| [point[0] as f64, point[1] as f64]);
        let matrix = estimate_affine(&src, &dst);
        for (actual, expected) in matrix
            .as_ref()
            .iter()
            .zip(AffineMatrix2D::IDENTITY.0.iter())
        {
            assert!((actual - expected).abs() < 1e-6);
        }
    }

    #[test]
    fn bilinear_affine_keeps_identity_image_values() {
        let image = vec![10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120];
        let output = apply_affine(&image, 2, 2, AffineMatrix2D::IDENTITY, 2);
        assert_eq!(output, image);
    }

    #[test]
    fn degenerate_landmarks_fall_back_without_panic() {
        let source = [[1.0, 1.0]; 5];
        let matrix = estimate_affine(&source, &ARC_FACE_TEMPLATE);
        assert_eq!(matrix, AffineMatrix2D::IDENTITY);
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
    fn matrix_constant_is_finite() {
        assert!(ARC_FACE_TEMPLATE
            .iter()
            .flatten()
            .all(|value| value.is_finite()));
    }
}
