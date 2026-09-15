//! 图像处理硬件驱动 SPI (CvEngine Trait)

use crate::cv::buffer::CvBuffer;
use crate::cv::types::{CropRect, PreprocessMode};
use crate::error::AlgoError;
use crate::frame::SafeFrame;

/// 图像预处理驱动 SPI 接口
pub trait CvEngine: Send + Sync {
    /// 将帧的一个 ROI 转换为紧凑 RGB24。
    ///
    /// 默认实现用于不提供原生 ROI 操作的平台；硬件引擎应覆盖该方法，避免整帧读回。
    fn crop_rgb(&self, frame: &SafeFrame<'_>, rect: CropRect) -> Result<CvBuffer, AlgoError> {
        crate::cv::platforms::cpu::CpuCvEngine::new().crop_rgb_fallback(frame, rect)
    }

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
