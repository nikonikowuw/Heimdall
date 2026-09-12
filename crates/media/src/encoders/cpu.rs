use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::{ExtendedColorType, RgbImage};
use types::{BoundingBox, FrameRef};

use super::{compute_crop_roi, DeviceSnapEncoder};
use crate::error::MediaError;
use crate::image_convert::snapshot_readback_to_rgb_image;

/// CPU 兜底快照编码器
///
/// 当硬件编码器不可用或报错时，作为最稳定的安全网兜底：
/// `D2H readback -> CPU NV12/格式转RGB -> CPU crop -> CPU libjpeg-turbo 编码`
#[derive(Debug, Default, Clone, Copy)]
pub struct CpuSnapEncoder;

impl CpuSnapEncoder {
    pub fn new() -> Self {
        Self
    }

    /// 一次 readback 同时生成全景与特写，避免同一帧被重复映射和色彩转换。
    pub fn encode_full_and_crop(
        &self,
        frame: &FrameRef,
        bbox: Option<BoundingBox>,
        padding_ratio: f32,
        full_quality: u8,
        crop_quality: u8,
    ) -> Result<(Vec<u8>, Option<Vec<u8>>), MediaError> {
        let rgb = snapshot_readback_to_rgb_image(frame)?;
        self.encode_full_and_crop_from_rgb(&rgb, bbox, padding_ratio, full_quality, crop_quality)
    }

    pub fn encode_full_and_crop_from_rgb(
        &self,
        rgb: &RgbImage,
        bbox: Option<BoundingBox>,
        padding_ratio: f32,
        full_quality: u8,
        crop_quality: u8,
    ) -> Result<(Vec<u8>, Option<Vec<u8>>), MediaError> {
        let full = encode_jpeg_from_rgb(rgb, full_quality)?;
        let crop = bbox
            .map(|bbox| {
                let cropped = crop_rgb_with_padding(rgb, bbox, padding_ratio);
                encode_jpeg_from_rgb(&cropped, crop_quality)
            })
            .transpose()?;
        Ok((full, crop))
    }

    pub fn encode_crop_from_rgb(
        &self,
        rgb: &RgbImage,
        bbox: BoundingBox,
        padding_ratio: f32,
        quality: u8,
    ) -> Result<Vec<u8>, MediaError> {
        let cropped = crop_rgb_with_padding(rgb, bbox, padding_ratio);
        encode_jpeg_from_rgb(&cropped, quality)
    }
}

impl DeviceSnapEncoder for CpuSnapEncoder {
    fn name(&self) -> &'static str {
        "cpu-snap"
    }

    fn encode_full_frame(&self, frame: &FrameRef, quality: u8) -> Result<Vec<u8>, MediaError> {
        let rgb = snapshot_readback_to_rgb_image(frame)?;
        encode_jpeg_from_rgb(&rgb, quality)
    }

    fn encode_crop(
        &self,
        frame: &FrameRef,
        bbox: BoundingBox,
        padding_ratio: f32,
        quality: u8,
    ) -> Result<Vec<u8>, MediaError> {
        let rgb = snapshot_readback_to_rgb_image(frame)?;
        self.encode_crop_from_rgb(&rgb, bbox, padding_ratio, quality)
    }

    fn is_ready(&self) -> bool {
        true
    }
}

/// 基于内存中的 RgbImage 编码为指定质量的 JPEG 字节流
pub fn encode_jpeg_from_rgb(img: &RgbImage, quality: u8) -> Result<Vec<u8>, MediaError> {
    if img.width() == 0 || img.height() == 0 {
        return Err(MediaError::Encode {
            reason: "图像宽高为0，无法执行 JPEG 编码".into(),
        });
    }

    let q = quality.clamp(1, 100);
    let mut buffer = Cursor::new(Vec::with_capacity(
        (img.width() * img.height() / 4).max(1024) as usize,
    ));
    let mut encoder = JpegEncoder::new_with_quality(&mut buffer, q);
    encoder
        .encode(
            img.as_raw(),
            img.width(),
            img.height(),
            ExtendedColorType::Rgb8,
        )
        .map_err(|e| MediaError::Encode {
            reason: format!("JPEG 软编码失败: {e}"),
        })?;

    Ok(buffer.into_inner())
}

/// 根据归一化 BoundingBox 对 RgbImage 进行扩边裁剪
///
/// 内部复用严格的 `compute_crop_roi` 算法，保证几何对齐与边界约束
pub fn crop_rgb_with_padding(img: &RgbImage, bbox: BoundingBox, padding_ratio: f32) -> RgbImage {
    let img_w = img.width();
    let img_h = img.height();
    if img_w == 0 || img_h == 0 {
        return img.clone();
    }

    let (x1, y1, crop_w, crop_h, _) = compute_crop_roi(img_w, img_h, bbox, padding_ratio);
    image::imageops::crop_imm(img, x1, y1, crop_w, crop_h).to_image()
}
