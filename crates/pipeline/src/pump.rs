//! 子码流分析驱动泵 (Sub-Stream Analysis Pump)
//!
//! 核心职责：
//! 1. 订阅指定摄像机子码流广播包，维持 H.264/H.265 解码器完整参考帧链；
//! 2. 抽帧节流器 (Analysis FPS Governor) 按需抽帧，降低 NPU 负载；
//! 3. 每帧解码结果实时同步更新至管线保底快照源 (sub_stream_fallback)；
//! 4. 抽帧通过单槽 Drop-Oldest 缓冲区送入专用常驻推理线程池 (InferenceWorkerHandle)，防范超载；
//! 5. 串行将推理结果输送至 `PipelineManager::process_detections`，规则触发告警时自动闭环执行靶向高清快照落地。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use media::decoder::VideoDecoder;
use media::stream_hub::CameraStreamSession;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use types::{FrameRef, MotionGateConfig};

use infer::InferenceWorkerHandle;

use crate::events::{
    EvidenceStatus, PipelineAlarmEvent, PipelineAnalysisEvent, PipelineTrackEvent,
};
use crate::manager::PipelineManager;
use crate::motion_gate::MotionGate;

/// 子码流分析驱动泵配置
#[derive(Debug, Clone)]
pub struct SubStreamPumpConfig {
    /// 目标分析抽帧率 (0 表示不限帧率全量抽帧)
    pub target_fps: u32,
    /// 是否启用简易帧差运动门控 (静止场景跳过推理)
    pub motion_gate_enabled: bool,
}

impl Default for SubStreamPumpConfig {
    fn default() -> Self {
        Self {
            target_fps: 10,
            motion_gate_enabled: false,
        }
    }
}

/// pump 任务停止等待上限；超时后后台任务继续持有硬件句柄，避免控制面被拖死。
pub const DEFAULT_PUMP_SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(3000);

struct BlockingDecoder {
    inner: Option<Box<dyn VideoDecoder + Send>>,
}

impl BlockingDecoder {
    fn new(inner: Box<dyn VideoDecoder + Send>) -> Self {
        Self { inner: Some(inner) }
    }

    async fn decode_packet(
        &mut self,
        packet: &[u8],
        pts: i64,
    ) -> Result<Option<FrameRef>, media::error::MediaError> {
        self.inner
            .as_mut()
            .expect("decoder owner must remain present")
            .decode_packet(packet, pts)
            .await
    }

    async fn flush(&mut self) -> Result<Vec<FrameRef>, media::error::MediaError> {
        self.inner
            .as_mut()
            .expect("decoder owner must remain present")
            .flush()
            .await
    }

    async fn dispose(&mut self) {
        if let Some(decoder) = self.inner.take() {
            let _ = tokio::task::spawn_blocking(move || drop(decoder)).await;
        }
    }
}

impl Drop for BlockingDecoder {
    fn drop(&mut self) {
        let Some(decoder) = self.inner.take() else {
            return;
        };
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn_blocking(move || drop(decoder));
        } else {
            drop(decoder);
        }
    }
}

/// 驱动泵运行统计指标
#[derive(Debug, Default)]
pub struct PumpMetrics {
    /// 累计接收的编码视频包数
    pub packets_received: AtomicU64,
    /// 累计成功解码的视频帧数
    pub frames_decoded: AtomicU64,
    /// 累计命中抽帧采样的帧数
    pub frames_sampled: AtomicU64,
    /// 累计完成推理的帧数
    pub frames_inferred: AtomicU64,
    /// 累计因运动门控跳过的推理帧数
    pub frames_skipped_motion: AtomicU64,
    /// 累计因队列积压跳过的网络包数 (Lagged)
    pub frames_dropped_lagged: AtomicU64,
    /// 累计解码失败次数
    pub decode_errors: AtomicU64,
    /// 累计推理失败或被丢弃次数
    pub inference_errors: AtomicU64,
    /// 累计触发的业务规则告警数
    pub alarms_triggered: AtomicU64,
    /// 累计成功落地的证据快照数
    pub snapshots_saved: AtomicU64,
}

/// 抽帧节流控制器 (按目标 FPS 采样分析帧)
#[derive(Debug, Clone)]
pub struct AnalysisFpsGovernor {
    target_fps: u32,
    interval_ms: i64,
    last_sampled_pts: Option<i64>,
}

impl AnalysisFpsGovernor {
    pub fn new(target_fps: u32) -> Self {
        let interval_ms = if target_fps > 0 {
            1000 / (target_fps as i64)
        } else {
            0
        };
        Self {
            target_fps,
            interval_ms,
            last_sampled_pts: None,
        }
    }

    /// 判定当前帧是否命中采样周期
    pub fn should_sample(&mut self, pts_ms: i64) -> bool {
        if self.interval_ms <= 0 {
            return true;
        }

        match self.last_sampled_pts {
            None => {
                self.last_sampled_pts = Some(pts_ms);
                true
            }
            Some(last_pts) => {
                // 应对时间戳跳跃、流重连或回绕场景
                if pts_ms < last_pts || (pts_ms - last_pts) >= self.interval_ms {
                    self.last_sampled_pts = Some(pts_ms);
                    true
                } else {
                    false
                }
            }
        }
    }

    #[inline]
    pub fn target_fps(&self) -> u32 {
        self.target_fps
    }
}

/// 内部单槽采样传递队列 (Drop-Oldest 单槽缓冲)
struct SamplingSlot {
    frame: std::sync::Mutex<Option<FrameRef>>,
    notify: tokio::sync::Notify,
}

/// 子码流分析驱动泵
pub struct SubStreamAnalysisPump {
    camera_id: String,
    cancel_token: CancellationToken,
    decode_handle: Option<tokio::task::JoinHandle<()>>,
    infer_handle: Option<tokio::task::JoinHandle<()>>,
    managed_worker: Option<infer::InferenceWorker>,
    metrics: Arc<PumpMetrics>,
    worker_holder: Arc<tokio::sync::RwLock<InferenceWorkerHandle>>,
}

impl std::fmt::Debug for SubStreamAnalysisPump {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubStreamAnalysisPump")
            .field("camera_id", &self.camera_id)
            .field("is_running", &self.is_running())
            .field(
                "packets_received",
                &self.metrics.packets_received.load(Ordering::Relaxed),
            )
            .field(
                "frames_decoded",
                &self.metrics.frames_decoded.load(Ordering::Relaxed),
            )
            .field(
                "frames_inferred",
                &self.metrics.frames_inferred.load(Ordering::Relaxed),
            )
            .field(
                "alarms_triggered",
                &self.metrics.alarms_triggered.load(Ordering::Relaxed),
            )
            .field(
                "snapshots_saved",
                &self.metrics.snapshots_saved.load(Ordering::Relaxed),
            )
            .finish()
    }
}

impl SubStreamAnalysisPump {
    /// 启动子码流分析驱动泵 (使用外部推理 Handle)
    pub fn start(
        camera_id: impl Into<String>,
        session: Arc<CameraStreamSession>,
        decoder: Box<dyn VideoDecoder + Send>,
        worker: InferenceWorkerHandle,
        pipeline_mgr: Arc<PipelineManager>,
        config: SubStreamPumpConfig,
    ) -> Self {
        Self::start_internal(
            camera_id,
            session,
            decoder,
            worker,
            None,
            pipeline_mgr,
            config,
        )
    }

    /// 启动子码流分析驱动泵并由驱动泵托管 InferenceWorker 运行周期
    pub fn start_with_worker(
        camera_id: impl Into<String>,
        session: Arc<CameraStreamSession>,
        decoder: Box<dyn VideoDecoder + Send>,
        worker: infer::InferenceWorker,
        pipeline_mgr: Arc<PipelineManager>,
        config: SubStreamPumpConfig,
    ) -> Self {
        let handle = worker.handle();
        Self::start_internal(
            camera_id,
            session,
            decoder,
            handle,
            Some(worker),
            pipeline_mgr,
            config,
        )
    }

    fn start_internal(
        camera_id: impl Into<String>,
        session: Arc<CameraStreamSession>,
        decoder: Box<dyn VideoDecoder + Send>,
        worker: InferenceWorkerHandle,
        managed_worker: Option<infer::InferenceWorker>,
        pipeline_mgr: Arc<PipelineManager>,
        config: SubStreamPumpConfig,
    ) -> Self {
        let mut decoder = BlockingDecoder::new(decoder);
        let camera_id = camera_id.into();
        let cancel_token = CancellationToken::new();
        let decode_cancel = cancel_token.clone();
        let infer_cancel = cancel_token.clone();
        let cam_id = camera_id.clone();
        let cam_id_for_infer = camera_id.clone();
        let target_fps = config.target_fps;
        let motion_gate_enabled = config.motion_gate_enabled;
        let metrics = Arc::new(PumpMetrics::default());
        let metrics_clone = metrics.clone();
        let infer_metrics = metrics.clone();
        let pipeline_mgr_for_infer = pipeline_mgr.clone();

        // 获取流会话的 RAII AI 保活租约，异常与正常关停均能保证引用回收
        let ai_lease = session.acquire_ai_task_lease();

        let mut packet_rx = session.broadcast_tx.subscribe();

        let sampling_slot = Arc::new(SamplingSlot {
            frame: std::sync::Mutex::new(None),
            notify: tokio::sync::Notify::new(),
        });
        let infer_slot = sampling_slot.clone();
        let worker_holder = Arc::new(tokio::sync::RwLock::new(worker));
        let infer_worker_holder = worker_holder.clone();

        // 协程 1: 专用解码主循环（维持参考帧链完整，快速轮转，决不被推理阻塞）
        let decode_handle = tokio::spawn(async move {
            let _ai_lease = ai_lease;
            tracing::info!(
                camera_id = %cam_id,
                target_fps = target_fps,
                "子码流分析驱动泵解码循环已启动"
            );

            let mut governor = AnalysisFpsGovernor::new(target_fps);
            let mut motion_gate =
                motion_gate_enabled.then(|| MotionGate::new(MotionGateConfig::default()));

            loop {
                tokio::select! {
                    biased;

                    _ = decode_cancel.cancelled() => {
                        tracing::info!(camera_id = %cam_id, "子码流解码驱动循环收到关停信号");
                        break;
                    }

                    recv_res = packet_rx.recv() => {
                        let pkt = match recv_res {
                            Ok(p) => {
                                metrics_clone.packets_received.fetch_add(1, Ordering::Relaxed);
                                p
                            }
                            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                                metrics_clone.frames_dropped_lagged.fetch_add(skipped, Ordering::Relaxed);
                                tracing::warn!(
                                    camera_id = %cam_id,
                                    skipped,
                                    "子码流分析驱动泵数据包积压掉队 (Lagged)，继续处理后续数据包"
                                );
                                continue;
                            }
                            Err(broadcast::error::RecvError::Closed) => {
                                tracing::info!(camera_id = %cam_id, "子码流数据广播通道已关闭，解码循环退出");
                                break;
                            }
                        };

                        // 喂入专属解码器提取零拷贝 FrameRef
                        match decoder.decode_packet(&pkt.payload, pkt.pts_ms).await {
                            Ok(Some(frame)) => {
                                metrics_clone.frames_decoded.fetch_add(1, Ordering::Relaxed);

                                // 1. 实时更新管线保底快照候选帧 (sub_stream_fallback)
                                pipeline_mgr.update_sub_stream_frame(&cam_id, frame.clone()).await;

                                // 2. 运动门控过滤：静止帧跳过推理，节省算力
                                if let Some(gate) = motion_gate.as_mut() {
                                    if gate.should_skip_frame(&frame) {
                                        metrics_clone
                                            .frames_skipped_motion
                                            .fetch_add(1, Ordering::Relaxed);
                                        continue;
                                    }
                                }

                                // 3. 检查当前帧是否命中抽帧节流采样
                                if governor.should_sample(frame.timestamp) {
                                    metrics_clone.frames_sampled.fetch_add(1, Ordering::Relaxed);

                                    // 单槽 Drop-Oldest 投递，若推理任务尚未取走上一帧，替换为最新采样帧
                                    if let Ok(mut guard) = sampling_slot.frame.lock() {
                                        if guard.replace(frame).is_some() {
                                            metrics_clone.inference_errors.fetch_add(1, Ordering::Relaxed);
                                        }
                                    }
                                    sampling_slot.notify.notify_one();
                                }
                            }
                            Ok(None) => {
                                // 解码器内部缓冲中（P/B 帧累积），继续喂入下一包
                            }
                            Err(err) => {
                                metrics_clone.decode_errors.fetch_add(1, Ordering::Relaxed);
                                tracing::warn!(
                                    camera_id = %cam_id,
                                    error = %err,
                                    "子码流硬件解码单包异常，跳过当前包"
                                );
                            }
                        }
                    }
                }
            }

            // 驱动泵退出前在有界时间内刷新解码器，并把句柄析构放到 blocking 线程。
            match tokio::time::timeout(
                media::decoder::DEFAULT_THREAD_SHUTDOWN_TIMEOUT,
                decoder.flush(),
            )
            .await
            {
                Ok(Ok(_)) => {}
                Ok(Err(err)) => {
                    tracing::warn!(camera_id = %cam_id, error = %err, "子码流解码器 flush 失败")
                }
                Err(_) => tracing::error!(
                    camera_id = %cam_id,
                    timeout_ms = media::decoder::DEFAULT_THREAD_SHUTDOWN_TIMEOUT.as_millis() as u64,
                    "子码流解码器 flush 超时，隔离硬件句柄"
                ),
            }
            decoder.dispose().await;

            // AI 保活租约随着 _ai_lease 变量在协程退出时自动释放 (Drop)，无泄露风险。
            tracing::info!(camera_id = %cam_id, "子码流分析驱动泵解码循环已完全停止并清理资源");
        });

        // 协程 2: 严格单调保序的常驻推理与告警抓拍闭环循环
        let infer_handle = tokio::spawn(async move {
            tracing::info!(camera_id = %cam_id_for_infer, "子码流推理后处理循环已启动");

            loop {
                tokio::select! {
                    biased;

                    _ = infer_cancel.cancelled() => {
                        tracing::info!(camera_id = %cam_id_for_infer, "子码流推理后处理循环收到关停信号");
                        break;
                    }

                    _ = infer_slot.notify.notified() => {
                        let maybe_frame = infer_slot.frame.lock().ok().and_then(|mut g| g.take());
                        if let Some(frame) = maybe_frame {
                            let timestamp = frame.timestamp;
                            let current_worker = infer_worker_holder.read().await.clone();
                            match current_worker.submit(frame).await {
                                Ok(detections) => {
                                    infer_metrics.frames_inferred.fetch_add(1, Ordering::Relaxed);

                                    // 驱动管线执行航迹跟踪与几何规则判定 (严格保序执行)
                                    let (tracked, alarms) = pipeline_mgr_for_infer
                                        .process_detections(&cam_id_for_infer, detections, timestamp)
                                        .await;

                                    // 广播航迹追踪事件；空列表也是有效的“清空旧框”更新。
                                    pipeline_mgr_for_infer.publish_analysis_event(
                                        PipelineAnalysisEvent::Tracks(PipelineTrackEvent {
                                            camera_id: cam_id_for_infer.clone(),
                                            timestamp,
                                            tracks: tracked,
                                        }),
                                    );

                                    // 告警触发时，自动联动 RingBuffer / 保底帧执行靶向快照落地
                                    if !alarms.is_empty() {
                                        infer_metrics.alarms_triggered.fetch_add(alarms.len() as u64, Ordering::Relaxed);
                                        for alarm in alarms {
                                            let bbox = Some(alarm.tracked_object.bbox);
                                            let event_id = uuid::Uuid::new_v4().to_string();
                                            match pipeline_mgr_for_infer
                                                .trigger_snapshot(&cam_id_for_infer, timestamp, bbox)
                                                .await
                                            {
                                                Ok(snapshot_res) => {
                                                    infer_metrics.snapshots_saved.fetch_add(1, Ordering::Relaxed);
                                                    tracing::info!(
                                                        camera_id = %cam_id_for_infer,
                                                        event_id = %event_id,
                                                        target_pts = timestamp,
                                                        rule_index = alarm.rule_index,
                                                        path = %snapshot_res.image_rel_path,
                                                        is_fallback = snapshot_res.is_fallback_sub_stream,
                                                        "规则引擎告警触发高清快照落地成功"
                                                    );
                                                    pipeline_mgr_for_infer.publish_analysis_event(
                                                        PipelineAnalysisEvent::Alarm(Box::new(
                                                            PipelineAlarmEvent {
                                                                event_id,
                                                                camera_id: cam_id_for_infer.clone(),
                                                                alarm,
                                                                snapshot: Some(snapshot_res),
                                                                evidence_status: EvidenceStatus::Ready,
                                                                evidence_error: None,
                                                                timestamp,
                                                            },
                                                        )),
                                                    );
                                                }
                                                Err(err) => {
                                                    tracing::error!(
                                                        camera_id = %cam_id_for_infer,
                                                        event_id = %event_id,
                                                        target_pts = timestamp,
                                                        error = %err,
                                                        "规则引擎告警证据捕获失败，保留告警事实"
                                                    );
                                                    pipeline_mgr_for_infer.publish_analysis_event(
                                                        PipelineAnalysisEvent::Alarm(Box::new(
                                                            PipelineAlarmEvent {
                                                                event_id,
                                                                camera_id: cam_id_for_infer.clone(),
                                                                alarm,
                                                                snapshot: None,
                                                                evidence_status: EvidenceStatus::Failed,
                                                                evidence_error: Some(err.to_string()),
                                                                timestamp,
                                                            },
                                                        )),
                                                    );
                                                }
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    infer_metrics.inference_errors.fetch_add(1, Ordering::Relaxed);
                                    tracing::debug!(
                                        camera_id = %cam_id_for_infer,
                                        timestamp,
                                        error = %e,
                                        "推理帧被丢弃或检测失败 (Drop-Oldest 背压生效)"
                                    );
                                }
                            }
                        }
                    }
                }
            }

            tracing::info!(camera_id = %cam_id_for_infer, "子码流推理后处理循环已平稳退出");
        });

        Self {
            camera_id,
            cancel_token,
            decode_handle: Some(decode_handle),
            infer_handle: Some(infer_handle),
            managed_worker,
            metrics,
            worker_holder,
        }
    }

    /// 在两帧间隙原子替换推理 Worker 句柄（单进程优雅热重载，不断流、不重启解码器）
    pub async fn replace_worker(&self, new_worker: InferenceWorkerHandle) {
        let mut guard = self.worker_holder.write().await;
        *guard = new_worker;
    }

    /// 停止驱动泵并等待任务终止与工作线程资源回收。
    /// 硬件线程回收在 blocking pool 中执行，async 控制面不会同步 join。
    pub async fn stop(&mut self) {
        self.cancel_token.cancel();
        if let Some(handle) = self.decode_handle.take() {
            match tokio::time::timeout(DEFAULT_PUMP_SHUTDOWN_TIMEOUT, handle).await {
                Ok(Ok(())) => {}
                Ok(Err(err)) => {
                    tracing::warn!(camera_id = %self.camera_id, error = %err, "子码流解码任务异常退出")
                }
                Err(_) => tracing::error!(
                    camera_id = %self.camera_id,
                    timeout_ms = DEFAULT_PUMP_SHUTDOWN_TIMEOUT.as_millis() as u64,
                    "子码流解码任务停止超时，保留后台句柄隔离"
                ),
            }
        }
        if let Some(handle) = self.infer_handle.take() {
            match tokio::time::timeout(DEFAULT_PUMP_SHUTDOWN_TIMEOUT, handle).await {
                Ok(Ok(())) => {}
                Ok(Err(err)) => {
                    tracing::warn!(camera_id = %self.camera_id, error = %err, "子码流推理任务异常退出")
                }
                Err(_) => tracing::error!(
                    camera_id = %self.camera_id,
                    timeout_ms = DEFAULT_PUMP_SHUTDOWN_TIMEOUT.as_millis() as u64,
                    "子码流推理任务停止超时，保留后台句柄隔离"
                ),
            }
        }
        if let Some(worker) = self.managed_worker.take() {
            let shutdown = tokio::task::spawn_blocking(move || {
                let mut worker = worker;
                worker.shutdown()
            });
            match tokio::time::timeout(DEFAULT_PUMP_SHUTDOWN_TIMEOUT, shutdown).await {
                Ok(Ok(true)) => {}
                Ok(Ok(false)) => {
                    tracing::error!(camera_id = %self.camera_id, "推理 Worker 停止超时，已隔离线程")
                }
                Ok(Err(err)) => {
                    tracing::error!(camera_id = %self.camera_id, error = %err, "推理 Worker 回收任务异常")
                }
                Err(_) => tracing::error!(
                    camera_id = %self.camera_id,
                    timeout_ms = DEFAULT_PUMP_SHUTDOWN_TIMEOUT.as_millis() as u64,
                    "推理 Worker 回收任务等待超时"
                ),
            }
        }
    }

    /// 查询驱动泵是否正在运行
    #[inline]
    pub fn is_running(&self) -> bool {
        if self.cancel_token.is_cancelled() {
            return false;
        }
        let decode_active = self
            .decode_handle
            .as_ref()
            .map(|h| !h.is_finished())
            .unwrap_or(false);
        let infer_active = self
            .infer_handle
            .as_ref()
            .map(|h| !h.is_finished())
            .unwrap_or(false);
        decode_active || infer_active
    }

    /// 获取驱动泵运行监控指标
    #[inline]
    pub fn metrics(&self) -> &Arc<PumpMetrics> {
        &self.metrics
    }
}

impl Drop for SubStreamAnalysisPump {
    fn drop(&mut self) {
        if !self.cancel_token.is_cancelled() {
            self.cancel_token.cancel();
        }
        if let Some(worker) = self.managed_worker.take() {
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn_blocking(move || {
                    let mut worker = worker;
                    worker.shutdown();
                });
            } else {
                let mut worker = worker;
                worker.shutdown();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fps_governor_unlimited() {
        let mut gov = AnalysisFpsGovernor::new(0);
        assert!(gov.should_sample(1000));
        assert!(gov.should_sample(1040));
        assert!(gov.should_sample(1080));
    }

    #[test]
    fn test_fps_governor_target_10fps() {
        // 10fps -> 间隔 100ms
        let mut gov = AnalysisFpsGovernor::new(10);
        assert!(gov.should_sample(1000), "首帧必须采样");
        assert!(!gov.should_sample(1040), "40ms 差距未达到 100ms 间隔");
        assert!(!gov.should_sample(1080), "80ms 差距未达到 100ms 间隔");
        assert!(gov.should_sample(1100), "100ms 差距必须采样");
        assert!(!gov.should_sample(1140));
        assert!(gov.should_sample(1200));
    }

    #[test]
    fn test_fps_governor_timestamp_reset_resilience() {
        let mut gov = AnalysisFpsGovernor::new(10);
        assert!(gov.should_sample(5000));
        assert!(!gov.should_sample(5050));
        // 时间戳回跳（流重连）
        assert!(gov.should_sample(1000), "时间戳回跳必须自适应重置采样基准");
    }
}
