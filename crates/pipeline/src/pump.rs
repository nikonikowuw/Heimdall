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

use infer::{InferenceWorker, InferenceWorkerHandle};

use crate::events::{
    EvidenceStatus, PipelineAlarmEvent, PipelineAnalysisEvent, PipelineTrackEvent,
};
use crate::manager::PipelineManager;
use crate::motion_gate::MotionGate;

/// 向后兼容单 Worker 模式使用的伪算法 ID
pub(crate) const LEGACY_SINGLE_WORKER_ID: &str = "__legacy_single__";

/// 单个算法实例在驱动泵中的运行指标
#[derive(Debug, Default)]
pub struct InstanceMetrics {
    /// 命中抽帧采样的帧数
    pub frames_sampled: AtomicU64,
    /// 完成推理的帧数
    pub frames_inferred: AtomicU64,
    /// 因背压队列已满被丢弃的采样帧数 (Drop-Oldest)
    pub frames_dropped: AtomicU64,
    /// 推理执行失败次数
    pub inference_errors: AtomicU64,
    /// 触发的业务规则告警数
    pub alarms_triggered: AtomicU64,
    /// 成功落地的证据快照数
    pub snapshots_saved: AtomicU64,
}

/// 单个算法实例的多工配置
#[derive(Debug, Clone)]
pub struct WorkerInstanceConfig {
    /// 算法 ID
    pub algorithm_id: String,
    /// 目标分析 FPS (0 表示不限帧率)
    pub target_fps: u32,
    /// 创建算法实例时使用的 JSON 配置
    pub config_json: Option<String>,
}

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
    /// 累计因背压队列已满被丢弃的采样帧数 (Drop-Oldest)
    pub frames_dropped: AtomicU64,
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
pub(crate) struct SamplingSlot {
    pub(crate) frame: std::sync::Mutex<Option<FrameRef>>,
    pub(crate) notify: tokio::sync::Notify,
}

impl SamplingSlot {
    pub(crate) fn new() -> Self {
        Self {
            frame: std::sync::Mutex::new(None),
            notify: tokio::sync::Notify::new(),
        }
    }
}

/// 解码循环本地的算法实例抽帧槽（独占运行，无需跨任务锁）
struct DecodeSlot {
    governor: AnalysisFpsGovernor,
    sampling_slot: Arc<SamplingSlot>,
    metrics: Arc<InstanceMetrics>,
}

/// 控制面持有的算法实例槽（用于 Worker 热重载、优雅退出与指标查询）
struct ControlSlot {
    algorithm_id: String,
    config_json: Option<String>,
    worker_holder: Arc<tokio::sync::RwLock<InferenceWorkerHandle>>,
    managed_worker: Option<InferenceWorker>,
    infer_handle: Option<tokio::task::JoinHandle<()>>,
    metrics: Arc<InstanceMetrics>,
}

/// 共享的控制槽容器
struct SharedControlSlots {
    inner: tokio::sync::Mutex<Vec<ControlSlot>>,
}

impl SharedControlSlots {
    fn new(slots: Vec<ControlSlot>) -> Self {
        Self {
            inner: tokio::sync::Mutex::new(slots),
        }
    }

    async fn stop_all_workers(&self) {
        let mut workers = {
            let mut guard = self.inner.lock().await;
            guard
                .iter_mut()
                .filter_map(|slot| slot.managed_worker.take())
                .collect::<Vec<_>>()
        };

        for mut worker in workers.drain(..) {
            let shutdown = tokio::task::spawn_blocking(move || worker.shutdown());
            let _ = tokio::time::timeout(DEFAULT_PUMP_SHUTDOWN_TIMEOUT, shutdown).await;
        }
    }

    async fn instance_metrics_snapshot(&self) -> Vec<(String, Arc<InstanceMetrics>)> {
        let guard = self.inner.lock().await;
        guard
            .iter()
            .map(|s| (s.algorithm_id.clone(), s.metrics.clone()))
            .collect()
    }
}

/// 子码流分析驱动泵
pub struct SubStreamAnalysisPump {
    camera_id: String,
    cancel_token: CancellationToken,
    decode_handle: Option<tokio::task::JoinHandle<()>>,
    metrics: Arc<PumpMetrics>,
    control_slots: Arc<SharedControlSlots>,
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
    /// 启动子码流分析驱动泵 (使用外部推理 Handle，向后兼容单 worker 模式)
    pub fn start(
        camera_id: impl Into<String>,
        session: Arc<CameraStreamSession>,
        decoder: Box<dyn VideoDecoder + Send>,
        worker: InferenceWorkerHandle,
        pipeline_mgr: Arc<PipelineManager>,
        config: SubStreamPumpConfig,
    ) -> Self {
        Self::start_multi_worker(
            camera_id,
            session,
            decoder,
            vec![WorkerInstanceConfig {
                algorithm_id: LEGACY_SINGLE_WORKER_ID.to_string(),
                target_fps: config.target_fps,
                config_json: None,
            }],
            vec![(LEGACY_SINGLE_WORKER_ID.to_string(), worker, None)],
            pipeline_mgr,
            config.motion_gate_enabled,
        )
    }

    /// 启动子码流分析驱动泵并由驱动泵托管 InferenceWorker 运行周期 (向后兼容)
    pub fn start_with_worker(
        camera_id: impl Into<String>,
        session: Arc<CameraStreamSession>,
        decoder: Box<dyn VideoDecoder + Send>,
        worker: infer::InferenceWorker,
        pipeline_mgr: Arc<PipelineManager>,
        config: SubStreamPumpConfig,
    ) -> Self {
        let handle = worker.handle();
        Self::start_multi_worker(
            camera_id,
            session,
            decoder,
            vec![WorkerInstanceConfig {
                algorithm_id: LEGACY_SINGLE_WORKER_ID.to_string(),
                target_fps: config.target_fps,
                config_json: None,
            }],
            vec![(LEGACY_SINGLE_WORKER_ID.to_string(), handle, Some(worker))],
            pipeline_mgr,
            config.motion_gate_enabled,
        )
    }

    /// 启动多算法实例子码流驱动泵
    pub fn start_multi_worker(
        camera_id: impl Into<String>,
        session: Arc<CameraStreamSession>,
        decoder: Box<dyn VideoDecoder + Send>,
        instance_configs: Vec<WorkerInstanceConfig>,
        workers: Vec<(String, InferenceWorkerHandle, Option<InferenceWorker>)>,
        pipeline_mgr: Arc<PipelineManager>,
        motion_gate_enabled: bool,
    ) -> Self {
        let mut decoder = BlockingDecoder::new(decoder);
        let camera_id = camera_id.into();
        let cancel_token = CancellationToken::new();
        let decode_cancel = cancel_token.clone();
        let cam_id = camera_id.clone();
        let metrics = Arc::new(PumpMetrics::default());
        let metrics_clone = metrics.clone();
        let pipeline_mgr_decode = pipeline_mgr.clone();

        // 获取流会话的 RAII AI 保活租约，异常与正常关停均能保证引用回收
        let ai_lease = session.acquire_ai_task_lease();

        let mut packet_rx = session.broadcast_tx.subscribe();

        // 为每个算法实例构建解码抽帧槽与控制面 Worker 槽。
        // 两类槽分离后，逐帧解码只访问本地 Vec，不与热重载/停机控制锁竞争。
        let mut decode_slot_vec: Vec<DecodeSlot> = Vec::with_capacity(instance_configs.len());
        let mut control_slot_vec: Vec<ControlSlot> = Vec::with_capacity(instance_configs.len());
        for (cfg, (alg_id, handle, managed)) in instance_configs.into_iter().zip(workers) {
            let sampling_slot = Arc::new(SamplingSlot::new());
            let instance_metrics = Arc::new(InstanceMetrics::default());

            let infer_cancel = cancel_token.clone();
            let infer_slot = sampling_slot.clone();
            let worker_holder = Arc::new(tokio::sync::RwLock::new(handle));
            let infer_worker_holder = worker_holder.clone();
            let infer_metrics = instance_metrics.clone();
            let pump_metrics = metrics.clone();
            let cam_id_infer = camera_id.clone();
            let pipeline_mgr_infer = pipeline_mgr.clone();
            let algorithm_id_infer = alg_id.clone();

            let infer_handle = tokio::spawn(async move {
                tracing::info!(
                    camera_id = %cam_id_infer,
                    algorithm_id = %algorithm_id_infer,
                    "子码流多算法实例推理循环已启动"
                );

                loop {
                    tokio::select! {
                        biased;

                        _ = infer_cancel.cancelled() => {
                            tracing::info!(
                                camera_id = %cam_id_infer,
                                algorithm_id = %algorithm_id_infer,
                                "子码流多算法实例推理循环收到关停信号"
                            );
                            break;
                        }

                        _ = infer_slot.notify.notified() => {
                            let maybe_frame = infer_slot.frame.lock().ok().and_then(|mut g| g.take());
                            if let Some(frame) = maybe_frame {
                                let timestamp = frame.timestamp;
                                let current_worker = infer_worker_holder.read().await.clone();
                                match current_worker.submit(frame).await {
                                    Ok(detections) => {
                                        infer_metrics
                                            .frames_inferred
                                            .fetch_add(1, Ordering::Relaxed);
                                        pump_metrics
                                            .frames_inferred
                                            .fetch_add(1, Ordering::Relaxed);

                                        // 驱动管线执行独立算法实例的航迹跟踪与几何规则判定 (保序执行)
                                        let (tracked, alarms) = pipeline_mgr_infer
                                            .process_detections_for_algo(
                                                &cam_id_infer,
                                                &algorithm_id_infer,
                                                detections,
                                                timestamp,
                                            )
                                            .await;

                                        // 广播航迹追踪事件
                                        pipeline_mgr_infer.publish_analysis_event(
                                            PipelineAnalysisEvent::Tracks(PipelineTrackEvent {
                                                camera_id: cam_id_infer.clone(),
                                                algorithm_id: algorithm_id_infer.clone(),
                                                timestamp,
                                                tracks: tracked,
                                            }),
                                        );

                                        // 告警触发时，自动联动快照抓拍落地并广播告警事件
                                        if !alarms.is_empty() {
                                            infer_metrics
                                                .alarms_triggered
                                                .fetch_add(alarms.len() as u64, Ordering::Relaxed);
                                            pump_metrics
                                                .alarms_triggered
                                                .fetch_add(alarms.len() as u64, Ordering::Relaxed);
                                            for alarm in alarms {
                                                let bbox = Some(alarm.tracked_object.bbox);
                                                let event_id = uuid::Uuid::new_v4().to_string();
                                                let (snapshot, evidence_status, evidence_error) =
                                                    match pipeline_mgr_infer
                                                        .trigger_snapshot(
                                                            &cam_id_infer,
                                                            timestamp,
                                                            bbox,
                                                        )
                                                        .await
                                                    {
                                                        Ok(snapshot_res) => {
                                                            infer_metrics
                                                                .snapshots_saved
                                                                .fetch_add(1, Ordering::Relaxed);
                                                            pump_metrics
                                                                .snapshots_saved
                                                                .fetch_add(1, Ordering::Relaxed);
                                                            tracing::info!(
                                                                camera_id = %cam_id_infer,
                                                                algorithm_id = %algorithm_id_infer,
                                                                target_pts = timestamp,
                                                                rule_index = alarm.rule_index,
                                                                path = %snapshot_res.image_rel_path,
                                                                is_fallback = snapshot_res.is_fallback_sub_stream,
                                                                "规则引擎告警触发高清快照落地成功"
                                                            );
                                                            (
                                                                Some(snapshot_res),
                                                                EvidenceStatus::Ready,
                                                                None,
                                                            )
                                                        }
                                                        Err(err) => {
                                                            tracing::error!(
                                                                camera_id = %cam_id_infer,
                                                                algorithm_id = %algorithm_id_infer,
                                                                target_pts = timestamp,
                                                                error = %err,
                                                                "规则引擎告警触发快照捕获失败"
                                                            );
                                                            (
                                                                None,
                                                                EvidenceStatus::Failed,
                                                                Some(err.to_string()),
                                                            )
                                                        }
                                                    };

                                                pipeline_mgr_infer.publish_analysis_event(
                                                    PipelineAnalysisEvent::Alarm(Box::new(
                                                        PipelineAlarmEvent {
                                                            event_id,
                                                            camera_id: cam_id_infer.clone(),
                                                            alarm,
                                                            snapshot,
                                                            evidence_status,
                                                            evidence_error,
                                                            timestamp,
                                                        },
                                                    )),
                                                );
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        infer_metrics
                                            .inference_errors
                                            .fetch_add(1, Ordering::Relaxed);
                                        pump_metrics
                                            .inference_errors
                                            .fetch_add(1, Ordering::Relaxed);
                                        tracing::debug!(
                                            camera_id = %cam_id_infer,
                                            algorithm_id = %algorithm_id_infer,
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

                tracing::info!(
                    camera_id = %cam_id_infer,
                    algorithm_id = %algorithm_id_infer,
                    "子码流多算法实例推理循环已平稳退出"
                );
            });

            decode_slot_vec.push(DecodeSlot {
                governor: AnalysisFpsGovernor::new(cfg.target_fps),
                sampling_slot,
                metrics: instance_metrics.clone(),
            });
            control_slot_vec.push(ControlSlot {
                algorithm_id: alg_id,
                config_json: cfg.config_json,
                worker_holder,
                managed_worker: managed,
                infer_handle: Some(infer_handle),
                metrics: instance_metrics,
            });
        }

        let slot_count = decode_slot_vec.len();
        let control_slots = Arc::new(SharedControlSlots::new(control_slot_vec));
        let mut decode_slots = decode_slot_vec;

        // 协程: 专用解码主循环（维持参考帧链完整，快速轮转，决不被推理阻塞）
        let decode_handle = tokio::spawn(async move {
            let _ai_lease = ai_lease;
            tracing::info!(
                camera_id = %cam_id,
                slot_count,
                "子码流多算法驱动泵解码循环已启动"
            );

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

                        match decoder.decode_packet(&pkt.payload, pkt.pts_ms).await {
                            Ok(Some(frame)) => {
                                metrics_clone.frames_decoded.fetch_add(1, Ordering::Relaxed);

                                // 1. 实时更新管线保底快照候选帧
                                pipeline_mgr_decode.update_sub_stream_frame(&cam_id, frame.clone()).await;

                                // 2. 运动门控过滤：静止帧跳过所有槽位推理，节省算力
                                if let Some(gate) = motion_gate.as_mut() {
                                    if gate.should_skip_frame(&frame) {
                                        metrics_clone
                                            .frames_skipped_motion
                                            .fetch_add(1, Ordering::Relaxed);
                                        continue;
                                    }
                                }

                                // 3. 轮询每个实例的独立 FPS 节流器，命中采样的实例投递帧。
                                // decode_slots 属于当前解码任务，不需要跨任务锁。
                                for slot in &mut decode_slots {
                                    if slot.governor.should_sample(frame.timestamp) {
                                        slot.metrics.frames_sampled.fetch_add(1, Ordering::Relaxed);
                                        metrics_clone.frames_sampled.fetch_add(1, Ordering::Relaxed);

                                        // Drop-Oldest 单槽投递
                                        if let Ok(mut slot_guard) = slot.sampling_slot.frame.lock() {
                                            if slot_guard.replace(frame.clone()).is_some() {
                                                slot.metrics.frames_dropped.fetch_add(1, Ordering::Relaxed);
                                                metrics_clone.frames_dropped.fetch_add(1, Ordering::Relaxed);
                                            }
                                        }
                                        slot.sampling_slot.notify.notify_one();
                                    }
                                }
                            }
                            Ok(None) => {}
                            Err(e) => {
                                metrics_clone.decode_errors.fetch_add(1, Ordering::Relaxed);
                                tracing::warn!(camera_id = %cam_id, error = %e, "解码子码流数据包失败");
                            }
                        }
                    }
                }
            }

            // 退出前先在有界时间内刷新解码器，再把硬件句柄放到 blocking 线程析构。
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

            tracing::info!(camera_id = %cam_id, "子码流分析驱动泵解码循环已完全停止并清理资源");
        });

        Self {
            camera_id,
            cancel_token,
            decode_handle: Some(decode_handle),
            metrics,
            control_slots,
        }
    }

    /// 返回指定算法实例的创建配置；`Some(None)` 表示实例存在但使用默认配置。
    pub async fn worker_config_for_algorithm(&self, algorithm_id: &str) -> Option<Option<String>> {
        let guard = self.control_slots.inner.lock().await;
        guard
            .iter()
            .find(|slot| slot.algorithm_id == algorithm_id)
            .map(|slot| slot.config_json.clone())
    }

    /// 查询指定算法实例是否存在于当前驱动泵。
    pub async fn has_worker_for_algorithm(&self, algorithm_id: &str) -> bool {
        self.worker_config_for_algorithm(algorithm_id)
            .await
            .is_some()
    }

    /// 在两帧间隙替换并托管指定算法实例的 Worker。
    ///
    /// 新 Worker 的所有权转移到驱动泵，旧 Worker 在 blocking 线程中关闭，
    /// 避免推理线程析构或底层 FFI 释放阻塞 Tokio worker。
    pub async fn replace_worker_with_owner(
        &self,
        algorithm_id: &str,
        mut new_worker: InferenceWorker,
    ) -> bool {
        let replacement_handle = new_worker.handle();
        let (worker_holder, old_worker) = {
            let mut guard = self.control_slots.inner.lock().await;
            let Some(slot) = guard
                .iter_mut()
                .find(|slot| slot.algorithm_id == algorithm_id)
            else {
                drop(guard);
                let shutdown = tokio::task::spawn_blocking(move || new_worker.shutdown());
                let _ = tokio::time::timeout(DEFAULT_PUMP_SHUTDOWN_TIMEOUT, shutdown).await;
                tracing::warn!(
                    camera_id = %self.camera_id,
                    algorithm_id = %algorithm_id,
                    "未找到目标算法实例槽，Worker 热替换失败"
                );
                return false;
            };
            let old_worker = slot.managed_worker.replace(new_worker);
            (slot.worker_holder.clone(), old_worker)
        };

        {
            let mut holder = worker_holder.write().await;
            *holder = replacement_handle;
        }

        if let Some(mut old_worker) = old_worker {
            let shutdown = tokio::task::spawn_blocking(move || old_worker.shutdown());
            let _ = tokio::time::timeout(DEFAULT_PUMP_SHUTDOWN_TIMEOUT, shutdown).await;
        }
        true
    }

    /// 在两帧间隙原子替换指定算法实例的推理 Worker 句柄
    pub async fn replace_worker(&self, algorithm_id: &str, new_worker: InferenceWorkerHandle) {
        let worker_holder = {
            let guard = self.control_slots.inner.lock().await;
            guard.iter().find_map(|slot| {
                (slot.algorithm_id == algorithm_id
                    || (algorithm_id == LEGACY_SINGLE_WORKER_ID && guard.len() == 1))
                    .then(|| slot.worker_holder.clone())
            })
        };

        if let Some(worker_holder) = worker_holder {
            let mut guard = worker_holder.write().await;
            *guard = new_worker;
            return;
        }

        tracing::warn!(
            camera_id = %self.camera_id,
            algorithm_id = %algorithm_id,
            "未找到目标算法实例槽，Worker 热替换失败"
        );
    }

    /// 停止驱动泵并等待任务终止与工作线程资源回收
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

        // 等待所有实例推理循环平稳退出
        let infer_handles: Vec<tokio::task::JoinHandle<()>> = {
            let mut guard = self.control_slots.inner.lock().await;
            guard
                .iter_mut()
                .filter_map(|s| s.infer_handle.take())
                .collect()
        };

        for handle in infer_handles {
            let _ = tokio::time::timeout(DEFAULT_PUMP_SHUTDOWN_TIMEOUT, handle).await;
        }

        // 回收全部托管 Worker
        self.control_slots.stop_all_workers().await;
    }

    /// 查询驱动泵是否正在运行
    #[inline]
    pub fn is_running(&self) -> bool {
        if self.cancel_token.is_cancelled() {
            return false;
        }
        self.decode_handle
            .as_ref()
            .map(|h| !h.is_finished())
            .unwrap_or(false)
    }

    /// 获取驱动泵运行监控指标
    #[inline]
    pub fn metrics(&self) -> &Arc<PumpMetrics> {
        &self.metrics
    }

    /// 获取所有实例的运行监控指标快照
    pub async fn instance_metrics(&self) -> Vec<(String, Arc<InstanceMetrics>)> {
        self.control_slots.instance_metrics_snapshot().await
    }
}

impl Drop for SubStreamAnalysisPump {
    fn drop(&mut self) {
        if !self.cancel_token.is_cancelled() {
            self.cancel_token.cancel();
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
        assert!(gov.should_sample(1000), "时间戳回跳必须自适应重置采样基准");
    }
}
