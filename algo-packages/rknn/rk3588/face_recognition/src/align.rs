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

/// 调整 112×112 对齐人脸图像亮度（乘法增益并截断至 [0, 255]）。
pub fn adjust_brightness_112(chip: &[u8], factor: f32) -> Vec<u8> {
    let mut out = chip.to_vec();
    for p in &mut out {
        *p = ((*p as f32 * factor).round() as u32).min(255) as u8;
    }
    out
}

/// 对 112×112 对齐人脸图像进行中心双线性缩放重采样（多尺度扰动）。
pub fn crop_and_resize_chip_112(chip: &[u8], scale: f32) -> Vec<u8> {
    const W: usize = ALIGNED_SIZE as usize;
    const H: usize = ALIGNED_SIZE as usize;
    if chip.len() != W * H * 3 {
        return chip.to_vec();
    }
    let mut out = vec![0u8; W * H * 3];
    let center_x = (W as f32 - 1.0) * 0.5;
    let center_y = (H as f32 - 1.0) * 0.5;

    for y in 0..H {
        for x in 0..W {
            let src_x = center_x + (x as f32 - center_x) * scale;
            let src_y = center_y + (y as f32 - center_y) * scale;

            if src_x < 0.0 || src_x > (W - 1) as f32 || src_y < 0.0 || src_y > (H - 1) as f32 {
                let clamp_x = src_x.clamp(0.0, (W - 1) as f32) as usize;
                let clamp_y = src_y.clamp(0.0, (H - 1) as f32) as usize;
                let src_idx = (clamp_y * W + clamp_x) * 3;
                let dst_idx = (y * W + x) * 3;
                out[dst_idx..dst_idx + 3].copy_from_slice(&chip[src_idx..src_idx + 3]);
            } else {
                let x0 = src_x.floor() as usize;
                let y0 = src_y.floor() as usize;
                let x1 = (x0 + 1).min(W - 1);
                let y1 = (y0 + 1).min(H - 1);

                let fx = src_x - x0 as f32;
                let fy = src_y - y0 as f32;

                let dst_idx = (y * W + x) * 3;
                for c in 0..3 {
                    let p00 = chip[(y0 * W + x0) * 3 + c] as f32;
                    let p10 = chip[(y0 * W + x1) * 3 + c] as f32;
                    let p01 = chip[(y1 * W + x0) * 3 + c] as f32;
                    let p11 = chip[(y1 * W + x1) * 3 + c] as f32;

                    let top = p00 + fx * (p10 - p00);
                    let bottom = p01 + fx * (p11 - p01);
                    let val = top + fy * (bottom - top);
                    out[dst_idx + c] = val.round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
    out
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

    #[test]
    fn test_adjust_brightness_112() {
        let sample = vec![100u8; (ALIGNED_SIZE * ALIGNED_SIZE * 3) as usize];
        let bright = adjust_brightness_112(&sample, 1.2);
        assert_eq!(bright[0], 120);

        let dark = adjust_brightness_112(&sample, 0.8);
        assert_eq!(dark[0], 80);

        let overflow = adjust_brightness_112(&sample, 3.0);
        assert_eq!(overflow[0], 255);
    }

    #[test]
    fn test_crop_and_resize_chip_112() {
        let sample = vec![128u8; (ALIGNED_SIZE * ALIGNED_SIZE * 3) as usize];
        let scaled = crop_and_resize_chip_112(&sample, 0.95);
        assert_eq!(scaled.len(), sample.len());
        // 均匀图像缩放后仍应保持均匀
        assert_eq!(scaled[0], 128);
    }
}
