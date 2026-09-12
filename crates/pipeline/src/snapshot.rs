//! 靶向精准高清抽帧与异步 JPEG 编码引擎
//!
//! 1. 告警触发时，按时标 T 从 MainStreamRingBuffer 索引前置 I 帧并快进解码出单帧 1080P/4K 原图；
//! 2. 若主码流断线、无可用 GOP 或 VPU 配额不足，自动平滑复用已解码候选帧，保证 100% 不漏图；
//! 3. 将图像色彩空间转换、扩边裁剪、JPEG 压缩与文件落盘卸载至 Dedicated Blocking 线程池，杜绝阻塞 Tokio Worker；
//! 4. 产物落盘至 `var/data/evidence/{camera_id}/`。

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{
    AtomicBool, AtomicI64, AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering,
};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[cfg(test)]
use image::RgbImage;
use media::decoder::{DecodeDeliveryPolicy, VideoDecoder};
use media::ring_buffer::MainStreamRingBuffer;
use media::SnapEncoder;
use tokio::sync::oneshot;
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
    /// 是否使用了子码流候选帧（历史字段名；主码流 decoded_ring 复用不会置 true）
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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotConfig {
    /// 极速单帧与精准追帧模式的时间戳相位差阈值（毫秒，默认 500ms）
    pub phase_diff_threshold_ms: i64,
    /// 前向追帧硬解的最大允许包数量（默认 30 包，约 1.2s GOP）
    pub max_burst_packets: usize,
    /// 前向追帧硬解的最大允许耗时预算（毫秒，默认 80ms，防止大 GOP 霸占硬件 VPU）
    pub max_burst_timeout_ms: u64,
    /// 抓拍策略模式
    pub capture_mode: SnapshotCaptureMode,

    // ── 新增：JPEG 编码质量（按码流类型区分） ──
    /// 主码流全景 JPEG 质量 (1-100)，默认 90（高清取证）
    #[serde(default = "default_main_panoramic_quality")]
    pub main_stream_panoramic_quality: u8,
    /// 主码流特写 JPEG 质量 (1-100)，默认 95（高清取证）
    #[serde(default = "default_main_crop_quality")]
    pub main_stream_crop_quality: u8,
    /// 子码流全景 JPEG 质量 (1-100)，默认 80（低功耗场景）
    #[serde(default = "default_sub_panoramic_quality")]
    pub sub_stream_panoramic_quality: u8,
    /// 子码流特写 JPEG 质量 (1-100)，默认 85（低功耗场景）
    #[serde(default = "default_sub_crop_quality")]
    pub sub_stream_crop_quality: u8,
    /// 设备侧裁剪边界扩展比例，默认 0.1 (10%)
    #[serde(default = "default_crop_padding_ratio")]
    pub crop_padding_ratio: f32,
}

fn default_main_panoramic_quality() -> u8 {
    90
}
fn default_main_crop_quality() -> u8 {
    95
}
fn default_sub_panoramic_quality() -> u8 {
    80
}
fn default_sub_crop_quality() -> u8 {
    85
}
fn default_crop_padding_ratio() -> f32 {
    0.1
}

impl Default for SnapshotConfig {
    fn default() -> Self {
        Self {
            phase_diff_threshold_ms: 500,
            max_burst_packets: 30,
            max_burst_timeout_ms: 80,
            capture_mode: SnapshotCaptureMode::AdaptiveDualMode,
            main_stream_panoramic_quality: default_main_panoramic_quality(),
            main_stream_crop_quality: default_main_crop_quality(),
            sub_stream_panoramic_quality: default_sub_panoramic_quality(),
            sub_stream_crop_quality: default_sub_crop_quality(),
            crop_padding_ratio: default_crop_padding_ratio(),
        }
    }
}

impl SnapshotConfig {
    pub fn validate(&self) -> Result<(), String> {
        for (name, val) in [
            (
                "mainStreamPanoramicQuality",
                self.main_stream_panoramic_quality,
            ),
            ("mainStreamCropQuality", self.main_stream_crop_quality),
            (
                "subStreamPanoramicQuality",
                self.sub_stream_panoramic_quality,
            ),
            ("subStreamCropQuality", self.sub_stream_crop_quality),
        ] {
            if val == 0 || val > 100 {
                return Err(format!("{name} 必须在 1-100 范围内"));
            }
        }
        if self.phase_diff_threshold_ms < 0 {
            return Err("phaseDiffThresholdMs 不能为负数".into());
        }
        if self.max_burst_packets == 0 {
            return Err("maxBurstPackets 必须大于 0".into());
        }
        if !(0.0..=0.5).contains(&self.crop_padding_ratio) {
            return Err("cropPaddingRatio 必须在 0.0-0.5 范围内".into());
        }
        Ok(())
    }

    /// 根据当前抓拍帧是否来源于子码流（或子码流保底），选择对应的全景与特写 JPEG 质量 (panoramic_q, crop_q)
    pub fn quality_for_stream(&self, is_sub_stream: bool) -> (u8, u8) {
        if is_sub_stream {
            (
                self.sub_stream_panoramic_quality,
                self.sub_stream_crop_quality,
            )
        } else {
            (
                self.main_stream_panoramic_quality,
                self.main_stream_crop_quality,
            )
        }
    }
}

struct AtomicSnapshotConfig {
    version: AtomicU64,
    phase_diff_threshold_ms: AtomicI64,
    max_burst_packets: AtomicUsize,
    max_burst_timeout_ms: AtomicU64,
    capture_mode: AtomicU8,
    main_stream_panoramic_quality: AtomicU8,
    main_stream_crop_quality: AtomicU8,
    sub_stream_panoramic_quality: AtomicU8,
    sub_stream_crop_quality: AtomicU8,
    crop_padding_ratio_bits: AtomicU32,
}

impl AtomicSnapshotConfig {
    fn new(config: SnapshotConfig) -> Self {
        Self {
            version: AtomicU64::new(0),
            phase_diff_threshold_ms: AtomicI64::new(config.phase_diff_threshold_ms),
            max_burst_packets: AtomicUsize::new(config.max_burst_packets),
            max_burst_timeout_ms: AtomicU64::new(config.max_burst_timeout_ms),
            capture_mode: AtomicU8::new(Self::encode_capture_mode(config.capture_mode)),
            main_stream_panoramic_quality: AtomicU8::new(config.main_stream_panoramic_quality),
            main_stream_crop_quality: AtomicU8::new(config.main_stream_crop_quality),
            sub_stream_panoramic_quality: AtomicU8::new(config.sub_stream_panoramic_quality),
            sub_stream_crop_quality: AtomicU8::new(config.sub_stream_crop_quality),
            crop_padding_ratio_bits: AtomicU32::new(config.crop_padding_ratio.to_bits()),
        }
    }

    fn load(&self) -> SnapshotConfig {
        loop {
            let version = self.version.load(Ordering::Acquire);
            if version & 1 != 0 {
                std::hint::spin_loop();
                continue;
            }

            let config = SnapshotConfig {
                phase_diff_threshold_ms: self.phase_diff_threshold_ms.load(Ordering::Relaxed),
                max_burst_packets: self.max_burst_packets.load(Ordering::Relaxed),
                max_burst_timeout_ms: self.max_burst_timeout_ms.load(Ordering::Relaxed),
                capture_mode: Self::decode_capture_mode(self.capture_mode.load(Ordering::Relaxed)),
                main_stream_panoramic_quality: self
                    .main_stream_panoramic_quality
                    .load(Ordering::Relaxed),
                main_stream_crop_quality: self.main_stream_crop_quality.load(Ordering::Relaxed),
                sub_stream_panoramic_quality: self
                    .sub_stream_panoramic_quality
                    .load(Ordering::Relaxed),
                sub_stream_crop_quality: self.sub_stream_crop_quality.load(Ordering::Relaxed),
                crop_padding_ratio: f32::from_bits(
                    self.crop_padding_ratio_bits.load(Ordering::Relaxed),
                ),
            };
            if self.version.load(Ordering::Acquire) == version {
                return config;
            }
        }
    }

    fn store(&self, config: SnapshotConfig) {
        let version = loop {
            let current = self.version.load(Ordering::Acquire);
            if current & 1 != 0 {
                std::hint::spin_loop();
                continue;
            }
            if self
                .version
                .compare_exchange_weak(
                    current,
                    current.wrapping_add(1),
                    Ordering::Acquire,
                    Ordering::Relaxed,
                )
                .is_ok()
            {
                break current;
            }
        };

        self.phase_diff_threshold_ms
            .store(config.phase_diff_threshold_ms, Ordering::Relaxed);
        self.max_burst_packets
            .store(config.max_burst_packets, Ordering::Relaxed);
        self.max_burst_timeout_ms
            .store(config.max_burst_timeout_ms, Ordering::Relaxed);
        self.capture_mode.store(
            Self::encode_capture_mode(config.capture_mode),
            Ordering::Relaxed,
        );
        self.main_stream_panoramic_quality
            .store(config.main_stream_panoramic_quality, Ordering::Relaxed);
        self.main_stream_crop_quality
            .store(config.main_stream_crop_quality, Ordering::Relaxed);
        self.sub_stream_panoramic_quality
            .store(config.sub_stream_panoramic_quality, Ordering::Relaxed);
        self.sub_stream_crop_quality
            .store(config.sub_stream_crop_quality, Ordering::Relaxed);
        self.crop_padding_ratio_bits
            .store(config.crop_padding_ratio.to_bits(), Ordering::Relaxed);
        self.version
            .store(version.wrapping_add(2), Ordering::Release);
    }

    const fn encode_capture_mode(mode: SnapshotCaptureMode) -> u8 {
        match mode {
            SnapshotCaptureMode::AdaptiveDualMode => 0,
            SnapshotCaptureMode::SubStreamOnLargeGap => 1,
            SnapshotCaptureMode::BurstDecodeOnly => 2,
        }
    }

    const fn decode_capture_mode(value: u8) -> SnapshotCaptureMode {
        match value {
            1 => SnapshotCaptureMode::SubStreamOnLargeGap,
            2 => SnapshotCaptureMode::BurstDecodeOnly,
            _ => SnapshotCaptureMode::AdaptiveDualMode,
        }
    }
}

const SNAPSHOT_WORK_QUEUE_CAPACITY: usize = 8;
const SNAPSHOT_WORKER_POLL_INTERVAL: Duration = Duration::from_millis(50);
const SNAPSHOT_WORKER_SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(500);

struct SnapshotWork {
    camera_id: String,
    frame: FrameRef,
    target_bbox: Option<BoundingBox>,
    is_sub_stream: bool,
    base_evidence_dir: PathBuf,
    config: SnapshotConfig,
    reply: oneshot::Sender<Result<SnapshotResult, PipelineError>>,
}

struct SnapshotWorker {
    tx: Option<SyncSender<SnapshotWork>>,
    cancel: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl SnapshotWorker {
    fn new() -> Result<Self, String> {
        let (tx, rx) = mpsc::sync_channel::<SnapshotWork>(SNAPSHOT_WORK_QUEUE_CAPACITY);
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let join = thread::Builder::new()
            .name("snapshot-worker".into())
            .spawn(move || {
                // 硬件 context 必须在固定线程创建，随后所有 MPP/RGA 调用都由该线程串行执行。
                let encoder = SnapEncoder::try_new();
                tracing::info!(encoder = %encoder.name(), "快照专用编码线程已就绪");
                while !worker_cancel.load(Ordering::Acquire) {
                    let work = match rx.recv_timeout(SNAPSHOT_WORKER_POLL_INTERVAL) {
                        Ok(work) => work,
                        Err(RecvTimeoutError::Timeout) => continue,
                        Err(RecvTimeoutError::Disconnected) => break,
                    };
                    if worker_cancel.load(Ordering::Acquire) {
                        break;
                    }
                    let result = encode_and_save_snapshot(
                        &work.camera_id,
                        work.frame,
                        work.target_bbox,
                        &work.base_evidence_dir,
                        work.is_sub_stream,
                        &work.config,
                        &encoder,
                    );
                    let _ = work.reply.send(result);
                }
                tracing::info!("快照专用编码线程已退出");
            })
            .map_err(|error| format!("创建快照专用线程失败: {error}"))?;
        Ok(Self {
            tx: Some(tx),
            cancel,
            join: Some(join),
        })
    }
}

impl std::fmt::Debug for SnapshotWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SnapshotWorker")
            .field("queue_capacity", &SNAPSHOT_WORK_QUEUE_CAPACITY)
            .field("running", &self.join.is_some())
            .finish()
    }
}

impl Drop for SnapshotWorker {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        self.tx.take();

        let Some(join) = self.join.take() else {
            return;
        };
        if join.thread().id() == thread::current().id() {
            return;
        }

        let deadline = Instant::now() + SNAPSHOT_WORKER_SHUTDOWN_TIMEOUT;
        while !join.is_finished() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }

        if join.is_finished() {
            if join.join().is_err() {
                tracing::warn!("快照专用编码线程异常退出");
            }
        } else {
            tracing::warn!(
                timeout_ms = SNAPSHOT_WORKER_SHUTDOWN_TIMEOUT.as_millis(),
                "快照专用编码线程未在期限内退出，已分离线程以避免阻塞进程停机"
            );
        }
    }
}

/// 快照抓拍引擎
pub struct SnapshotEngine {
    base_evidence_dir: PathBuf,
    config: AtomicSnapshotConfig,
    /// 快照编码工作线程；队列固定容量，避免 Tokio blocking pool 无界堆积。
    worker: Option<SnapshotWorker>,
}

impl std::fmt::Debug for SnapshotEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SnapshotEngine")
            .field("base_evidence_dir", &self.base_evidence_dir)
            .field("config", &self.config())
            .field("worker", &self.worker)
            .finish()
    }
}

impl SnapshotEngine {
    pub fn new(base_evidence_dir: impl Into<PathBuf>) -> Self {
        Self::with_config(base_evidence_dir, SnapshotConfig::default())
    }

    pub fn with_config(base_evidence_dir: impl Into<PathBuf>, config: SnapshotConfig) -> Self {
        let config = match config.validate() {
            Ok(()) => config,
            Err(error) => {
                tracing::error!(%error, "快照配置非法，恢复内置默认值");
                SnapshotConfig::default()
            }
        };
        let worker = match SnapshotWorker::new() {
            Ok(worker) => Some(worker),
            Err(error) => {
                tracing::error!(%error, "快照专用线程不可用，抓拍请求将失败");
                None
            }
        };
        Self {
            base_evidence_dir: base_evidence_dir.into(),
            worker,
            config: AtomicSnapshotConfig::new(config),
        }
    }

    pub fn base_evidence_dir(&self) -> &std::path::Path {
        &self.base_evidence_dir
    }

    pub fn config(&self) -> SnapshotConfig {
        self.config.load()
    }

    pub fn update_config(&self, new_config: SnapshotConfig) -> Result<(), String> {
        new_config.validate()?;
        self.config.store(new_config);
        Ok(())
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
            // 快照抽帧采用无损反压交付策略，确保大 GOP 追解帧一帧不漏
            decoder.set_delivery_policy(DecodeDeliveryPolicy::LosslessBackpressure);

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
                                            "追帧解码超时熔断，已排空解码器并优雅回退至已解码候选帧"
                                        );
                                        return Ok((fallback.clone(), true));
                                    }
                                }
                            } else if let Some(fallback) = sub_stream_fallback {
                                // 3. 精准追帧模式 (已解帧直接复用)：前向追解包数超限，直接复用当时检测命中的那张真实解码候选帧
                                tracing::warn!(
                                    camera_id = %camera_id,
                                    target_pts = target_pts_ms,
                                    keyframe_pts,
                                    phase_diff_ms,
                                    packet_count,
                                    max_burst = config.max_burst_packets,
                                    "大 GOP 前向追解包数超限，精准追帧模式直接复用已解码候选帧 (时标零偏差)"
                                );
                                return Ok((fallback.clone(), true));
                            } else {
                                // 候选帧不可用，尽力而为前向硬解
                                tracing::warn!(
                                    camera_id = %camera_id,
                                    target_pts = target_pts_ms,
                                    keyframe_pts,
                                    phase_diff_ms,
                                    packet_count,
                                    "已解码候选帧未就绪，尽力而为前向硬解"
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
                                // 精准追帧模式：偏差较大，直接复用当时检测命中的真实解码候选帧 (0延迟/100%时标精准)
                                tracing::info!(
                                    camera_id = %camera_id,
                                    target_pts = target_pts_ms,
                                    keyframe_pts,
                                    phase_diff_ms,
                                    "大 GOP 相位偏差较大 (>= 500ms)，直接复用已解码候选帧 (时标零偏差)"
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

        // 2. 若主流未能解码出有效帧，自动回退借用已解码备用帧
        if let Some(frame) = decoded_frame {
            Ok((frame, false))
        } else if let Some(fallback) = sub_stream_fallback {
            tracing::warn!(
                camera_id = %camera_id,
                target_pts = target_pts_ms,
                "主码流靶向抽帧未就绪，平滑降级使用已解码备用帧"
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
        let config = self.config();
        Self::decode_target_frame_with_config(
            camera_id,
            target_pts_ms,
            ring_buffer,
            sub_stream_fallback,
            main_decoder,
            &config,
        )
        .await
    }

    /// 将快照作业提交至固定容量的专用编码线程。
    pub async fn save_snapshot_async(
        &self,
        camera_id: &str,
        frame: FrameRef,
        target_bbox: Option<BoundingBox>,
        is_sub_stream: bool,
    ) -> Result<SnapshotResult, PipelineError> {
        let worker = self
            .worker
            .as_ref()
            .ok_or_else(|| PipelineError::Snapshot("快照专用编码线程不可用".into()))?;
        let (reply, result) = oneshot::channel();
        let work = SnapshotWork {
            camera_id: camera_id.to_string(),
            frame,
            target_bbox,
            is_sub_stream,
            base_evidence_dir: self.base_evidence_dir.clone(),
            config: self.config(),
            reply,
        };
        let sender = worker
            .tx
            .as_ref()
            .ok_or_else(|| PipelineError::Snapshot("快照专用线程已关闭".into()))?;
        match sender.try_send(work) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                tracing::warn!(camera_id, "快照专用队列已满，丢弃本次抓拍");
                return Err(PipelineError::Snapshot("快照工作队列已满".into()));
            }
            Err(TrySendError::Disconnected(_)) => {
                return Err(PipelineError::Snapshot("快照专用线程已退出".into()));
            }
        }
        result
            .await
            .map_err(|_| PipelineError::Snapshot("快照专用线程未返回结果".into()))?
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
    config: &SnapshotConfig,
    encoder: &SnapEncoder,
) -> Result<SnapshotResult, PipelineError> {
    // 0. 物理存储空间与 Inode 全维度硬断路器 (Storage Circuit Breaker)
    // 写入前做轻量级 statvfs 预检：根据健康决策评估硬熔断与紧急抓拍降级
    if let Ok(stat) = crate::storage_cleaner::stat_fs(base_evidence_dir) {
        let decision = crate::storage_cleaner::StorageCircuitBreaker::evaluate_with_defaults(&stat);
        if decision.is_circuit_broken {
            let reason = decision
                .reason
                .as_deref()
                .unwrap_or("存储资源严重匮乏触发熔断");
            tracing::error!(
                %camera_id,
                %reason,
                "触发存储写盘硬熔断保护，拒绝写入快照以保全系统数据库核心生命线"
            );
            return Err(PipelineError::Snapshot(format!(
                "触发写盘断路保护: {reason}"
            )));
        }

        // 紧急严重水位：抑制普通抓拍，仅放行携带靶向检测目标的违规告警凭据
        if !decision.allow_normal_capture && target_bbox.is_none() {
            let reason = decision
                .reason
                .as_deref()
                .unwrap_or("存储处于紧急水位，已抑制普通抓拍");
            tracing::warn!(
                %camera_id,
                %reason,
                "存储处于紧急水位：自动抑制普通抓拍写入，保全关键违规告警证据"
            );
            return Err(PipelineError::Snapshot(format!(
                "存储紧急降级抑制: {reason}"
            )));
        }
    }

    let (panoramic_q, crop_q) = config.quality_for_stream(is_fallback);

    // 1. [snapshot_readback_path] 完成全景与特写编码后再写盘。
    // CPU fallback 会复用同一次 D2H readback，硬件路径也保持单线程串行。
    let (full_jpeg_bytes, crop_jpeg_bytes) = encoder
        .encode_full_and_crop(
            &frame,
            target_bbox,
            config.crop_padding_ratio,
            panoramic_q,
            crop_q,
        )
        .map_err(|e| PipelineError::Snapshot(format!("快照 JPEG 编码失败: {e}")))?;
    let crop_jpeg_bytes = crop_jpeg_bytes.unwrap_or_else(|| full_jpeg_bytes.clone());

    let width = frame.width;
    let height = frame.height;

    // 2. 生成唯一图片 ID 与落盘相对路径。只有两个 bitstream 都准备好后才创建产物。
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
    let file_size_bytes = full_jpeg_bytes.len();

    atomic_write_file(&full_path, &full_jpeg_bytes)
        .map_err(|e| PipelineError::Snapshot(format!("写入全景抓拍图片失败: {e}")))?;
    if let Err(error) = atomic_write_file(&crop_path, &crop_jpeg_bytes) {
        // 数据库尚未记录任何文件；第二个文件失败时撤销第一个文件，避免孤儿证据。
        let _ = fs::remove_file(&full_path);
        return Err(PipelineError::Snapshot(format!(
            "写入特写抠图图片失败: {error}"
        )));
    }

    let image_rel_path = format!("{camera_id}/{full_filename}");
    let crop_image_rel_path = format!("{camera_id}/{crop_filename}");

    tracing::info!(
        camera_id = %camera_id,
        image_id = %image_id,
        crop_id = %crop_image_id,
        encoder = %encoder.name(),
        file_size_bytes,
        is_fallback,
        "靶向证据高清抓拍完成 (dedicated snapshot worker)"
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

/// Test-only helper: wraps `media::crop_rgb_with_padding` for test assertions.
#[inline]
#[cfg(test)]
fn crop_with_padding(img: &RgbImage, bbox: BoundingBox, padding_ratio: f32) -> RgbImage {
    media::crop_rgb_with_padding(img, bbox, padding_ratio)
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

        // CPU crop 保持 32px 最小下限；RGA 硬件路径单独执行更严格的尺寸校验
        assert!(cropped.width() >= 32 && cropped.width() <= 36);
        assert!(cropped.height() >= 32 && cropped.height() <= 36);

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

    #[test]
    fn test_snapshot_config_defaults_and_validation() {
        let config = SnapshotConfig::default();
        assert_eq!(config.main_stream_panoramic_quality, 90);
        assert_eq!(config.main_stream_crop_quality, 95);
        assert_eq!(config.sub_stream_panoramic_quality, 80);
        assert_eq!(config.sub_stream_crop_quality, 85);
        assert!((config.crop_padding_ratio - 0.1).abs() < f32::EPSILON);
        assert!(config.validate().is_ok());

        // 质量溢出非法校验
        let mut invalid_cfg = config.clone();
        invalid_cfg.main_stream_panoramic_quality = 101;
        assert!(invalid_cfg.validate().is_err());

        // 扩边比例超界非法校验
        let mut invalid_padding = config.clone();
        invalid_padding.crop_padding_ratio = 0.6;
        assert!(invalid_padding.validate().is_err());

        // 码流路由参数验证
        assert_eq!(config.quality_for_stream(false), (90, 95));
        assert_eq!(config.quality_for_stream(true), (80, 85));
    }

    #[test]
    fn test_snapshot_config_hot_update_is_atomic() {
        let temp_dir = std::env::temp_dir().join(format!(
            "test_snapshot_config_{}",
            uuid::Uuid::new_v4().simple()
        ));
        let engine = SnapshotEngine::new(temp_dir);
        let mut updated = engine.config();
        updated.main_stream_panoramic_quality = 73;

        engine
            .update_config(updated.clone())
            .expect("合法配置应支持热更新");
        assert_eq!(engine.config().main_stream_panoramic_quality, 73);

        let mut invalid = updated;
        invalid.main_stream_panoramic_quality = 0;
        assert!(engine.update_config(invalid).is_err());
        assert_eq!(engine.config().main_stream_panoramic_quality, 73);
    }

    #[test]
    fn test_atomic_snapshot_config_never_returns_mixed_versions() {
        let first = SnapshotConfig {
            main_stream_panoramic_quality: 11,
            main_stream_crop_quality: 12,
            ..Default::default()
        };
        let mut second = first.clone();
        second.main_stream_panoramic_quality = 91;
        second.main_stream_crop_quality = 92;

        let store = std::sync::Arc::new(AtomicSnapshotConfig::new(first.clone()));
        let writer_store = std::sync::Arc::clone(&store);
        let writer = std::thread::spawn(move || {
            for index in 0..10_000 {
                writer_store.store(if index & 1 == 0 {
                    second.clone()
                } else {
                    first.clone()
                });
            }
        });

        for _ in 0..10_000 {
            let snapshot = store.load();
            assert!(matches!(
                (
                    snapshot.main_stream_panoramic_quality,
                    snapshot.main_stream_crop_quality
                ),
                (11, 12) | (91, 92)
            ));
        }
        writer.join().expect("配置写线程应正常退出");
    }

    #[tokio::test]
    async fn test_snapshot_worker_queue_overload_protection() {
        let temp_dir = std::env::temp_dir().join(format!(
            "test_evidence_ovl_{}",
            uuid::Uuid::new_v4().simple()
        ));
        let engine = SnapshotEngine::new(&temp_dir);

        let width = 320;
        let height = 240;
        let nv12_size = (width * height * 3 / 2) as usize;
        let dummy_nv12: std::sync::Arc<[u8]> = vec![128u8; nv12_size].into();

        let make_frame = || {
            FrameRef::new(
                "cam_ovl".to_string(),
                1741100000000,
                width,
                height,
                StrideInfo::new(width, height),
                PixelFormat::Nv12,
                FrameHandle::Host(dummy_nv12.clone()),
            )
        };

        // 并发提交远超过队列容量 (8) 的快照任务，验证背压防护与有界通道拒绝
        let engine = std::sync::Arc::new(engine);
        let mut join_set = tokio::task::JoinSet::new();
        for _ in 0..24 {
            let frame = make_frame();
            let engine_ref = engine.clone();
            join_set.spawn(async move {
                engine_ref
                    .save_snapshot_async(
                        "cam_ovl",
                        frame,
                        Some(BoundingBox::new(0.1, 0.1, 0.5, 0.5)),
                        false,
                    )
                    .await
            });
        }

        let mut overloaded = 0;
        while let Some(res) = join_set.join_next().await {
            if let Ok(Err(PipelineError::Snapshot(msg))) = res {
                if msg.contains("快照工作队列已满") {
                    overloaded += 1;
                }
            }
        }

        // 必然有部分任务由于超过有界队列阈值被及时拒绝，杜绝了无限内存泄漏
        assert!(overloaded > 0, "队列饱和时必须返回队列已满错误");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_snapshot_saves_both_full_and_crop_files_transactionally() {
        let temp_dir = std::env::temp_dir().join(format!(
            "test_evidence_clean_{}",
            uuid::Uuid::new_v4().simple()
        ));
        let cam_dir = temp_dir.join("cam_clean");
        fs::create_dir_all(&cam_dir).expect("创建测试证据目录失败");

        let width = 64;
        let height = 64;
        let nv12_size = (width * height * 3 / 2) as usize;
        let frame = FrameRef::new(
            "cam_clean".to_string(),
            1741100000000,
            width,
            height,
            StrideInfo::new(width, height),
            PixelFormat::Nv12,
            FrameHandle::Host(vec![128u8; nv12_size].into()),
        );

        let encoder = SnapEncoder::cpu_only();
        let config = SnapshotConfig::default();

        let res = encode_and_save_snapshot(
            "cam_clean",
            frame,
            Some(BoundingBox::new(0.2, 0.2, 0.6, 0.6)),
            &temp_dir,
            false,
            &config,
            &encoder,
        );
        let res = res.expect("编码与落盘必须成功");

        let full_file = temp_dir.join(&res.image_rel_path);
        let crop_file = temp_dir.join(&res.crop_image_rel_path);
        assert!(full_file.is_file(), "全景文件必须存在");
        assert!(crop_file.is_file(), "特写文件必须存在");

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
