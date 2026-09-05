//! Letterbox 几何布局纯数学计算

use super::types::LetterboxLayout;

/// 计算 Letterbox 缩放与黑边布局
///
/// 保持原图比例缩放，并居中贴在目标画布上，四周填充黑边
pub fn compute_letterbox_layout(src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> LetterboxLayout {
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
        return LetterboxLayout {
            scale: 1.0,
            pad_left: 0,
            pad_top: 0,
            dst_w,
            dst_h,
            scaled_w: dst_w,
            scaled_h: dst_h,
        };
    }

    let r_w = dst_w as f32 / src_w as f32;
    let r_h = dst_h as f32 / src_h as f32;
    let scale = r_w.min(r_h);

    let scaled_w = ((src_w as f32 * scale).round() as u32).min(dst_w);
    let scaled_h = ((src_h as f32 * scale).round() as u32).min(dst_h);

    let pad_left = (dst_w.saturating_sub(scaled_w)) / 2;
    let pad_top = (dst_h.saturating_sub(scaled_h)) / 2;

    LetterboxLayout {
        scale,
        pad_left,
        pad_top,
        dst_w,
        dst_h,
        scaled_w,
        scaled_h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_letterbox_layout_16_9_to_square() {
        // 1920x1080 -> 640x640: scale = 640/1920 = 1/3 (0.33333)
        // scaled_w = 640, scaled_h = 360
        // pad_left = 0, pad_top = (640 - 360) / 2 = 140
        let layout = compute_letterbox_layout(1920, 1080, 640, 640);
        assert_eq!(layout.scaled_w, 640);
        assert_eq!(layout.scaled_h, 360);
        assert_eq!(layout.pad_left, 0);
        assert_eq!(layout.pad_top, 140);
        assert!((layout.scale - (640.0 / 1920.0)).abs() < 1e-4);
    }

    #[test]
    fn test_letterbox_layout_tall_to_square() {
        // 1080x1920 -> 640x640: scale = 640/1920
        // scaled_h = 640, scaled_w = 360
        // pad_top = 0, pad_left = 140
        let layout = compute_letterbox_layout(1080, 1920, 640, 640);
        assert_eq!(layout.scaled_h, 640);
        assert_eq!(layout.scaled_w, 360);
        assert_eq!(layout.pad_top, 0);
        assert_eq!(layout.pad_left, 140);
    }
}
