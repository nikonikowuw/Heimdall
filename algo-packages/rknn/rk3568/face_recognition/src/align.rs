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
    const INV_N: f64 = 1.0 / 5.0;

    // 1. 计算源点集与目标点集的质心 (Centroids)
    let (src_sum, dst_sum) = src.iter().zip(dst.iter()).fold(
        ([0.0f64; 2], [0.0f64; 2]),
        |(mut s_acc, mut d_acc), (s, d)| {
            s_acc[0] += s[0] as f64;
            s_acc[1] += s[1] as f64;
            d_acc[0] += d[0];
            d_acc[1] += d[1];
            (s_acc, d_acc)
        },
    );
    let (src_cx, src_cy) = (src_sum[0] * INV_N, src_sum[1] * INV_N);
    let (dst_cx, dst_cy) = (dst_sum[0] * INV_N, dst_sum[1] * INV_N);

    // 2. 中心化并计算方差与协方差
    let (src_var, s_xx, s_xy, s_yx, s_yy) = src.iter().zip(dst.iter()).fold(
        (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64),
        |(var, xx, xy, yx, yy), (s, d)| {
            let sx = s[0] as f64 - src_cx;
            let sy = s[1] as f64 - src_cy;
            let dx = d[0] - dst_cx;
            let dy = d[1] - dst_cy;
            (
                var + sx * sx + sy * sy,
                xx + sx * dx,
                xy + sx * dy,
                yx + sy * dx,
                yy + sy * dy,
            )
        },
    );

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

    let Some(inverse) = inverse_coeffs(matrix.as_ref()) else {
        return Vec::new();
    };
    // 预先计算逆变换仿射矩阵系数 (转换为 f32 向量化单精度浮点)
    let [m00, m01, m02, m10, m11, m12] = inverse.map(|value| value as f32);

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
        let mut out_offset = y * out_size * 3;

        for _ in 0..out_size {
            if sx >= 0.0 && sy >= 0.0 && sx <= max_x && sy <= max_y {
                let clamped_x = sx.clamp(0.0, max_x);
                let clamped_y = sy.clamp(0.0, max_y);
                let x0 = clamped_x as usize;
                let y0 = clamped_y as usize;
                let x1 = (x0 + 1).min(max_x_idx);
                let y1 = (y0 + 1).min(max_y_idx);

                let fx = clamped_x - x0 as f32;
                let fy = clamped_y - y0 as f32;

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

            out_offset += 3;
            sx += m00;
            sy += m10;
        }
    }
    output
}

/// 逆仿射系数 `[m00, m01, m02, m10, m11, m12]`，对应 `source = M⁻¹ · output`：
///   `sx = m00 * x + m01 * y + m02`，`sy = m10 * x + m11 * y + m12`。
///
/// `apply_affine` 的采样与 `aligned_source_bounds` 的采样域推导共用此系数，
/// 避免两处逆变换公式各自漂移；矩阵退化（行列式非有限或接近 0）时返回 `None`。
fn inverse_coeffs(matrix: &[f64; 6]) -> Option<[f64; 6]> {
    let [a, b, tx, c, d, ty] = *matrix;
    let determinant = a * d - b * c;
    if !determinant.is_finite() || determinant.abs() <= 1e-12 {
        return None;
    }
    let inv = 1.0 / determinant;
    Some([
        d * inv,
        -b * inv,
        (-d * tx + b * ty) * inv,
        -c * inv,
        a * inv,
        (c * tx - a * ty) * inv,
    ])
}

/// 对齐输出在源图中的采样域 `[left, top, right, bottom]`（右下为排他边界，已取整）。
///
/// 双线性插值取 `floor(v)` 与 `floor(v) + 1` 两个像素，故边界已外扩 1 像素余量。
/// 返回值可能超出源图边界：调用方按帧尺寸收窄即可，超界像素在整帧路径本就同样填黑。
pub fn aligned_source_bounds(landmarks: &[[f32; 2]; 5], out_size: u32) -> Option<[f32; 4]> {
    if out_size == 0 {
        return None;
    }
    let inverse = inverse_coeffs(&estimate_affine(landmarks, &ARC_FACE_TEMPLATE).0)?;
    let max = f64::from(out_size - 1);
    let mut bounds = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for (out_x, out_y) in [(0.0, 0.0), (max, 0.0), (0.0, max), (max, max)] {
        let sx = inverse[0] * out_x + inverse[1] * out_y + inverse[2];
        let sy = inverse[3] * out_x + inverse[4] * out_y + inverse[5];
        bounds[0] = bounds[0].min(sx - 1.0);
        bounds[1] = bounds[1].min(sy - 1.0);
        bounds[2] = bounds[2].max(sx + 1.0);
        bounds[3] = bounds[3].max(sy + 1.0);
    }
    Some([
        bounds[0].floor() as f32,
        bounds[1].floor() as f32,
        bounds[2].ceil() as f32,
        bounds[3].ceil() as f32,
    ])
}

/// 将已经处于像素坐标系的关键点对齐为 112×112 RGB 图像。
pub fn align_face_pixels(
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
    if landmarks.iter().flatten().any(|value| !value.is_finite()) {
        return Err(AlgoError::Preprocess {
            reason: "人脸关键点包含非有限浮点数".to_string(),
        });
    }
    let matrix = estimate_affine(landmarks, &ARC_FACE_TEMPLATE);
    let aligned = apply_affine(image, width, height, matrix, ALIGNED_SIZE);
    if aligned.len() == (ALIGNED_SIZE * ALIGNED_SIZE * 3) as usize {
        Ok(aligned)
    } else {
        Err(AlgoError::Preprocess {
            reason: "仿射对齐输出为空或尺寸错误".to_string(),
        })
    }
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
        if !point[0].is_finite() || !point[1].is_finite() {
            return Err(AlgoError::Preprocess {
                reason: "人脸关键点包含非有限浮点数".to_string(),
            });
        }
    }
    // 统一尺度判定：若全部关键点均处于归一化相对坐标空间（允许适度浮点越界），则整组按原图尺寸缩放到绝对像素坐标。
    // 避免单个越界关键点判断不一致导致部分点缩放、部分点保持相对值，造成仿射矩阵退化崩溃。
    let is_normalized = source
        .iter()
        .all(|point| (-0.5..=2.0).contains(&point[0]) && (-0.5..=2.0).contains(&point[1]));
    if is_normalized {
        for point in &mut source {
            point[0] *= width as f32;
            point[1] *= height as f32;
        }
    }
    align_face_pixels(image, width, height, &source)
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
