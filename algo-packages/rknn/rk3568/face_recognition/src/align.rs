//! 5 点人脸对齐：基于 Umeyama 相似变换与双线性插值的 112×112 RGB 裁剪
//!
//! 核心算法已下沉至 [`algo_sdk::face::align`]。本模块提供重导出及算法包内部调试落盘工具。

use algo_sdk::error::AlgoError;
pub use algo_sdk::face::align::*;

/// ArcFace 官方标准 112×112 对齐五点像素坐标 (圆整用于十字准星高亮)
const ARC_FACE_BENCHMARK_PTS: [(u32, u32); 5] = [
    (38, 52), // left eye
    (74, 52), // right eye
    (56, 72), // nose
    (42, 92), // left mouth
    (71, 92), // right mouth
];

/// 调试落盘是否开启（`HEIMDALL_DUMP_ALIGNED=1`）。
///
/// 常驻采样路径应先判该开关再拼装 `tag`，避免在生产链路上做无条件字符串分配。
#[inline]
pub fn debug_dump_enabled() -> bool {
    std::env::var("HEIMDALL_DUMP_ALIGNED").is_ok_and(|value| value == "1")
}

/// 调试辅助工具：将 112×112 对齐人脸图像落盘到 `/tmp/heimdall_aligned/`
///
/// 文件名为固定的 `{tag}_raw.png` / `{tag}_overlay.png`：调试设施不得无界累积
/// （原生采样路径每帧都会调用本函数，若按时间戳命名会在现场误开时持续写盘），
/// 同一 tag 的后续采样直接覆盖。`tag` 由调用方按「来源 + 航迹」收敛，文件数有界。
pub fn dump_debug_aligned_face(tag: &str, rgb_112: &[u8], quality_score: f32) {
    if !debug_dump_enabled() {
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

    let raw_name = format!("{tag}_raw.png");
    let raw_path = std::path::Path::new(&dump_dir).join(raw_name);
    let _ = image::save_buffer(
        &raw_path,
        rgb_112,
        ALIGNED_SIZE,
        ALIGNED_SIZE,
        image::ExtendedColorType::Rgb8,
    );

    let mut overlay_rgb = rgb_112.to_vec();
    for &(cx, cy) in &ARC_FACE_BENCHMARK_PTS {
        for d in -3i32..=3 {
            for (px, py) in [(cx as i32 + d, cy as i32), (cx as i32, cy as i32 + d)] {
                if (0..ALIGNED_SIZE as i32).contains(&px) && (0..ALIGNED_SIZE as i32).contains(&py)
                {
                    let idx = (py as usize * ALIGNED_SIZE as usize + px as usize) * 3;
                    overlay_rgb[idx..idx + 3].copy_from_slice(&[0, 255, 0]);
                }
            }
        }
    }

    let overlay_name = format!("{tag}_overlay.png");
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
}

/// 对连续 RGB 图像执行水平翻转 (左右镜像)。
///
/// 专用于特征提取阶段的测试时增强 (TTA, Test-Time Augmentation)。
pub fn flip_horizontal_rgb(rgb: &[u8], width: u32, height: u32) -> Result<Vec<u8>, AlgoError> {
    let w = width as usize;
    let h = height as usize;
    let total = w
        .checked_mul(h)
        .and_then(|v| v.checked_mul(3))
        .ok_or(AlgoError::OutOfMemory)?;
    if rgb.len() != total {
        return Err(AlgoError::Preprocess {
            reason: format!(
                "图像水平翻转输入尺寸非法: expected={total}, actual={}",
                rgb.len()
            ),
        });
    }
    let mut flipped = vec![0u8; total];
    let row_len = w * 3;
    for (src_row, dst_row) in rgb
        .chunks_exact(row_len)
        .zip(flipped.chunks_exact_mut(row_len))
    {
        // row_len 恒为 3 的整数倍（w * 3），因此不会丢余数。
        let (src_pixels, _) = src_row.as_chunks::<3>();
        let (dst_pixels, _) = dst_row.as_chunks_mut::<3>();
        for (src_px, dst_px) in src_pixels.iter().rev().zip(dst_pixels.iter_mut()) {
            dst_px.copy_from_slice(src_px);
        }
    }
    Ok(flipped)
}

/// 对 112×112 对齐 RGB 人脸图像执行水平翻转。
#[inline]
pub fn flip_horizontal_112(rgb: &[u8]) -> Result<Vec<u8>, AlgoError> {
    flip_horizontal_rgb(rgb, ALIGNED_SIZE, ALIGNED_SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dump_debug_aligned_face_handles_valid_and_invalid_buffers() {
        dump_debug_aligned_face("test_invalid", &[1, 2, 3], 0.8);

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

        let valid_rgb = vec![120u8; 112 * 112 * 3];
        dump_debug_aligned_face("test_valid", &valid_rgb, 0.95);

        let files: Vec<_> = std::fs::read_dir(&temp_dir)
            .expect("读取目录")
            .filter_map(|e| e.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(files.len(), 2, "应生成 raw 和 overlay 两张图片");
        assert!(files.iter().any(|name| name == "test_valid_raw.png"));
        assert!(files.iter().any(|name| name == "test_valid_overlay.png"));

        // 落盘必须封顶：同一 tag 反复采样只能覆盖，不得新增文件（现场误开调试开关时
        // 否则会按采样频率无界写盘）。
        for quality in [0.10f32, 0.55, 0.99] {
            dump_debug_aligned_face("test_valid", &valid_rgb, quality);
        }
        let file_count = std::fs::read_dir(&temp_dir).expect("读取目录").count();
        assert_eq!(file_count, 2, "重复采样应覆盖同名文件而非累积");

        let _ = std::fs::remove_dir_all(&temp_dir);
        std::env::remove_var("HEIMDALL_DUMP_ALIGNED");
        std::env::remove_var("HEIMDALL_ALIGNED_DIR");
    }

    #[test]
    fn test_flip_horizontal_rgb_correctness() {
        // 构造 3x2 的 RGB 图像
        // 行 0: [R1,G1,B1], [R2,G2,B2], [R3,G3,B3]
        // 行 1: [R4,G4,B4], [R5,G5,B5], [R6,G6,B6]
        #[rustfmt::skip]
        let src = vec![
            10, 11, 12,  20, 21, 22,  30, 31, 32,
            40, 41, 42,  50, 51, 52,  60, 61, 62,
        ];
        let flipped = flip_horizontal_rgb(&src, 3, 2).expect("翻转应成功");
        #[rustfmt::skip]
        let expected = vec![
            30, 31, 32,  20, 21, 22,  10, 11, 12,
            60, 61, 62,  50, 51, 52,  40, 41, 42,
        ];
        assert_eq!(flipped, expected);

        // 再次翻转恢复原状
        let double_flipped = flip_horizontal_rgb(&flipped, 3, 2).expect("第二次翻转应成功");
        assert_eq!(double_flipped, src);
    }

    #[test]
    fn test_flip_horizontal_112_involutory_and_size_check() {
        let invalid = vec![0u8; 100];
        assert!(flip_horizontal_112(&invalid).is_err());

        // 构造具有单侧特征的 112x112 图
        let mut sample = vec![128u8; (ALIGNED_SIZE * ALIGNED_SIZE * 3) as usize];
        // 将第 0 行第 0 像素设为红色
        sample[0] = 255;
        sample[1] = 0;
        sample[2] = 0;

        let flipped = flip_horizontal_112(&sample).expect("112 翻转应成功");
        // 翻转后第 0 行最后像素 (第 111 列) 应为红色
        let last_px_idx = (111 * 3) as usize;
        assert_eq!(&flipped[last_px_idx..last_px_idx + 3], &[255, 0, 0]);
        // 原第 0 像素变为默认值 128
        assert_eq!(&flipped[0..3], &[128, 128, 128]);

        // 对合性质：连续翻转两次等于自身
        let restored = flip_horizontal_112(&flipped).expect("对合翻转");
        assert_eq!(restored, sample);
    }
}
