//! 图像处理硬件驱动 SPI (CvEngine Trait)

use crate::cv::buffer::CvBuffer;
use crate::cv::types::PreprocessMode;
use crate::error::AlgoError;
use crate::frame::SafeFrame;

/// 图像预处理驱动 SPI 接口
pub trait CvEngine: Send + Sync {
    /// 保持原图宽高比缩放并在四周补齐底色 (Letterbox)
    fn letterbox(
        &self,
        frame: &SafeFrame<'_>,
        dst_w: u32,
        dst_h: u32,
        fill_color: [u8; 3],
    ) -> Result<(CvBuffer, PreprocessMode), AlgoError>;

    /// 强制拉伸缩放到目标宽高 (Resize)
    fn resize(
        &self,
        frame: &SafeFrame<'_>,
        dst_w: u32,
        dst_h: u32,
    ) -> Result<(CvBuffer, PreprocessMode), AlgoError>;
}
