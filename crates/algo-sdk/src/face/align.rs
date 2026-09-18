//! 5 点人脸对齐：基于 Umeyama 相似变换与双线性插值的 112×112 RGB 裁剪
//!
//! 使用标准 ArcFace 5 关键点模板，通过最小二乘求解 2D 相似变换矩阵（保角无剪切：缩放、旋转、平移），
//! 并使用双线性插值执行逆仿射采样，确保送入 EdgeFace 特征提取的人脸图像不失真且无混叠。

use crate::error::AlgoError;

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
    pub const fn coefficients(&self) -> [f64; 6] {
        self.0
    }

    #[inline]
    pub fn iter(&self) -> std::slice::Iter<'_, f64> {
        self.0.iter()
    }

    pub fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        let [a, b, tx, c, d, ty] = self.0;
        let x_f = x as f64;
        let y_f = y as f64;
        (
            (a * x_f + b * y_f + tx) as f32,
            (c * x_f + d * y_f + ty) as f32,
        )
    }

    pub fn invert(&self) -> Option<Self> {
        let [a, b, tx, c, d, ty] = self.0;
        let det = self.determinant();
        if !det.is_finite() || det.abs() <= 1e-12 {
            return None;
        }
        let inv = 1.0 / det;
        Some(Self([
            d * inv,
            -b * inv,
            (-d * tx + b * ty) * inv,
            -c * inv,
            a * inv,
            (c * tx - a * ty) * inv,
        ]))
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

    if !src_var.is_finite() || src_var < 1e-6 {
        return Err("源关键点退化（方差接近零），无法确定尺度");
    }

    let a = s_xx + s_yy;
    let b = s_xy - s_yx;
    let det = a * a + b * b;
    if !det.is_finite() || det < 1e-12 {
        return Err("源点集与目标点集无显著相关性");
    }

    let alpha = a / src_var;
    let beta = b / src_var;

    let tx = dst_cx - (alpha * src_cx - beta * src_cy);
    let ty = dst_cy - (beta * src_cx + alpha * src_cy);

    if !alpha.is_finite() || !beta.is_finite() || !tx.is_finite() || !ty.is_finite() {
        return Err("估计得到的仿射参数包含非有限浮点数");
    }

    Ok(AffineMatrix2D([alpha, -beta, tx, beta, alpha, ty]))
}

/// 兼容接口：使用带保角相似变换约束的最小二乘估计二维变换矩阵。
pub fn estimate_affine(src: &[[f32; 2]; 5], dst: &[[f64; 2]; 5]) -> AffineMatrix2D {
    estimate_similarity_checked(src, dst).unwrap_or(AffineMatrix2D::IDENTITY)
}

/// `apply_affine` 的采样与 `aligned_source_bounds` 的采样域推导共用此系数。
pub fn inverse_coeffs(matrix: &[f64; 6]) -> Option<[f64; 6]> {
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

                let w00 = (1.0 - fx) * (1.0 - fy);
                let w10 = fx * (1.0 - fy);
                let w01 = (1.0 - fx) * fy;
                let w11 = fx * fy;

                for c in 0..3 {
                    let v00 = image[idx00 + c] as f32;
                    let v10 = image[idx10 + c] as f32;
                    let v01 = image[idx01 + c] as f32;
                    let v11 = image[idx11 + c] as f32;
                    let val = (w00 * v00 + w10 * v10 + w01 * v01 + w11 * v11).round() as u8;
                    output[out_offset + c] = val;
                }
            }

            sx += m00;
            sy += m10;
            out_offset += 3;
        }
    }

    output
}

/// 对齐输出在源图中的采样域 `[left, top, right, bottom]`（右下为排他边界，已取整）。
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

/// 对 112×112 RGB 人脸裁剪执行自适应光照与对比度归一化。
pub fn normalize_illumination_inplace(rgb: &mut [u8]) {
    const PIXEL_COUNT: usize = 112 * 112;
    if rgb.len() != PIXEL_COUNT * 3 {
        return;
    }

    let mut luma_sum = 0u64;
    let (pixels, _) = rgb.as_chunks::<3>();
    for px in pixels {
        let y = 77 * px[0] as u32 + 150 * px[1] as u32 + 29 * px[2] as u32;
        luma_sum += (y >> 8) as u64;
    }
    let avg_luma = (luma_sum / PIXEL_COUNT as u64) as f32;

    if (70.0..=185.0).contains(&avg_luma) {
        return;
    }

    let gamma = if avg_luma < 70.0 {
        let t = ((avg_luma - 20.0) / 50.0).clamp(0.0, 1.0);
        0.60 + 0.40 * t
    } else {
        let t = ((avg_luma - 185.0) / 50.0).clamp(0.0, 1.0);
        1.0 + 0.40 * t
    };

    let mut lut = [0u8; 256];
    for (i, entry) in lut.iter_mut().enumerate() {
        let normalized = i as f32 / 255.0;
        let corrected = normalized.powf(gamma);
        *entry = (corrected * 255.0).round().clamp(0.0, 255.0) as u8;
    }

    for val in rgb.iter_mut() {
        *val = lut[*val as usize];
    }
}

/// 对 112×112 RGB 人脸图像执行五官细节自适应反锐化。
pub fn enhance_face_details_inplace(rgb: &mut [u8]) {
    const W: usize = 112;
    const H: usize = 112;
    const TOTAL_BYTES: usize = W * H * 3;
    if rgb.len() != TOTAL_BYTES {
        return;
    }

    let mut blurred = [0u8; TOTAL_BYTES];

    for y in 0..H {
        let ym1 = y.saturating_sub(1);
        let yp1 = (y + 1).min(H - 1);
        let row_curr = y * W * 3;
        let row_prev = ym1 * W * 3;
        let row_next = yp1 * W * 3;

        for x in 0..W {
            let xm1 = x.saturating_sub(1);
            let xp1 = (x + 1).min(W - 1);

            let c_x = x * 3;
            let l_x = xm1 * 3;
            let r_x = xp1 * 3;

            for ch in 0..3 {
                let p_tl = rgb[row_prev + l_x + ch] as u32;
                let p_tc = rgb[row_prev + c_x + ch] as u32;
                let p_tr = rgb[row_prev + r_x + ch] as u32;
                let p_ml = rgb[row_curr + l_x + ch] as u32;
                let p_mc = rgb[row_curr + c_x + ch] as u32;
                let p_mr = rgb[row_curr + r_x + ch] as u32;
                let p_bl = rgb[row_next + l_x + ch] as u32;
                let p_bc = rgb[row_next + c_x + ch] as u32;
                let p_br = rgb[row_next + r_x + ch] as u32;

                let sum =
                    (p_tl + p_tr + p_bl + p_br) + ((p_tc + p_ml + p_mr + p_bc) << 1) + (p_mc << 2);

                blurred[row_curr + c_x + ch] = ((sum + 8) >> 4) as u8;
            }
        }
    }

    const MIN_DIFF: i32 = 4;
    const MAX_BOOST: i32 = 24;

    for (orig, blur) in rgb.iter_mut().zip(blurred.iter()) {
        let diff = *orig as i32 - *blur as i32;
        if diff.abs() > MIN_DIFF {
            let sign = diff.signum();
            let mag = diff.abs();
            let boost = ((mag - MIN_DIFF) * 3 / 8).min(MAX_BOOST);
            let enhanced = *orig as i32 + sign * boost;
            *orig = enhanced.clamp(0, 255) as u8;
        }
    }
}

/// 将处于像素坐标系的关键点对齐为 112×112 RGB 图像。
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
    let mut aligned = apply_affine(image, width, height, matrix, ALIGNED_SIZE);
    if aligned.len() == (ALIGNED_SIZE * ALIGNED_SIZE * 3) as usize {
        normalize_illumination_inplace(&mut aligned);
        enhance_face_details_inplace(&mut aligned);
        Ok(aligned)
    } else {
        Err(AlgoError::Preprocess {
            reason: "人脸对齐几何求解退化".to_string(),
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
        let mat = estimate_similarity_checked(&src, &dst).expect("应成功求解");
        let coeffs = mat.as_ref();
        assert!((coeffs[0] - 1.0).abs() < 1e-4, "alpha 接近 1.0");
        assert!(coeffs[1].abs() < 1e-4, "beta 接近 0.0");
        assert!(coeffs[2].abs() < 1e-3, "tx 接近 0.0");
        assert!(coeffs[3].abs() < 1e-4, "beta 接近 0.0");
        assert!((coeffs[4] - 1.0).abs() < 1e-4, "alpha 接近 1.0");
        assert!(coeffs[5].abs() < 1e-3, "ty 接近 0.0");
    }

    #[test]
    fn degenerate_landmarks_fall_back_without_panic() {
        let collapsed = [[0.0f32; 2]; 5];
        assert!(estimate_similarity_checked(&collapsed, &ARC_FACE_TEMPLATE).is_err());
        let fallback = estimate_affine(&collapsed, &ARC_FACE_TEMPLATE);
        assert_eq!(fallback, AffineMatrix2D::IDENTITY);
    }

    #[test]
    fn bilinear_affine_keeps_identity_image_values() {
        let width = 4u32;
        let height = 4u32;
        let mut image = vec![0u8; (width * height * 3) as usize];
        let (pixels, _) = image.as_chunks_mut::<3>();
        for (i, px) in pixels.iter_mut().enumerate() {
            let val = (i * 15).min(255) as u8;
            px[0] = val;
            px[1] = val;
            px[2] = val;
        }

        let sampled = apply_affine(&image, width, height, AffineMatrix2D::IDENTITY, width);
        assert_eq!(sampled, image);
    }

    #[test]
    fn align_face_returns_fixed_size_rgb() {
        let image = vec![128u8; 32 * 32 * 3];
        let landmarks: [[f32; 2]; 5] = [
            [0.35, 0.35],
            [0.65, 0.35],
            [0.5, 0.5],
            [0.35, 0.7],
            [0.65, 0.7],
        ];
        let aligned = align_face_pixels(&image, 32, 32, &landmarks).expect("对齐应成功");
        assert_eq!(aligned.len(), 112 * 112 * 3);
    }

    #[test]
    fn matrix_constant_is_finite() {
        assert!(ARC_FACE_TEMPLATE
            .iter()
            .flatten()
            .all(|value| value.is_finite()));
    }

    #[test]
    fn test_normalize_illumination_inplace() {
        let mut normal_img = vec![120u8; 112 * 112 * 3];
        let original = normal_img.clone();
        normalize_illumination_inplace(&mut normal_img);
        assert_eq!(normal_img, original);

        let mut dark_img = vec![40u8; 112 * 112 * 3];
        normalize_illumination_inplace(&mut dark_img);
        assert!(dark_img[0] > 40, "暗部图像应被自适应 Gamma 提亮");

        let mut bright_img = vec![220u8; 112 * 112 * 3];
        normalize_illumination_inplace(&mut bright_img);
        assert!(bright_img[0] < 220, "过曝图像应被自适应 Gamma 压制");
    }

    #[test]
    fn test_enhance_face_details_inplace() {
        let mut flat = vec![128u8; 112 * 112 * 3];
        let original = flat.clone();
        enhance_face_details_inplace(&mut flat);
        assert_eq!(flat, original);

        let mut subtle = vec![128u8; 112 * 112 * 3];
        subtle[56 * 112 * 3 + 56 * 3] = 130;
        let subtle_orig = subtle.clone();
        enhance_face_details_inplace(&mut subtle);
        assert_eq!(subtle, subtle_orig);
    }
}
