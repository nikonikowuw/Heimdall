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

    /// 释放本引擎持有的全部硬件资源（RGA 句柄、映射与池）。
    ///
    /// **为何需要显式方法**：进程级默认引擎通常存放在 `static` 容器里
    /// （见 [`crate::cv::default_engine`]），而 Rust 静态变量**永不执行 `Drop`**。
    /// 宿主动态卸载算法包时，若不显式回收，RGA 池持有的 DMA-BUF 导入句柄会一直
    /// 挂在 `drm`/`rga_mm` 上，直到进程退出才由内核强制回收。
    ///
    /// 由 [`crate::cv::DefaultEngineLease`] 在最后一个算法实例销毁时触发
    /// （见 [`crate::cv::release_default_engine`]）；**不要**改挂到
    /// `library_close_hook`——该钩子在每次 `RawAlgoLibrary::drop` 都触发，
    /// 含 `extract_face` 等高频短操作，会造成池反复销毁/重建。
    /// 默认空实现适用于不持有硬件资源的引擎（CPU / Apple / 宿主代理）。
    ///
    /// 释放后引擎不应再被使用；重复调用必须幂等。
    fn release_hardware(&self) {}
}
