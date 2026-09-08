/// ArcFace/InsightFace 常用的 112x112 五点对齐模板。
pub const ARC_FACE_TEMPLATE: [[f64; 2]; 5] = [
    [38.2946, 51.6963],
    [73.5318, 51.6963],
    [56.0252, 71.7366],
    [41.5493, 92.3655],
    [70.7299, 92.3655],
];

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

    #[inline]
    pub fn determinant(&self) -> f64 {
        self.0[0] * self.0[4] - self.0[1] * self.0[3]
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
}

impl Default for AffineMatrix2D {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl From<[f64; 6]> for AffineMatrix2D {
    fn from(coeffs: [f64; 6]) -> Self {
        Self(coeffs)
    }
}

impl AsRef<[f64; 6]> for AffineMatrix2D {
    fn as_ref(&self) -> &[f64; 6] {
        &self.0
    }
}

impl<'a> IntoIterator for &'a AffineMatrix2D {
    type Item = &'a f64;
    type IntoIter = std::slice::Iter<'a, f64>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl IntoIterator for AffineMatrix2D {
    type Item = f64;
    type IntoIter = std::array::IntoIter<f64, 6>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

/// 最小二乘估计二维仿射变换矩阵。
pub fn estimate_affine_checked(
    src: &[[f32; 2]; 5],
    dst: &[[f64; 2]; 5],
) -> Result<AffineMatrix2D, &'static str> {
    let mut normal = [[0.0f64; 7]; 6];
    for (source, target) in src.iter().zip(dst.iter()) {
        let x = source[0] as f64;
        let y = source[1] as f64;
        let rows = [
            ([x, y, 1.0, 0.0, 0.0, 0.0], target[0]),
            ([0.0, 0.0, 0.0, x, y, 1.0], target[1]),
        ];
        for (row, value) in rows {
            for column in 0..6 {
                for rhs_column in 0..6 {
                    normal[column][rhs_column] += row[column] * row[rhs_column];
                }
                normal[column][6] += row[column] * value;
            }
        }
    }

    for pivot in 0..6 {
        let mut best = pivot;
        for row in (pivot + 1)..6 {
            if normal[row][pivot].abs() > normal[best][pivot].abs() {
                best = row;
            }
        }
        if normal[best][pivot].abs() <= 1e-10 {
            return Err("关键点几何退化，无法估计仿射矩阵");
        }
        normal.swap(pivot, best);
        let divisor = normal[pivot][pivot];
        for value in normal[pivot].iter_mut().skip(pivot) {
            *value /= divisor;
        }
        for row in 0..6 {
            if row == pivot {
                continue;
            }
            let factor = normal[row][pivot];
            let pivot_row = normal[pivot];
            for (column, value) in normal[row].iter_mut().enumerate().skip(pivot) {
                *value -= factor * pivot_row[column];
            }
        }
    }

    let mut matrix = [0.0; 6];
    for (index, value) in matrix.iter_mut().enumerate() {
        *value = normal[index][6];
    }
    Ok(AffineMatrix2D(matrix))
}

/// 与任务设计保持兼容的非 fallible 包装；退化输入回退到恒等矩阵。
pub fn estimate_affine(src: &[[f32; 2]; 5], dst: &[[f64; 2]; 5]) -> AffineMatrix2D {
    estimate_affine_checked(src, dst).unwrap_or(AffineMatrix2D::IDENTITY)
}

/// 使用双线性插值把 RGB 图像按 `src -> dst` 仿射矩阵采样到正方形输出。
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

                for c in 0..3 {
                    let c00 = p00[c] as f32;
                    let c10 = p10[c] as f32;
                    let c01 = p01[c] as f32;
                    let c11 = p11[c] as f32;

                    let top = c00 + (c10 - c00) * fx;
                    let bottom = c01 + (c11 - c01) * fx;
                    out_pixel[c] = (top + (bottom - top) * fy + 0.5f32) as u8;
                }
            }

            sx += m00;
            sy += m10;
        }
    }
    output
}

/// 将归一化/像素关键点转换为 112x112 对齐图像。
pub fn align_face(
    image: &[u8],
    width: u32,
    height: u32,
    landmarks: &[[f32; 2]; 5],
) -> Result<Vec<u8>, &'static str> {
    if width == 0 || height == 0 {
        return Err("人脸图像尺寸不能为 0");
    }
    let mut source = *landmarks;
    for point in &mut source {
        if point[0].abs() <= 1.0 && point[1].abs() <= 1.0 {
            point[0] *= width as f32;
            point[1] *= height as f32;
        }
    }
    let matrix = estimate_affine(&source, &ARC_FACE_TEMPLATE);
    let aligned = apply_affine(image, width, height, matrix, 112);
    if aligned.len() == 112 * 112 * 3 {
        Ok(aligned)
    } else {
        Err("仿射对齐输出为空或尺寸错误")
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
        for (actual, expected) in matrix.iter().zip(AffineMatrix2D::IDENTITY.iter()) {
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
