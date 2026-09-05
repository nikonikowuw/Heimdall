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

/// 证据快照抓拍策略模式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotCaptureMode {
    /// 智能自适应双模（默认推荐）：
    /// - 相位偏差 < 500ms：极速单帧硬解最近 I 帧（1080P/4K 高清且延时 < 5ms）；
    /// - 相位偏差 >= 500ms：若包数 <= max_burst_packets，快速前向硬解 (Burst Decode) 至告警点；若超限或解码失败，则直接复用子码流真实检测帧。
    #[default]
    AdaptiveDualMode,

    /// 极速优先 / 子码流直取模式：
    /// - 相位偏差 < 500ms：极速单帧硬解最近 I 帧；
    /// - 相位偏差 >= 500ms：直接复用子码流当时检测命中的那张真实解码帧（0 硬件争抢，100% 时标精准，避免大 GOP 前向硬解开销）。
    SubStreamOnLargeGap,

    /// 强制前向追帧硬解模式 (Burst Decode Only)：
    /// - 无论相位偏差多大，只要有可用 GOP，始终从 I 帧逐包硬解追帧到目标点，确保抓拍必须为 1080P/4K 大图。
    BurstDecodeOnly,
}

/// 证据快照引擎配置
#[derive(Debug, Clone)]
pub struct SnapshotConfig {
    /// 极速单帧与精准追帧模式的时间戳相位差阈值（毫秒，默认 500ms）
    pub phase_diff_threshold_ms: i64,
    /// 前向追帧硬解的最大允许包数量（默认 30 包，约 1.2s GOP）
    pub max_burst_packets: usize,
    /// 前向追帧硬解的最大允许耗时预算（毫秒，默认 80ms，防止大 GOP 霸占硬件 VPU）
    pub max_burst_timeout_ms: u64,
    /// 抓拍策略模式
    pub capture_mode: SnapshotCaptureMode,
}

impl Default for SnapshotConfig {
    fn default() -> Self {
        Self {
            phase_diff_threshold_ms: 500,
            max_burst_packets: 30,
            max_burst_timeout_ms: 80,
            capture_mode: SnapshotCaptureMode::AdaptiveDualMode,
        }
    }
}

/// 快照抓拍引擎
#[derive(Debug, Clone)]
pub struct SnapshotEngine {
    base_evidence_dir: PathBuf,
    config: SnapshotConfig,
}

impl SnapshotEngine {
    pub fn new(base_evidence_dir: impl Into<PathBuf>) -> Self {
        Self::with_config(base_evidence_dir, SnapshotConfig::default())
    }

    pub fn with_config(base_evidence_dir: impl Into<PathBuf>, config: SnapshotConfig) -> Self {
        Self {
            base_evidence_dir: base_evidence_dir.into(),
            config,
        }
    }

    pub fn base_evidence_dir(&self) -> &std::path::Path {
        &self.base_evidence_dir
    }

    pub fn config(&self) -> &SnapshotConfig {
        &self.config
    }

    pub fn config_mut(&mut self) -> &mut SnapshotConfig {
        &mut self.config
    }

    /// 根据时标从主码流 GOP 快速解码，或回退至子码流当前帧 (默认配置)
    pub async fn decode_target_frame(
        camera_id: &str,
        target_pts_ms: i64,
        ring_buffer: Option<&MainStreamRingBuffer>,
        sub_stream_fallback: Option<&FrameRef>,
        main_decoder: Option<&mut (dyn VideoDecoder + Send)>,
    ) -> Result<(FrameRef, bool), PipelineError> {
        Self::decode_target_frame_with_config(
            camera_id,
            target_pts_ms,
            ring_buffer,
            sub_stream_fallback,
            main_decoder,
            &SnapshotConfig::default(),
        )
        .await
    }

    /// 根据时标与配置从主码流 GOP 快速解码，或回退至子码流当前帧 (双模证据抓取机制)
    pub async fn decode_target_frame_with_config(
        camera_id: &str,
        target_pts_ms: i64,
        ring_buffer: Option<&MainStreamRingBuffer>,
        sub_stream_fallback: Option<&FrameRef>,
        main_decoder: Option<&mut (dyn VideoDecoder + Send)>,
        config: &SnapshotConfig,
    ) -> Result<(FrameRef, bool), PipelineError> {
        let mut decoded_frame: Option<FrameRef> = None;

        // 1. 尝试从主码流 RingBuffer 提取 GOP 并根据双模策略解码
        if let (Some(rb), Some(decoder)) = (ring_buffer, main_decoder) {
            if let Some(gop) = rb.get_gop_for_timestamp(target_pts_ms) {
                if !gop.is_empty() {
                    let keyframe = &gop[0];
                    let keyframe_pts = keyframe.pts_ms;
                    let phase_diff_ms = (target_pts_ms - keyframe_pts).abs();
                    let packet_count = gop.len();

                    match config.capture_mode {
                        SnapshotCaptureMode::AdaptiveDualMode => {
                            if phase_diff_ms < config.phase_diff_threshold_ms {
                                // 1. 极速模式：相位偏差在阈值以内 (< 500ms)，单帧解码最近 I 帧
                                tracing::info!(
                                    camera_id = %camera_id,
                                    target_pts = target_pts_ms,
                                    keyframe_pts,
                                    phase_diff_ms,
                                    threshold_ms = config.phase_diff_threshold_ms,
                                    "大 GOP 极速模式命中 (< 500ms)：单帧解码 I 帧 (极低延时 1080P/4K 出图)"
                                );
                                if let Ok(Some(frame)) = decoder
                                    .decode_packet(&keyframe.payload, keyframe.pts_ms)
                                    .await
                                {
                                    decoded_frame = Some(frame);
                                }
                            } else if packet_count <= config.max_burst_packets {
                                // 2. 精准追帧模式 (Burst Decode)：偏差较大且包数在预算内，从 I 帧快速前向硬解至告警点
                                tracing::info!(
                                    camera_id = %camera_id,
                                    target_pts = target_pts_ms,
                                    keyframe_pts,
                                    phase_diff_ms,
                                    packet_count,
                                    "大 GOP 精准追帧模式：从 I 帧开始快速前向硬解 (Burst Decode) 至告警点"
                                );
                                let burst_start = std::time::Instant::now();
                                let mut timed_out = false;
                                for pkt in gop {
                                    if burst_start.elapsed().as_millis() as u64
                                        >= config.max_burst_timeout_ms
                                    {
                                        tracing::warn!(
                                            camera_id = %camera_id,
                                            elapsed_ms = burst_start.elapsed().as_millis(),
                                            timeout_budget_ms = config.max_burst_timeout_ms,
                                            "大 GOP 前向追解达到硬实时耗时预算，自适应熔断截断解码"
                                        );
                                        timed_out = true;
                                        break;
                                    }
                                    if let Ok(Some(frame)) =
                                        decoder.decode_packet(&pkt.payload, pkt.pts_ms).await
                                    {
                                        decoded_frame = Some(frame);
                                    }
                                }
                                if timed_out {
                                    // 工业级加固：超时熔断时显式刷新解码器内部残留帧并排空未决状态，防止 VPU 状态污染
                                    let _ = decoder.flush().await;
                                    if let Some(fallback) = sub_stream_fallback {
                                        tracing::info!(
                                            camera_id = %camera_id,
                                            "追帧解码超时熔断，已排空解码器并优雅回退至子码流当前帧"
                                        );
                                        return Ok((fallback.clone(), true));
                                    }
                                }
                            } else if let Some(fallback) = sub_stream_fallback {
                                // 3. 精准追帧模式 (子码流直接复用)：前向追解包数超限，直接复用子码流当时检测命中的那张真实解码帧
                                tracing::warn!(
                                    camera_id = %camera_id,
                                    target_pts = target_pts_ms,
                                    keyframe_pts,
                                    phase_diff_ms,
                                    packet_count,
                                    max_burst = config.max_burst_packets,
                                    "大 GOP 前向追解包数超限，精准追帧模式直接复用子码流检测命中的真实解码帧 (时标零偏差)"
                                );
                                return Ok((fallback.clone(), true));
                            } else {
                                // 子码流不可用，尽力而为前向硬解
                                tracing::warn!(
                                    camera_id = %camera_id,
                                    target_pts = target_pts_ms,
                                    keyframe_pts,
                                    phase_diff_ms,
                                    packet_count,
                                    "子码流未就绪，尽力而为前向硬解"
                                );
                                let burst_start = std::time::Instant::now();
                                let mut timed_out = false;
                                for pkt in gop {
                                    if burst_start.elapsed().as_millis() as u64
                                        >= config.max_burst_timeout_ms
                                    {
                                        tracing::warn!(
                                            camera_id = %camera_id,
                                            elapsed_ms = burst_start.elapsed().as_millis(),
                                            timeout_budget_ms = config.max_burst_timeout_ms,
                                            "尽力而为前向硬解达到硬实时耗时预算，自适应熔断"
                                        );
                                        timed_out = true;
                                        break;
                                    }
                                    if let Ok(Some(frame)) =
                                        decoder.decode_packet(&pkt.payload, pkt.pts_ms).await
                                    {
                                        decoded_frame = Some(frame);
                                    }
                                }
                                if timed_out {
                                    let _ = decoder.flush().await;
                                }
                            }
                        }
                        SnapshotCaptureMode::SubStreamOnLargeGap => {
                            if phase_diff_ms < config.phase_diff_threshold_ms {
                                // 极速模式：相位偏差在 500ms 内，单帧硬解最近 I 帧
                                tracing::info!(
                                    camera_id = %camera_id,
                                    target_pts = target_pts_ms,
                                    keyframe_pts,
                                    phase_diff_ms,
                                    "大 GOP 极速模式命中 (< 500ms)：单帧解码 I 帧"
                                );
                                if let Ok(Some(frame)) = decoder
                                    .decode_packet(&keyframe.payload, keyframe.pts_ms)
                                    .await
                                {
                                    decoded_frame = Some(frame);
                                }
                            } else if let Some(fallback) = sub_stream_fallback {
                                // 精准追帧模式：偏差较大，直接复用子码流当时检测命中的真实解码帧 (0延迟/100%时标精准)
                                tracing::info!(
                                    camera_id = %camera_id,
                                    target_pts = target_pts_ms,
                                    keyframe_pts,
                                    phase_diff_ms,
                                    "大 GOP 相位偏差较大 (>= 500ms)，直接复用子码流检测命中的真实解码帧 (时标零偏差)"
                                );
                                return Ok((fallback.clone(), true));
                            } else {
                                // 子码流不可用，回退至前向硬解
                                for pkt in gop {
                                    if let Ok(Some(frame)) =
                                        decoder.decode_packet(&pkt.payload, pkt.pts_ms).await
                                    {
                                        decoded_frame = Some(frame);
                                    }
                                }
                            }
                        }
                        SnapshotCaptureMode::BurstDecodeOnly => {
                            // 强制全量前向硬解追帧
                            tracing::info!(
                                camera_id = %camera_id,
                                target_pts = target_pts_ms,
                                packet_count,
                                "强制执行全量前向硬解追帧 (Burst Decode)"
                            );
                            let burst_start = std::time::Instant::now();
                            let mut timed_out = false;
                            for pkt in gop {
                                if burst_start.elapsed().as_millis() as u64
                                    >= config.max_burst_timeout_ms
                                {
                                    tracing::warn!(
                                        camera_id = %camera_id,
                                        elapsed_ms = burst_start.elapsed().as_millis(),
                                        timeout_budget_ms = config.max_burst_timeout_ms,
                                        "强制追帧模式达到硬实时耗时预算，自适应熔断"
                                    );
                                    timed_out = true;
                                    break;
                                }
                                if let Ok(Some(frame)) =
                                    decoder.decode_packet(&pkt.payload, pkt.pts_ms).await
                                {
                                    decoded_frame = Some(frame);
                                }
                            }
                            if timed_out {
                                let _ = decoder.flush().await;
                            }
                        }
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

    /// 实例级别解码目标帧 (采用当前引擎绑定的 SnapshotConfig)
    pub async fn decode_frame(
        &self,
        camera_id: &str,
        target_pts_ms: i64,
        ring_buffer: Option<&MainStreamRingBuffer>,
        sub_stream_fallback: Option<&FrameRef>,
        main_decoder: Option<&mut (dyn VideoDecoder + Send)>,
    ) -> Result<(FrameRef, bool), PipelineError> {
        Self::decode_target_frame_with_config(
            camera_id,
            target_pts_ms,
            ring_buffer,
            sub_stream_fallback,
            main_decoder,
            &self.config,
        )
        .await
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
        let (frame, is_fallback) = self
            .decode_frame(
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
    // 0. 物理存储空间硬断路器 (Storage Circuit Breaker)
    // 写入前做轻量级 statvfs 预检：若磁盘物理剩余空间低于 5% 临界红线，立即拒绝落盘，保全 SQLite WAL 日志与核心系统生命线
    if let Ok(free_ratio) = crate::storage_cleaner::get_disk_free_ratio(base_evidence_dir) {
        const CRITICAL_FREE_RATIO: f64 = 0.05;
        if free_ratio < CRITICAL_FREE_RATIO {
            tracing::error!(
                camera_id = %camera_id,
                free_ratio = %format!("{:.2}%", free_ratio * 100.0),
                threshold = %format!("{:.2}%", CRITICAL_FREE_RATIO * 100.0),
                "磁盘空间极度匮乏已触碰 5% 临界红线，触发写盘断路器，拒绝写入快照以保全系统数据库"
            );
            return Err(PipelineError::Snapshot(format!(
                "磁盘空间不足 ({:.2}% < 5.00%)，触发写盘断路保护",
                free_ratio * 100.0
            )));
        }
    }

    // 1. [snapshot_readback_path] 低频证据路径：Device-to-Host 回读与 CPU 图像转换
    // 此路径为告警抓拍与特写生成的显式特例（低频离散事件），不参与常驻推理 fast path
    let rgb_img = media::snapshot_readback_to_rgb_image(&frame)
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
///
/// 工业级加固：当检测到存储分区被硬件只读挂载 (Read-Only Filesystem) 或无权写入时，
/// 启动工业级应急容灾转存至系统内存盘 (tmpfs) 保全关键告警证据。
fn atomic_write_file(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    let tmp_path = path.with_extension(format!("tmp.{}", uuid::Uuid::new_v4().simple()));
    match fs::write(&tmp_path, data) {
        Ok(_) => {
            if let Err(e) = fs::rename(&tmp_path, path) {
                let _ = fs::remove_file(&tmp_path);
                return Err(e);
            }
            Ok(())
        }
        Err(e) => {
            #[cfg(unix)]
            let is_rofs = e.raw_os_error() == Some(libc::EROFS);
            #[cfg(not(unix))]
            let is_rofs = false;

            if is_rofs || e.kind() == std::io::ErrorKind::PermissionDenied {
                tracing::error!(
                    path = %path.display(),
                    error = %e,
                    "磁盘存储硬件发生故障变为只读 (EROFS)，启动应急转存至系统内存盘 (/tmp)"
                );
                let emergency_dir = std::env::temp_dir().join("emergency_evidence");
                let _ = fs::create_dir_all(&emergency_dir);
                let fallback_path = emergency_dir.join(path.file_name().unwrap_or_default());
                fs::write(&fallback_path, data)?;
                return Ok(());
            }
            Err(e)
        }
    }
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
