//! 5 点人脸对齐：基于 Umeyama 相似变换与双线性插值的 112×112 RGB 裁剪
//!
//! 核心算法已下沉至 [`algo_sdk::face::align`]。本模块提供重导出及算法包内部调试落盘工具。

pub use algo_sdk::face::align::*;

/// ArcFace 官方标准 112×112 对齐五点像素坐标 (圆整用于十字准星高亮)
const ARC_FACE_BENCHMARK_PTS: [(u32, u32); 5] = [
    (38, 52), // left eye
    (74, 52), // right eye
    (56, 72), // nose
    (42, 92), // left mouth
    (71, 92), // right mouth
];

/// 调试辅助工具：将 112×112 对齐人脸图像落盘到 `/tmp/heimdall_aligned/`
pub fn dump_debug_aligned_face(tag: &str, rgb_112: &[u8], quality_score: f32) {
    if std::env::var("HEIMDALL_DUMP_ALIGNED").unwrap_or_default() != "1" {
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

    let raw_name = format!("{tag}_q{q}_{ts}_raw.png");
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
            .collect();
        assert_eq!(files.len(), 2, "应生成 raw 和 overlay 两张图片");

        let _ = std::fs::remove_dir_all(&temp_dir);
        std::env::remove_var("HEIMDALL_DUMP_ALIGNED");
        std::env::remove_var("HEIMDALL_ALIGNED_DIR");
    }
}
