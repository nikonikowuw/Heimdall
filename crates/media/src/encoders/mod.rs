pub mod cpu;

#[cfg(all(target_os = "linux", feature = "hw-snap-mpp"))]
pub mod mpp_snap;

pub use cpu::{crop_rgb_with_padding, encode_jpeg_from_rgb, CpuSnapEncoder};
use std::fmt;
use types::{BoundingBox, FrameRef};

use crate::error::MediaError;
use crate::image_convert::snapshot_readback_to_rgb_image;

/// 设备侧快照编码器统一抽象
///
/// 封装「全景大图编码」与「设备侧裁剪 + 特写编码」两个原子操作。
/// 各平台实现负责在设备侧完成裁剪和 JPEG 编码，仅将压缩后的
/// JPEG bitstream 拷贝至 CPU 内存用于写盘。
///
/// ## 路径约束
/// 仅用于 `snapshot_readback_path`，不得在 `infer_fast_path` 中调用。
pub trait DeviceSnapEncoder {
    /// 编码器标识名称（用于日志和降级追踪）
    fn name(&self) -> &'static str;

    /// 将视频原生帧编码为全景 JPEG 字节流
    ///
    /// - `quality`: JPEG 质量参数 (1-100)
    fn encode_full_frame(&self, frame: &FrameRef, quality: u8) -> Result<Vec<u8>, MediaError>;

    /// 裁剪目标区域并编码为特写 JPEG 字节流
    ///
    /// - `bbox`: 归一化裁剪区域 [0.0, 1.0]
    /// - `padding_ratio`: 边界扩展比例（如 0.1 = 10%）
    /// - `quality`: JPEG 质量 (1-100)
    fn encode_crop(
        &self,
        frame: &FrameRef,
        bbox: BoundingBox,
        padding_ratio: f32,
        quality: u8,
    ) -> Result<Vec<u8>, MediaError>;

    /// 编码器是否就绪（硬件上下文已初始化且可用）
    fn is_ready(&self) -> bool;
}

/// 计算符合硬件（如 RGA/VPU/DVPP）约束的裁剪 ROI
///
/// 返回 `(x, y, crop_w, crop_h, w_stride)`:
/// - `x, y`: 裁剪起始点（偶数对齐，满足 NV12 色度采样要求）
/// - `crop_w, crop_h`: 裁剪逻辑可见宽高（偶数对齐，硬件 RGA 路径另行校验平台最小尺寸）
/// - `w_stride`: 硬件 Stride 对齐后的宽度（16 字节对齐，供 DMA-BUF 步长与 Buffer 填充使用）
pub fn compute_crop_roi(
    frame_w: u32,
    frame_h: u32,
    bbox: BoundingBox,
    padding_ratio: f32,
) -> (u32, u32, u32, u32, u32) {
    if frame_w == 0 || frame_h == 0 {
        return (0, 0, 0, 0, 0);
    }

    let w = frame_w as f32;
    let h = frame_h as f32;

    // 1. 归一化 BBox 扩边
    let pad_w = (bbox.x2 - bbox.x1).max(0.0) * padding_ratio;
    let pad_h = (bbox.y2 - bbox.y1).max(0.0) * padding_ratio;

    let x1_f = ((bbox.x1 - pad_w).max(0.0) * w).floor();
    let y1_f = ((bbox.y1 - pad_h).max(0.0) * h).floor();
    let x2_f = ((bbox.x2 + pad_w).min(1.0) * w).ceil();
    let y2_f = ((bbox.y2 + pad_h).min(1.0) * h).ceil();

    // 2. 钳位与偶数对齐 (NV12 2x2 子采样要求 x, y 必须为偶数)
    let mut x1 = (x1_f as u32).min(frame_w.saturating_sub(2)) & !1;
    let mut y1 = (y1_f as u32).min(frame_h.saturating_sub(2)) & !1;
    let x2 = (x2_f as u32).min(frame_w);
    let y2 = (y2_f as u32).min(frame_h);

    let mut crop_w = ((x2.saturating_sub(x1)) & !1).max(2);
    let mut crop_h = ((y2.saturating_sub(y1)) & !1).max(2);

    // CPU crop 保持 32px 最小下限；RGA 硬件路径在 rga_crop 中按目标 BSP 的更严格下限校验。
    const MIN_CROP_DIM: u32 = 32;
    if frame_w >= MIN_CROP_DIM {
        crop_w = crop_w.max(MIN_CROP_DIM).min(frame_w & !1);
    } else {
        crop_w = crop_w.min(frame_w & !1).max(2);
    }

    if frame_h >= MIN_CROP_DIM {
        crop_h = crop_h.max(MIN_CROP_DIM).min(frame_h & !1);
    } else {
        crop_h = crop_h.min(frame_h & !1).max(2);
    }

    // 4. 确保向右/向下不超出帧边界（超出时向左上平移补偿并保持偶数）
    if x1 + crop_w > frame_w {
        x1 = frame_w.saturating_sub(crop_w) & !1;
    }
    if y1 + crop_h > frame_h {
        y1 = frame_h.saturating_sub(crop_h) & !1;
    }

    // 5. 16 字节 Stride 对齐 (RGA3 / DMA-BUF 对齐规范)
    let w_stride = (crop_w + 15) & !15;

    (x1, y1, crop_w, crop_h, w_stride)
}

/// 快照编码器：单实例串行调度器
///
/// 硬件编码器常驻单实例，串行排队处理快照，避免多实例争抢硬件通道及耗尽 CMA 内存。
/// 若硬件编码失败，自动平滑降级至 CPU 路径，确保案由证据 100% 留存。
pub struct SnapEncoder {
    hw_encoder: Option<Box<dyn DeviceSnapEncoder>>,
    cpu_encoder: CpuSnapEncoder,
}

impl fmt::Debug for SnapEncoder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SnapEncoder")
            .field("hw_encoder", &self.hw_encoder.as_ref().map(|e| e.name()))
            .field("cpu_encoder", &self.cpu_encoder)
            .finish()
    }
}

impl SnapEncoder {
    pub fn new(hw_encoder: Option<Box<dyn DeviceSnapEncoder>>) -> Self {
        Self {
            hw_encoder,
            cpu_encoder: CpuSnapEncoder::new(),
        }
    }

    pub fn cpu_only() -> Self {
        Self::new(None)
    }

    /// 返回当前活跃编码器标识（用于日志追踪）
    pub fn name(&self) -> &'static str {
        self.hw_encoder
            .as_ref()
            .map(|e| e.name())
            .unwrap_or("cpu-snap")
    }

    /// 启动时探测平台硬件能力，常驻创建最优快照编码器实例。
    ///
    /// 遵循并发规范「硬件上下文全局常驻单一实例」：
    /// - Linux MPP：创建 `MppSnapEncoder`（含 RGA 运行时 + Scratchpad DMA-BUF）
    /// - macOS VT / 华为 DVPP：后续阶段接入
    /// - 无硬件或初始化失败：降级至 `CpuSnapEncoder`
    ///
    /// 此方法应在 `SnapshotEngine` 构造时调用一次，结果常驻使用。
    pub fn try_new() -> Self {
        #[cfg(all(target_os = "linux", feature = "hw-snap-mpp"))]
        {
            if let Ok(enc) = mpp_snap::MppSnapEncoder::try_new(85) {
                tracing::info!("MPP 硬件快照编码器初始化成功");
                return Self::new(Some(Box::new(enc)));
            }
            tracing::warn!("MPP 硬件快照编码器初始化失败，降级至 CPU");
        }
        #[cfg(all(target_os = "macos", feature = "hw-snap-vt"))]
        {
            tracing::info!("VideoToolbox 硬件快照编码器尚未就绪，降级至 CPU 兜底");
        }
        #[cfg(all(target_os = "linux", feature = "hw-snap-dvpp"))]
        {
            tracing::info!("Ascend DVPP 硬件快照编码器尚未就绪，降级至 CPU 兜底");
        }
        Self::cpu_only()
    }

    /// 编码全景图（硬件优先，失败则降级到 CPU）
    pub fn encode_full_frame(&self, frame: &FrameRef, quality: u8) -> Result<Vec<u8>, MediaError> {
        if let Some(ref hw) = self.hw_encoder {
            if hw.is_ready() {
                match hw.encode_full_frame(frame, quality) {
                    Ok(jpeg) => return Ok(jpeg),
                    Err(e) => {
                        tracing::warn!(
                            encoder = %hw.name(),
                            error = %e,
                            "硬件全景快照编码失败，平滑降级至 CPU 编码"
                        );
                    }
                }
            }
        }

        self.cpu_encoder.encode_full_frame(frame, quality)
    }

    /// 编码特写裁剪图（硬件优先，失败则降级到 CPU）
    pub fn encode_crop(
        &self,
        frame: &FrameRef,
        bbox: BoundingBox,
        padding_ratio: f32,
        quality: u8,
    ) -> Result<Vec<u8>, MediaError> {
        if let Some(ref hw) = self.hw_encoder {
            if hw.is_ready() {
                match hw.encode_crop(frame, bbox, padding_ratio, quality) {
                    Ok(jpeg) => return Ok(jpeg),
                    Err(e) => {
                        tracing::warn!(
                            encoder = %hw.name(),
                            error = %e,
                            "硬件特写快照裁剪编码失败，平滑降级至 CPU 编码"
                        );
                    }
                }
            }
        }

        self.cpu_encoder
            .encode_crop(frame, bbox, padding_ratio, quality)
    }
    /// 单次 readback 生成全景与特写，硬件全景成功而特写失败时仅回读一次用于特写。
    pub fn encode_full_and_crop(
        &self,
        frame: &FrameRef,
        bbox: Option<BoundingBox>,
        padding_ratio: f32,
        full_quality: u8,
        crop_quality: u8,
    ) -> Result<(Vec<u8>, Option<Vec<u8>>), MediaError> {
        let Some(bbox) = bbox else {
            return self
                .encode_full_frame(frame, full_quality)
                .map(|jpeg| (jpeg, None));
        };

        if let Some(ref hw) = self.hw_encoder {
            if hw.is_ready() {
                match hw.encode_full_frame(frame, full_quality) {
                    Ok(full) => match hw.encode_crop(frame, bbox, padding_ratio, crop_quality) {
                        Ok(crop) => return Ok((full, Some(crop))),
                        Err(error) => {
                            tracing::warn!(
                                encoder = %hw.name(),
                                error = %error,
                                "硬件特写失败，复用一次 CPU readback 生成特写"
                            );
                            let rgb = snapshot_readback_to_rgb_image(frame)?;
                            let crop = self.cpu_encoder.encode_crop_from_rgb(
                                &rgb,
                                bbox,
                                padding_ratio,
                                crop_quality,
                            )?;
                            return Ok((full, Some(crop)));
                        }
                    },
                    Err(error) => {
                        tracing::warn!(
                            encoder = %hw.name(),
                            error = %error,
                            "硬件全景失败，使用一次 CPU readback 生成全景与特写"
                        );
                    }
                }
            }
        }

        self.cpu_encoder.encode_full_and_crop(
            frame,
            Some(bbox),
            padding_ratio,
            full_quality,
            crop_quality,
        )
    }
}

impl Default for SnapEncoder {
    fn default() -> Self {
        Self::cpu_only()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_crop_roi_basic() {
        let (x, y, w, h, stride) =
            compute_crop_roi(1920, 1080, BoundingBox::new(0.3, 0.3, 0.7, 0.7), 0.1);
        assert!(w > 0 && h > 0);
        assert!(x + w <= 1920);
        assert!(y + h <= 1080);
        assert_eq!(w % 2, 0, "crop_w 必须偶数对齐");
        assert_eq!(h % 2, 0, "crop_h 必须偶数对齐");
        assert_eq!(x % 2, 0, "x 必须偶数对齐");
        assert_eq!(y % 2, 0, "y 必须偶数对齐");
        assert_eq!(stride % 16, 0, "w_stride 必须 16 字节对齐");
        assert!(stride >= w, "w_stride 必须大于等于 crop_w");
    }

    #[test]
    fn test_compute_crop_roi_edge_cases() {
        let (x, y, w, h, stride) =
            compute_crop_roi(1920, 1080, BoundingBox::new(0.0, 0.0, 1.0, 1.0), 0.1);
        assert_eq!(x, 0);
        assert_eq!(y, 0);
        assert_eq!(w, 1920);
        assert_eq!(h, 1080);
        assert_eq!(stride % 16, 0);
    }

    #[test]
    fn test_compute_crop_roi_odd_dimensions() {
        let (x, y, w, h, stride) =
            compute_crop_roi(1920, 1080, BoundingBox::new(0.05, 0.05, 0.31, 0.33), 0.0);
        assert_eq!(w % 2, 0, "宽度必须对齐为偶数");
        assert_eq!(h % 2, 0, "高度必须对齐为偶数");
        assert_eq!(x % 2, 0, "x 起点必须偶数对齐");
        assert_eq!(y % 2, 0, "y 起点必须偶数对齐");
        assert_eq!(stride % 16, 0);
    }

    #[test]
    fn test_compute_crop_roi_tiny_target() {
        // 微小目标 (例如 4x4 像素)，必须被钳位至硬件下限 32x32
        let (x, y, w, h, stride) =
            compute_crop_roi(1920, 1080, BoundingBox::new(0.5, 0.5, 0.502, 0.503), 0.0);
        assert!(w >= 32, "必须满足硬件下限 >= 32");
        assert!(h >= 32, "必须满足硬件下限 >= 32");
        assert_eq!(w % 2, 0);
        assert_eq!(h % 2, 0);
        assert_eq!(x % 2, 0);
        assert_eq!(y % 2, 0);
        assert!(x + w <= 1920);
        assert!(y + h <= 1080);
        assert_eq!(stride % 16, 0);
    }

    #[test]
    fn test_compute_crop_roi_right_bottom_boundary() {
        // 贴紧右下角的微小目标，平移补偿后仍不能越界
        let (x, y, w, h, stride) =
            compute_crop_roi(1920, 1080, BoundingBox::new(0.999, 0.999, 1.0, 1.0), 0.0);
        assert!(w >= 32);
        assert!(h >= 32);
        assert!(x + w <= 1920, "不得超出右边界");
        assert!(y + h <= 1080, "不得超出下边界");
        assert_eq!(x % 2, 0);
        assert_eq!(y % 2, 0);
        assert_eq!(stride % 16, 0);
    }

    #[test]
    fn test_hardware_crop_failure_reuses_single_cpu_readback_path() {
        struct CropFailureEncoder;

        impl DeviceSnapEncoder for CropFailureEncoder {
            fn name(&self) -> &'static str {
                "mock-hardware"
            }

            fn encode_full_frame(
                &self,
                _frame: &FrameRef,
                _quality: u8,
            ) -> Result<Vec<u8>, MediaError> {
                Ok(vec![0xff, 0xd8, 0xff, 0xd9])
            }

            fn encode_crop(
                &self,
                _frame: &FrameRef,
                _bbox: BoundingBox,
                _padding_ratio: f32,
                _quality: u8,
            ) -> Result<Vec<u8>, MediaError> {
                Err(MediaError::Encode {
                    reason: "mock RGA crop failure".into(),
                })
            }

            fn is_ready(&self) -> bool {
                true
            }
        }

        let frame = FrameRef::new(
            "camera".into(),
            1,
            4,
            4,
            types::StrideInfo::new(4, 4),
            types::PixelFormat::Nv12,
            types::FrameHandle::Host(std::sync::Arc::from(vec![128u8; 24])),
        );
        let encoder = SnapEncoder::new(Some(Box::new(CropFailureEncoder)));
        let (full, crop) = encoder
            .encode_full_and_crop(
                &frame,
                Some(BoundingBox::new(0.0, 0.0, 1.0, 1.0)),
                0.1,
                90,
                90,
            )
            .expect("硬件特写失败后应由 CPU 生成特写");

        assert_eq!(full, vec![0xff, 0xd8, 0xff, 0xd9]);
        let crop = crop.expect("应返回 CPU 特写 JPEG");
        assert_eq!(&crop[..2], &[0xff, 0xd8]);
    }

    #[test]
    fn test_cpu_snap_encoder_encode_jpeg() {
        let mut img = image::RgbImage::new(64, 64);
        for pixel in img.pixels_mut() {
            *pixel = image::Rgb([120, 140, 160]);
        }
        let jpeg = encode_jpeg_from_rgb(&img, 85).expect("JPEG 编码必须成功");
        assert!(!jpeg.is_empty());
        // JPEG SOI 标记 0xFF, 0xD8
        assert_eq!(jpeg[0], 0xFF);
        assert_eq!(jpeg[1], 0xD8);
    }
}
