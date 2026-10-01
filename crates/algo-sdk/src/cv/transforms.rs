//! 硬件加速变换算子 (`Transform`)
//!
//! 类似于 `torchvision.transforms`，将异构 2D 硬件操作抽象为可组合的流水线。
//!
//! 当前仅交付 [`HwLetterbox`]。ROI 抠图请使用 [`crate::cv::CvEngine::crop_rgb`]（非本 trait 实现），
//! 五点仿射变换请使用 [`crate::face::align`]；两者尚未收敛为 `Transform` 算子。

use super::buffer::CvBuffer;
use super::types::PreprocessMode;
use crate::error::AlgoError;
use crate::frame::SafeFrame;

/// 硬件加速变换算子契约
pub trait Transform: Send + Sync {
    /// 对输入帧执行变换，产出驻留在物理显存/DMA-BUF 中的 `CvBuffer` 与几何映射模式
    fn apply(&self, frame: &SafeFrame<'_>) -> Result<(CvBuffer, PreprocessMode), AlgoError>;
}

/// 硬件级等比缩放与居中填充算子 (Letterbox)
#[derive(Debug, Clone)]
pub struct HwLetterbox {
    pub target_w: u32,
    pub target_h: u32,
    pub fill_color: [u8; 3],
}

impl HwLetterbox {
    /// 构造指定目标分辨率的 Letterbox 变换算子（默认填充底色 [114, 114, 114]）
    pub fn new(target_w: u32, target_h: u32) -> Self {
        Self {
            target_w,
            target_h,
            fill_color: [114, 114, 114],
        }
    }

    /// 自定义填充背景底色
    pub fn with_fill(mut self, fill: [u8; 3]) -> Self {
        self.fill_color = fill;
        self
    }
}

impl Transform for HwLetterbox {
    fn apply(&self, frame: &SafeFrame<'_>) -> Result<(CvBuffer, PreprocessMode), AlgoError> {
        let engine = super::active_engine();
        engine.letterbox(frame, self.target_w, self.target_h, self.fill_color)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::MockFrameBuilder;

    #[test]
    fn test_hw_letterbox_basic() {
        let transform = HwLetterbox::new(640, 640).with_fill([0, 0, 0]);
        let frame = MockFrameBuilder::new()
            .dimensions(1920, 1080)
            .to_nv12(16)
            .build();
        let safe = frame.as_safe_frame();

        let (buf, mode) = transform.apply(&safe).expect("letterbox apply");
        assert_eq!(buf.width(), 640);
        assert_eq!(buf.height(), 640);

        match mode {
            PreprocessMode::Letterbox(layout) => {
                assert!(layout.scale > 0.0);
                assert_eq!(layout.pad_left, 0);
                assert!(layout.pad_top > 0);
            }
            _ => panic!("Expected PreprocessMode::Letterbox"),
        }
    }
}
