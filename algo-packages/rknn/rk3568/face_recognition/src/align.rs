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

/// ArcFace 5 点官方标准坐标 (112x112 画布)
pub const ARC_FACE_BENCHMARK_PTS: [(u32, u32); 5] = [
    (38, 52), // 左眼 (38.29, 51.70)
    (74, 52), // 右眼 (73.53, 51.70)
    (56, 72), // 鼻尖 (56.03, 71.74)
    (42, 92), // 左嘴角 (41.55, 92.37)
    (71, 92), // 右嘴角 (70.73, 92.37)
];

/// 调试落盘对齐人脸：保存 112x112 原始 RGB 图以及叠加 ArcFace 标准模板准星十字的对照图。
///
/// 通过环境变量控制：
/// - `HEIMDALL_DUMP_ALIGNED=1`: 显式开启落盘 (无论是 Debug 还是 Release)
/// - `HEIMDALL_ALIGNED_DIR=/path/to/dir`: 自定义落盘目录，默认为 `/tmp/heimdall_aligned`
pub fn dump_debug_aligned_face(tag: &str, rgb_112: &[u8], quality_score: f32) {
    let enabled = std::env::var("HEIMDALL_DUMP_ALIGNED")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
        || (cfg!(debug_assertions) && std::env::var("HEIMDALL_DUMP_ALIGNED_DISABLE").is_err());

    if !enabled {
        return;
    }

    if rgb_112.len() != (ALIGNED_SIZE * ALIGNED_SIZE * 3) as usize {
        return;
    }

    let dump_dir = std::env::var("HEIMDALL_ALIGNED_DIR")
        .unwrap_or_else(|_| "/tmp/heimdall_aligned".to_string());
    if std::fs::create_dir_all(&dump_dir).is_err() {
        return;
    }

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let q = (quality_score.clamp(0.0, 1.0) * 100.0) as u32;

    // 1. 保存纯净 112x112 原始切片 (用于检查色彩 RGB/BGR、原图清晰度)
    let raw_name = format!("{tag}_q{q}_{ts}_raw.png");
    let raw_path = std::path::Path::new(&dump_dir).join(raw_name);
    let _ = image::save_buffer(
        &raw_path,
        rgb_112,
        ALIGNED_SIZE,
        ALIGNED_SIZE,
        image::ExtendedColorType::Rgb8,
    );

    // 2. 保存叠加 ArcFace 官方标准模板准星 (5 点高亮绿色十字，用于检查关键点对齐误差)
    let mut overlay_rgb = rgb_112.to_vec();
    for &(cx, cy) in &ARC_FACE_BENCHMARK_PTS {
        // 水平线段 (中心点左右各延展 3 像素)
        for dx in -3i32..=3 {
            let px = cx as i32 + dx;
            if (0..ALIGNED_SIZE as i32).contains(&px) {
                let idx = ((cy as usize * ALIGNED_SIZE as usize) + px as usize) * 3;
                overlay_rgb[idx] = 0; // R
                overlay_rgb[idx + 1] = 255; // G (高亮绿)
                overlay_rgb[idx + 2] = 0; // B
            }
        }
        // 垂直线段 (中心点上下各延展 3 像素)
        for dy in -3i32..=3 {
            let py = cy as i32 + dy;
            if (0..ALIGNED_SIZE as i32).contains(&py) {
                let idx = ((py as usize * ALIGNED_SIZE as usize) + cx as usize) * 3;
                overlay_rgb[idx] = 0; // R
                overlay_rgb[idx + 1] = 255; // G (高亮绿)
                overlay_rgb[idx + 2] = 0; // B
            }
        }
    }

    let overlay_name = format!("{tag}_q{q}_{ts}_overlay.png");
    let overlay_path = std::path::Path::new(&dump_dir).join(overlay_name);
    let _ = image::save_buffer(
        &overlay_path,
        &overlay_rgb,
        ALIGNED_SIZE,
        ALIGNED_SIZE,
        image::ExtendedColorType::Rgb8,
    );

    tracing::info!(
        tag,
        quality = quality_score,
        raw = %raw_path.display(),
        overlay = %overlay_path.display(),
        "【DEBUG】112x112 对齐人脸已落盘"
    );

    if std::env::var("HEIMDALL_DUMP_ALIGNED_STDOUT")
        .map(|v| v != "0")
        .unwrap_or(true)
    {
        println!(
            "[ALIGN DEBUG] [{tag}] Q={:.1}% -> Raw: {}, Overlay: {}",
            quality_score * 100.0,
            raw_path.display(),
            overlay_path.display()
        );
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

    #[test]
    fn dump_debug_aligned_face_handles_valid_and_invalid_buffers() {
        // 短 buffer 直接无害返回
        dump_debug_aligned_face("test_invalid", &[1, 2, 3], 0.8);

        // 合法 112x112 RGB 写入临时目录验证
        let temp_dir = std::env::temp_dir().join(format!(
            "heimdall_test_dump_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::env::set_var("HEIMDALL_DUMP_ALIGNED", "1");
        std::env::set_var(
            "HEIMDALL_ALIGNED_DIR",
            temp_dir.to_str().expect("有效 UTF-8 临时路径"),
        );
        std::env::set_var("HEIMDALL_DUMP_ALIGNED_STDOUT", "0");

        let valid_rgb = vec![120u8; 112 * 112 * 3];
        dump_debug_aligned_face("test_valid", &valid_rgb, 0.95);

        let files: Vec<_> = std::fs::read_dir(&temp_dir)
            .expect("读取目录")
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(files.len(), 2, "应生成 raw 和 overlay 两张图片");

        // 清理临时文件与环境变量
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::env::remove_var("HEIMDALL_DUMP_ALIGNED");
        std::env::remove_var("HEIMDALL_ALIGNED_DIR");
        std::env::remove_var("HEIMDALL_DUMP_ALIGNED_STDOUT");
    }
}
