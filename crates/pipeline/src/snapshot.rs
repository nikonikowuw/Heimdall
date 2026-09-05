//! 靶向精准高清抽帧与异步 JPEG 编码引擎
//!
//! 1. 告警触发时，按时标 T 从 MainStreamRingBuffer 索引前置 I 帧并快进解码出单帧 1080P/4K 原图；
//! 2. 若主码流断线或无可用 GOP，自动平滑降级抓取子码流当前帧，保证 100% 不漏图；
//! 3. 将图像色彩空间转换、扩边裁剪、JPEG 压缩与文件落盘卸载至 Dedicated Blocking 线程池，杜绝阻塞 Tokio Worker；
//! 4. 产物落盘至 `var/data/evidence/{camera_id}/`。

use std::fs;
use std::io::Cursor;
use std::path::PathBuf;

use image::codecs::jpeg::JpegEncoder;
use image::{ExtendedColorType, RgbImage};
use media::decoder::VideoDecoder;
use media::ring_buffer::MainStreamRingBuffer;
use types::{BoundingBox, FrameRef};

use crate::error::PipelineError;

/// 快照抓拍产物信息
#[derive(Debug, Clone)]
pub struct SnapshotResult {
    pub image_id: String,
    pub crop_image_id: String,
    pub image_rel_path: String,
    pub crop_image_rel_path: String,
    pub file_size_bytes: usize,
    pub width: u32,
    pub height: u32,
    pub is_fallback_sub_stream: bool,
}

/// 快照抓拍引擎
#[derive(Debug, Clone)]
pub struct SnapshotEngine {
    base_evidence_dir: PathBuf,
}

impl SnapshotEngine {
    pub fn new(base_evidence_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_evidence_dir: base_evidence_dir.into(),
        }
    }

    pub fn base_evidence_dir(&self) -> &std::path::Path {
        &self.base_evidence_dir
    }

    /// 根据时标从主码流 GOP 快速解码，或回退至子码流当前帧
    pub async fn decode_target_frame(
        camera_id: &str,
        target_pts_ms: i64,
        ring_buffer: Option<&MainStreamRingBuffer>,
        sub_stream_fallback: Option<&FrameRef>,
        main_decoder: Option<&mut (dyn VideoDecoder + Send)>,
    ) -> Result<(FrameRef, bool), PipelineError> {
        let mut decoded_frame: Option<FrameRef> = None;

        // 1. 尝试从主码流 RingBuffer 快进解码
        if let (Some(rb), Some(decoder)) = (ring_buffer, main_decoder) {
            if let Some(gop) = rb.get_gop_for_timestamp(target_pts_ms) {
                tracing::debug!(
                    camera_id = %camera_id,
                    target_pts = target_pts_ms,
                    packet_count = gop.len(),
                    "从主码流 RingBuffer 提取出 GOP 切片，开始快进解码"
                );

                for pkt in gop {
                    if let Ok(Some(frame)) = decoder.decode_packet(&pkt.payload, pkt.pts_ms).await {
                        decoded_frame = Some(frame);
                    }
                }
            }
        }

        // 2. 若主流未能解码出有效帧，自动回退借用子码流当前帧
        if let Some(frame) = decoded_frame {
            Ok((frame, false))
        } else if let Some(fallback) = sub_stream_fallback {
            tracing::warn!(
                camera_id = %camera_id,
                target_pts = target_pts_ms,
                "主码流靶向抽帧未就绪，平滑降级使用子码流当前帧"
            );
            Ok((fallback.clone(), true))
        } else {
            Err(PipelineError::PipelineNotFound {
                camera_id: format!("{camera_id} (无可用视频帧用于抓拍)"),
            })
        }
    }

    /// 将 CPU 密集型图像色彩转换、扩边裁剪、JPEG 压缩与磁盘 IO 卸载至 blocking 线程池
    pub async fn save_snapshot_async(
        &self,
        camera_id: &str,
        frame: FrameRef,
        target_bbox: Option<BoundingBox>,
        is_fallback: bool,
    ) -> Result<SnapshotResult, PipelineError> {
        let cam_id_owned = camera_id.to_string();
        let base_dir = self.base_evidence_dir.clone();

        tokio::task::spawn_blocking(move || {
            encode_and_save_snapshot(&cam_id_owned, frame, target_bbox, &base_dir, is_fallback)
        })
        .await
        .map_err(|e| PipelineError::Snapshot(format!("后台抓拍任务执行异常: {e}")))?
    }

    /// 执行靶向快拍：优先从主码流 RingBuffer 提取并解码，失败则回退至子码流
    pub async fn capture_snapshot(
        &self,
        camera_id: &str,
        target_pts_ms: i64,
        target_bbox: Option<BoundingBox>,
        ring_buffer: Option<&MainStreamRingBuffer>,
        sub_stream_fallback: Option<&FrameRef>,
        main_decoder: Option<&mut (dyn VideoDecoder + Send)>,
    ) -> Result<SnapshotResult, PipelineError> {
        let (frame, is_fallback) = Self::decode_target_frame(
            camera_id,
            target_pts_ms,
            ring_buffer,
            sub_stream_fallback,
            main_decoder,
        )
        .await?;

        self.save_snapshot_async(camera_id, frame, target_bbox, is_fallback)
            .await
    }
}

/// 同步高效执行图像转换、抠图裁切、JPEG 压缩与原子文件落盘
pub(crate) fn encode_and_save_snapshot(
    camera_id: &str,
    frame: FrameRef,
    target_bbox: Option<BoundingBox>,
    base_evidence_dir: &std::path::Path,
    is_fallback: bool,
) -> Result<SnapshotResult, PipelineError> {
    // 1. 调用 media 层统一定点数快速图像色彩转换
    let rgb_img = media::frame_to_rgb_image(&frame)
        .map_err(|e| PipelineError::Snapshot(format!("提取视频帧 RGB 图像失败: {e}")))?;

    let width = rgb_img.width();
    let height = rgb_img.height();

    // 2. 生成唯一图片 ID 与落盘相对路径
    let image_id = format!("img_{}_{}", now_compact_ts(), uuid::Uuid::new_v4().simple());
    let crop_image_id = format!(
        "crop_{}_{}",
        now_compact_ts(),
        uuid::Uuid::new_v4().simple()
    );

    let cam_dir = base_evidence_dir.join(camera_id);
    fs::create_dir_all(&cam_dir)
        .map_err(|e| PipelineError::Snapshot(format!("创建证据目录失败: {e}")))?;

    let full_filename = format!("{image_id}.jpg");
    let crop_filename = format!("{crop_image_id}.jpg");

    let full_path = cam_dir.join(&full_filename);
    let crop_path = cam_dir.join(&crop_filename);

    // 3. 编码全景 JPEG (质量 85)
    let full_jpeg_bytes = encode_jpeg(&rgb_img, 85)?;
    let file_size_bytes = full_jpeg_bytes.len();
    atomic_write_file(&full_path, &full_jpeg_bytes)
        .map_err(|e| PipelineError::Snapshot(format!("写入全景抓拍图片失败: {e}")))?;

    // 4. 按 BBox 裁切特写图并编码 JPEG (扩边 10% 避免切边)
    let crop_storage;
    let crop_img: &RgbImage = match target_bbox {
        Some(bbox) => {
            crop_storage = crop_with_padding(&rgb_img, bbox, 0.1);
            &crop_storage
        }
        None => &rgb_img,
    };

    let crop_jpeg_bytes = encode_jpeg(crop_img, 90)?;
    atomic_write_file(&crop_path, &crop_jpeg_bytes)
        .map_err(|e| PipelineError::Snapshot(format!("写入特写抠图图片失败: {e}")))?;

    let image_rel_path = format!("{camera_id}/{full_filename}");
    let crop_image_rel_path = format!("{camera_id}/{crop_filename}");

    tracing::info!(
        camera_id = %camera_id,
        image_id = %image_id,
        crop_id = %crop_image_id,
        file_size_bytes,
        is_fallback,
        "靶向证据高清抓拍完成 (blocking 线程池异步落盘)"
    );

    Ok(SnapshotResult {
        image_id,
        crop_image_id,
        image_rel_path,
        crop_image_rel_path,
        file_size_bytes,
        width,
        height,
        is_fallback_sub_stream: is_fallback,
    })
}

/// 编码为 JPEG 字节切片
fn encode_jpeg(img: &RgbImage, quality: u8) -> Result<Vec<u8>, PipelineError> {
    let mut buffer = Cursor::new(Vec::with_capacity(
        (img.width() * img.height()) as usize / 4,
    ));
    let mut encoder = JpegEncoder::new_with_quality(&mut buffer, quality);
    encoder
        .encode(
            img.as_raw(),
            img.width(),
            img.height(),
            ExtendedColorType::Rgb8,
        )
        .map_err(|e| PipelineError::Snapshot(format!("JPEG 编码失败: {e}")))?;

    Ok(buffer.into_inner())
}

/// 原子化文件写入：通过写入同目录临时文件后重命名保证写入原子性
fn atomic_write_file(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    let tmp_path = path.with_extension(format!("tmp.{}", uuid::Uuid::new_v4().simple()));
    fs::write(&tmp_path, data)?;
    if let Err(e) = fs::rename(&tmp_path, path) {
        let _ = fs::remove_file(&tmp_path);
        return Err(e);
    }
    Ok(())
}

/// 根据 BoundingBox 扩边裁剪 (具备边界自适应与极值防越界防护)
fn crop_with_padding(img: &RgbImage, bbox: BoundingBox, padding_ratio: f32) -> RgbImage {
    let img_w = img.width();
    let img_h = img.height();
    if img_w == 0 || img_h == 0 {
        return img.clone();
    }

    let w = img_w as f32;
    let h = img_h as f32;

    let pad_w = (bbox.x2 - bbox.x1).max(0.0) * padding_ratio;
    let pad_h = (bbox.y2 - bbox.y1).max(0.0) * padding_ratio;

    let x1 = ((bbox.x1 - pad_w).clamp(0.0, 1.0) * w).floor() as u32;
    let y1 = ((bbox.y1 - pad_h).clamp(0.0, 1.0) * h).floor() as u32;
    let x2 = ((bbox.x2 + pad_w).clamp(0.0, 1.0) * w).ceil() as u32;
    let y2 = ((bbox.y2 + pad_h).clamp(0.0, 1.0) * h).ceil() as u32;

    let x1 = x1.min(img_w.saturating_sub(1));
    let y1 = y1.min(img_h.saturating_sub(1));
    let x2 = x2.clamp(x1 + 1, img_w);
    let y2 = y2.clamp(y1 + 1, img_h);

    let crop_w = x2 - x1;
    let crop_h = y2 - y1;

    image::imageops::crop_imm(img, x1, y1, crop_w, crop_h).to_image()
}

fn now_compact_ts() -> String {
    chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use types::{FrameHandle, PixelFormat, StrideInfo};

    #[tokio::test]
    async fn test_snapshot_engine_fallback_flow() {
        let temp_dir =
            std::env::temp_dir().join(format!("test_evidence_{}", uuid::Uuid::new_v4().simple()));
        let engine = SnapshotEngine::new(&temp_dir);

        // 创建模拟子流帧 (320x240 NV12)
        let width = 320;
        let height = 240;
        let nv12_size = (width * height * 3 / 2) as usize;
        let dummy_nv12 = vec![160u8; nv12_size].into();

        let fallback_frame = FrameRef::new(
            "cam_test_001".to_string(),
            1741100000000,
            width,
            height,
            StrideInfo::new(width, height),
            PixelFormat::Nv12,
            FrameHandle::Host(dummy_nv12),
        );

        let bbox = BoundingBox::new(0.2, 0.2, 0.6, 0.6);

        // 测试主码流为空时的平滑降级抓拍
        let result = engine
            .capture_snapshot(
                "cam_test_001",
                1741100000000,
                Some(bbox),
                None, // 无主码流 RingBuffer
                Some(&fallback_frame),
                None, // 无主码流解码器
            )
            .await
            .expect("抓拍应当成功");

        assert!(result.is_fallback_sub_stream);
        assert_eq!(result.width, 320);
        assert_eq!(result.height, 240);
        assert!(result.file_size_bytes > 0);

        // 验证文件真实落盘
        let full_file = temp_dir.join(&result.image_rel_path);
        let crop_file = temp_dir.join(&result.crop_image_rel_path);
        assert!(full_file.is_file(), "全景大图必须落盘");
        assert!(crop_file.is_file(), "特写抠图必须落盘");

        // 清理测试临时目录
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_crop_with_padding() {
        let mut img = RgbImage::new(100, 100);
        img.put_pixel(50, 50, image::Rgb([255, 0, 0]));

        let bbox = BoundingBox::new(0.4, 0.4, 0.6, 0.6);
        let cropped = crop_with_padding(&img, bbox, 0.1);

        assert!(cropped.width() >= 20 && cropped.width() <= 28);
        assert!(cropped.height() >= 20 && cropped.height() <= 28);

        // 测试极限退化与越界坐标防护
        let bbox_edge = BoundingBox::new(0.99, 0.99, 1.0, 1.0);
        let crop_edge = crop_with_padding(&img, bbox_edge, 0.1);
        assert!(crop_edge.width() >= 1 && crop_edge.width() <= 100);
        assert!(crop_edge.height() >= 1 && crop_edge.height() <= 100);

        let bbox_outside = BoundingBox::new(-0.5, -0.5, 1.5, 1.5);
        let crop_full = crop_with_padding(&img, bbox_outside, 0.1);
        assert_eq!(crop_full.width(), 100);
        assert_eq!(crop_full.height(), 100);
    }
}
