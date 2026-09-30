//! 事件录像专用 OS Worker：压缩流封装与落盘
//!
//! - 通过 [`ConsumerKind::Recording`] 订阅主码流，`recv_blocking` 同步消费
//! - 内存 `PreCaptureRingBuffer` 持续滚动，事件触发时 flush 出前置片段
//! - fMP4 写入与 `fsync` 全部在本线程执行，绝不进入 Tokio worker
//!
//! 停机契约：Drop 时关闭 mailbox → 线程收到 `None` 后闭合当前录像 → 有界等待退出。

use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use media::fmp4::FMP4Writer;
use media::pre_capture_ring::{PreCaptureConfig, PreCaptureRingBuffer};
use media::{StreamItem, StreamSubscription};
use types::EncodedPacket;

use super::config::RecordingConfig;

/// 单次 `recv_blocking` 的等待时长（同时作为停机检查间隔）
const RECV_TIMEOUT: Duration = Duration::from_millis(200);
/// 停机有界等待上限
const SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(1500);

/// 录像事件触发信号
#[derive(Debug, Clone)]
pub struct RecordingTrigger {
    /// 事件 ID（用于 `recording_events` 关联）
    pub event_id: String,
    /// 事件类型
    pub event_type: RecordingEventType,
    /// 事件发生时间（UTC 毫秒），与主码流 PTS 同轴
    pub event_time_ms: i64,
}

/// 触发事件类型
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingEventType {
    Alarm,
    Recognition,
}

impl RecordingEventType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Alarm => "alarm",
            Self::Recognition => "recognition",
        }
    }
}

/// 一个已关联的事件（落库用）
#[derive(Debug, Clone)]
pub struct LinkedEvent {
    pub event_id: String,
    pub event_type: RecordingEventType,
    pub event_time_ms: i64,
    /// 事件在录像文件内的偏移（毫秒）
    pub offset_ms: i64,
}

/// 录像闭合后的落库载荷
#[derive(Debug, Clone)]
pub struct FinishedRecording {
    pub camera_id: String,
    pub recording_id: String,
    pub file_path: PathBuf,
    /// 文件内首包 PTS（UTC 毫秒）
    pub start_time_ms: i64,
    /// 文件内末包 PTS（UTC 毫秒）
    pub end_time_ms: i64,
    /// 文件字节数
    pub file_size: u64,
    /// `completed` | `truncated`
    pub status: &'static str,
    /// 编码格式
    pub codec: types::CodecType,
    pub events: Vec<LinkedEvent>,
}

/// 进行中的录像会话
struct ActiveRecording {
    recording_id: String,
    file_path: PathBuf,
    writer: FMP4Writer<BufWriter<std::fs::File>>,
    codec: types::CodecType,
    /// 文件内首包 PTS
    start_pts_ms: i64,
    /// 文件内末包 PTS
    last_pts_ms: i64,
    /// post-capture 截止时刻（单调毫秒）
    deadline_mono_ms: u64,
    /// 已关联事件
    events: Vec<LinkedEvent>,
    /// 是否因 SourceReset 截断
    truncated: bool,
}

/// 录像 Worker 对外句柄
pub struct RecordingWorker {
    thread: Option<std::thread::JoinHandle<()>>,
    stop: Arc<AtomicBool>,
    degraded: Arc<AtomicBool>,
    camera_id: String,
    recordings_written: Arc<AtomicU64>,
    recordings_truncated: Arc<AtomicU64>,
}

impl std::fmt::Debug for RecordingWorker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordingWorker")
            .field("camera", &self.camera_id)
            .field("is_alive", &self.is_alive())
            .field("written", &self.recordings_written())
            .field("truncated", &self.recordings_truncated())
            .finish()
    }
}

impl RecordingWorker {
    /// 启动录像 Worker。
    ///
    /// - `subscription`：`ConsumerKind::Recording` 的 dispatcher 订阅（主码流）
    /// - `trigger_rx`：事件触发信号接收端（由 Pipeline 的 `analysis_event_tx` 桥接）
    /// - `config`：通道级录像配置
    /// - `data_dir`：录像根目录，实际路径为 `{data_dir}/recordings/{camera_id}/{date}/`
    /// - `on_finished`：录像闭合后的落库回调（在 worker 线程内执行）
    pub fn spawn(
        camera_id: impl Into<String>,
        subscription: StreamSubscription,
        trigger_rx: std::sync::mpsc::Receiver<RecordingTrigger>,
        config: RecordingConfig,
        data_dir: PathBuf,
        on_finished: Box<dyn Fn(FinishedRecording) + Send>,
    ) -> Self {
        let camera_id = camera_id.into();
        let stop = Arc::new(AtomicBool::new(false));
        let degraded = Arc::new(AtomicBool::new(false));
        let recordings_written = Arc::new(AtomicU64::new(0));
        let recordings_truncated = Arc::new(AtomicU64::new(0));

        let ctx = WorkerContext {
            camera_id: camera_id.clone(),
            config: config.normalized(),
            data_dir,
            stop: Arc::clone(&stop),
            degraded: Arc::clone(&degraded),
            recordings_written: Arc::clone(&recordings_written),
            recordings_truncated: Arc::clone(&recordings_truncated),
            on_finished,
        };

        let spawned = std::thread::Builder::new()
            .name(format!("rec-{camera_id}"))
            .spawn(move || ctx.run(subscription, trigger_rx));

        match spawned {
            Ok(thread) => Self {
                thread: Some(thread),
                stop,
                degraded,
                camera_id,
                recordings_written,
                recordings_truncated,
            },
            Err(error) => {
                tracing::error!(
                    camera = %camera_id,
                    error = %error,
                    "录像工作线程创建失败，本路录像降级为禁用"
                );
                degraded.store(true, Ordering::Relaxed);
                Self {
                    thread: None,
                    stop,
                    degraded,
                    camera_id,
                    recordings_written,
                    recordings_truncated,
                }
            }
        }
    }

    #[inline]
    pub fn is_alive(&self) -> bool {
        !self.degraded.load(Ordering::Relaxed)
    }

    #[inline]
    pub fn camera_id(&self) -> &str {
        &self.camera_id
    }

    #[inline]
    pub fn recordings_written(&self) -> u64 {
        self.recordings_written.load(Ordering::Relaxed)
    }

    #[inline]
    pub fn recordings_truncated(&self) -> u64 {
        self.recordings_truncated.load(Ordering::Relaxed)
    }

    /// 请求停机（幂等）。线程将在闭合当前录像后退出。
    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Drop for RecordingWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.degraded.store(true, Ordering::Relaxed);

        let Some(thread) = self.thread.take() else {
            return;
        };
        // 停机可能触及磁盘 fsync：不能在异步执行器上阻塞，交给瞬态看护线程有界 join。
        let camera_id = std::mem::take(&mut self.camera_id);
        let reaper_camera = camera_id.clone();
        let spawned = std::thread::Builder::new()
            .name(format!("rec-reap-{camera_id}"))
            .spawn(move || {
                let deadline = std::time::Instant::now() + SHUTDOWN_TIMEOUT;
                while !thread.is_finished() && std::time::Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(10));
                }
                if thread.is_finished() {
                    let _ = thread.join();
                    tracing::debug!(camera = %reaper_camera, "录像工作线程已优雅退出并回收");
                } else {
                    tracing::error!(
                        camera = %reaper_camera,
                        timeout_ms = SHUTDOWN_TIMEOUT.as_millis() as u64,
                        "录像工作线程停机超时（疑似磁盘挂起），隔离句柄受控保活"
                    );
                }
            });
        if let Err(error) = spawned {
            tracing::error!(
                camera = %camera_id,
                error = %error,
                "录像停机看护线程创建失败，已放弃回收录像工作线程"
            );
        }
    }
}

/// Worker 线程上下文（独占，无跨线程共享可变状态）
struct WorkerContext {
    camera_id: String,
    config: RecordingConfig,
    data_dir: PathBuf,
    stop: Arc<AtomicBool>,
    degraded: Arc<AtomicBool>,
    recordings_written: Arc<AtomicU64>,
    recordings_truncated: Arc<AtomicU64>,
    on_finished: Box<dyn Fn(FinishedRecording) + Send>,
}

impl WorkerContext {
    fn run(
        self,
        subscription: StreamSubscription,
        trigger_rx: std::sync::mpsc::Receiver<RecordingTrigger>,
    ) {
        let mut buffer = PreCaptureRingBuffer::new(PreCaptureConfig::from_seconds(
            self.config.pre_capture_seconds,
        ));
        let mut active: Option<ActiveRecording> = None;
        let mut awaiting_keyframe = true;

        loop {
            // 先排空触发事件（即使已请求停机，也要先把在途事件应用完，
            // 否则最后一个事件的合并关联会丢失）
            while let Ok(trigger) = trigger_rx.try_recv() {
                if let Err(error) = self.apply_trigger(&mut active, &mut buffer, trigger) {
                    tracing::error!(camera = %self.camera_id, %error, "事件触发处理失败");
                }
            }

            if self.stop.load(Ordering::Relaxed) {
                break;
            }

            // 再阻塞等待码流（有界超时以检查 stop 与 deadline）
            let item = match subscription.recv_blocking(RECV_TIMEOUT) {
                media::BlockingRecv::Item(item) => item,
                media::BlockingRecv::Timeout => {
                    // 超时：仅检查截止时间，继续循环
                    self.maybe_close_on_deadline(&mut active, &mut buffer);
                    continue;
                }
                media::BlockingRecv::Closed => {
                    // 订阅关闭（StreamSubscription 被 drop）→ 退出
                    break;
                }
            };

            match item {
                StreamItem::Packet(pkt) => {
                    if !is_recordable_video(&pkt) {
                        continue;
                    }
                    if awaiting_keyframe && !pkt.is_keyframe {
                        continue;
                    }
                    awaiting_keyframe = false;
                    self.handle_packet(&mut active, &mut buffer, pkt);
                }
                StreamItem::Replay(snapshot) => {
                    buffer.clear();
                    awaiting_keyframe = false;
                    for packet in snapshot.packets.iter() {
                        if is_recordable_video(packet) {
                            self.handle_packet(&mut active, &mut buffer, Arc::clone(packet));
                        }
                    }
                }
                StreamItem::SourceReset { epoch } => {
                    tracing::info!(
                        camera = %self.camera_id,
                        epoch,
                        "录像 Worker 收到源流重建：截断当前录像并清空前置缓冲"
                    );
                    buffer.clear();
                    awaiting_keyframe = true;
                    if let Some(rec) = active.take() {
                        self.finish(rec, true);
                    }
                }
            }

            // 检查 post-capture 截止
            self.maybe_close_on_deadline(&mut active, &mut buffer);
        }

        // 停机：闭合在途录像
        if let Some(rec) = active.take() {
            tracing::info!(camera = %self.camera_id, "录像 Worker 停机，闭合当前录像");
            self.finish(rec, false);
        }
        self.degraded.store(true, Ordering::Relaxed);
        tracing::info!(camera = %self.camera_id, "录像 Worker 已退出");
    }

    /// 处理单个视频包：状态机推进
    fn handle_packet(
        &self,
        active: &mut Option<ActiveRecording>,
        buffer: &mut PreCaptureRingBuffer,
        pkt: Arc<EncodedPacket>,
    ) {
        match active {
            None => {
                // Idle：仅滚动前置缓冲
                buffer.push(pkt);
            }
            Some(rec) => {
                if let Err(error) = rec.writer.push(&pkt) {
                    tracing::error!(
                        camera = %self.camera_id,
                        recording_id = %rec.recording_id,
                        %error,
                        "fMP4 写入失败，截断当前录像"
                    );
                    if let Some(rec) = active.take() {
                        self.finish(rec, true);
                    }
                    return;
                }
                rec.last_pts_ms = pkt.pts_ms;
            }
        }
    }

    /// 应用事件触发：Idle → 开新录像；Recording → 延长合并
    fn apply_trigger(
        &self,
        active: &mut Option<ActiveRecording>,
        buffer: &mut PreCaptureRingBuffer,
        trigger: RecordingTrigger,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let post_ms = i64::from(self.config.post_capture_seconds) * 1000;
        let new_deadline = monotonic_ms() + post_ms as u64;

        match active {
            None => {
                // 从缓冲导出前置片段（首包保证为关键帧）
                let packets = buffer.drain_from_keyframe();
                if packets.is_empty() {
                    tracing::debug!(
                        camera = %self.camera_id,
                        event_id = %trigger.event_id,
                        "前置缓冲无可用关键帧，本次事件录像跳过"
                    );
                    return Ok(());
                }

                let recording_id = uuid::Uuid::new_v4().to_string();
                let start_pts_ms = packets[0].pts_ms;
                let codec = packets[0].codec;

                let file_path = build_recording_path(
                    &self.data_dir,
                    &self.camera_id,
                    &recording_id,
                    start_pts_ms,
                )?;
                if let Some(parent) = file_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }

                let file = std::fs::File::create(&file_path)?;
                let mut writer = FMP4Writer::new(BufWriter::new(file), codec);

                // flush 前置片段
                let mut last_pts_ms = start_pts_ms;
                for pkt in &packets {
                    writer.push(pkt)?;
                    last_pts_ms = pkt.pts_ms;
                }

                let offset_ms = (trigger.event_time_ms - start_pts_ms).max(0);
                tracing::info!(
                    camera = %self.camera_id,
                    recording_id = %recording_id,
                    path = %file_path.display(),
                    pre_packets = packets.len(),
                    pre_span_ms = last_pts_ms - start_pts_ms,
                    "事件触发：开始录像（含前置片段）"
                );

                *active = Some(ActiveRecording {
                    recording_id,
                    file_path,
                    writer,
                    codec,
                    start_pts_ms,
                    last_pts_ms,
                    deadline_mono_ms: new_deadline,
                    events: vec![LinkedEvent {
                        event_id: trigger.event_id,
                        event_type: trigger.event_type,
                        event_time_ms: trigger.event_time_ms,
                        offset_ms,
                    }],
                    truncated: false,
                });
            }
            Some(rec) => {
                // 延长合并：重置倒计时 + 追加事件关联
                rec.deadline_mono_ms = new_deadline;
                let offset_ms = (trigger.event_time_ms - rec.start_pts_ms).max(0);
                rec.events.push(LinkedEvent {
                    event_id: trigger.event_id,
                    event_type: trigger.event_type,
                    event_time_ms: trigger.event_time_ms,
                    offset_ms,
                });
                tracing::info!(
                    camera = %self.camera_id,
                    recording_id = %rec.recording_id,
                    linked_events = rec.events.len(),
                    "事件合并：延长当前录像"
                );
            }
        }
        Ok(())
    }

    /// 检查 post-capture 截止与文件时长硬上限
    fn maybe_close_on_deadline(
        &self,
        active: &mut Option<ActiveRecording>,
        buffer: &mut PreCaptureRingBuffer,
    ) {
        let Some(rec) = active.as_ref() else {
            return;
        };
        let over_deadline = monotonic_ms() >= rec.deadline_mono_ms;
        let max_file_ms = i64::from(self.config.max_file_seconds) * 1000;
        let over_max_duration = rec.last_pts_ms.saturating_sub(rec.start_pts_ms) >= max_file_ms;

        if over_deadline || over_max_duration {
            let reason = if over_max_duration {
                "达到文件时长硬上限"
            } else {
                "post-capture 到期"
            };
            tracing::info!(
                camera = %self.camera_id,
                recording_id = %rec.recording_id,
                reason,
                "闭合录像"
            );
            if let Some(rec) = active.take() {
                let _ = buffer; // 文件闭合后前置缓冲重新积累
                self.finish(rec, false);
            }
        }
    }

    /// 闭合录像：flush fMP4、回调落库
    fn finish(&self, rec: ActiveRecording, truncated: bool) {
        let ActiveRecording {
            recording_id,
            file_path,
            writer,
            codec,
            start_pts_ms,
            last_pts_ms,
            events,
            truncated: already_truncated,
            ..
        } = rec;

        let truncated = truncated || already_truncated;
        let status = if truncated { "truncated" } else { "completed" };

        match writer.finish() {
            Ok(info) => {
                self.recordings_written.fetch_add(1, Ordering::Relaxed);
                if truncated {
                    self.recordings_truncated.fetch_add(1, Ordering::Relaxed);
                }
                tracing::info!(
                    camera = %self.camera_id,
                    recording_id = %recording_id,
                    bytes = info.bytes_written,
                    fragments = info.fragments_written,
                    linked_events = events.len(),
                    status,
                    "录像文件闭合完成"
                );
                (self.on_finished)(FinishedRecording {
                    camera_id: self.camera_id.clone(),
                    recording_id,
                    file_path,
                    start_time_ms: start_pts_ms,
                    end_time_ms: last_pts_ms,
                    file_size: info.bytes_written,
                    status,
                    codec,
                    events,
                });
            }
            Err(error) => {
                tracing::error!(
                    camera = %self.camera_id,
                    recording_id = %recording_id,
                    %error,
                    "录像文件闭合失败，删除残留文件"
                );
                let _ = std::fs::remove_file(&file_path);
            }
        }
    }
}

/// 判断包是否属于可录像的视频流
#[inline]
fn is_recordable_video(pkt: &EncodedPacket) -> bool {
    pkt.codec.is_video() && pkt.stream_tag != types::StreamTag::Audio
}

/// 构建录像文件路径：`{data_dir}/recordings/{camera_id}/{YYYY-MM-DD}/{HHMMSS}_{short_id}.mp4`
fn build_recording_path(
    data_dir: &Path,
    camera_id: &str,
    recording_id: &str,
    start_pts_ms: i64,
) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
    let dt = chrono::DateTime::from_timestamp_millis(start_pts_ms)
        .ok_or("invalid pts for recording path")?;
    let date = dt.format("%Y-%m-%d").to_string();
    let time = dt.format("%H%M%S").to_string();
    let short_id = &recording_id[..recording_id.len().min(8)];
    Ok(data_dir
        .join("recordings")
        .join(sanitize_component(camera_id))
        .join(date)
        .join(format!("{time}_{short_id}.mp4")))
}

/// 清理路径组件中的不安全字符（防路径穿越）
fn sanitize_component(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// 单调时钟毫秒（进程启动基准）
#[inline]
fn monotonic_ms() -> u64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    START
        .get_or_init(Instant::now)
        .elapsed()
        .as_millis() as u64
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_build_recording_path_layout() {
        // 2025-07-15 08:30:52 UTC
        let pts = 1752568252000;
        let path = build_recording_path(
            Path::new("/data"),
            "cam-01",
            "abcdef12-3456-7890-abcd-ef1234567890",
            pts,
        )
        .unwrap();

        let s = path.to_string_lossy();
        assert!(s.starts_with("/data/recordings/cam-01/2025-07-15/"), "实际: {s}");
        assert!(s.ends_with("_abcdef12.mp4"), "实际: {s}");
    }

    #[test]
    fn test_sanitize_component_blocks_traversal() {
        // 路径分隔符与点号均被替换，杜绝路径穿越
        assert_eq!(sanitize_component("../../etc/passwd"), "______etc_passwd");
        assert_eq!(sanitize_component("cam/../01"), "cam____01");
        assert_eq!(sanitize_component("cam-01_ok"), "cam-01_ok");
    }

    #[test]
    fn test_monotonic_ms_is_monotonic() {
        let a = monotonic_ms();
        std::thread::sleep(Duration::from_millis(5));
        let b = monotonic_ms();
        assert!(b >= a);
    }
}
