//! 分析码流驱动泵 (Analysis Pump)
//!
//! 核心职责：
//! 1. 订阅已决议的分析码流广播包，维持 H.264/H.265 解码器完整参考帧链；
//! 2. 抽帧节流器 (Analysis FPS Governor) 按需抽帧，降低 NPU 负载；
//! 3. 每帧解码结果实时同步更新至管线快照直通与保底队列；
//! 4. 抽帧通过单槽 Drop-Oldest 缓冲区送入专用常驻推理线程池 (InferenceWorkerHandle)，防范超载；
//! 5. 串行将推理结果输送至 `PipelineManager::process_detections`，规则触发告警时自动闭环执行靶向高清快照落地。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use media::decoder::VideoDecoder;
use media::stream_hub::CameraStreamSession;
use media::{ConsumerKind, StreamItem};
use tokio_util::sync::CancellationToken;
use types::{DetectionRule, FrameRef, MotionGateConfig, StreamTag, TrackedObject};

use infer::{InferenceWorker, InferenceWorkerHandle};

use crate::capture_settle::{CaptureAction, RecordCandidateOutcome};
use crate::events::{
    EvidenceStatus, PipelineAlarmEvent, PipelineAnalysisEvent, PipelineCaptureEvent,
    PipelineTrackEvent,
};
use crate::manager::PipelineManager;
use crate::motion_gate::MotionGate;
use crate::snapshot::SnapshotResult;

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
    /// 算法类型 (如 "detection", "recognition")
    pub algorithm_type: String,
    /// 目标分析 FPS (0 表示不限帧率)
    pub target_fps: u32,
    /// 创建算法实例时使用的 JSON 配置
    pub config_json: Option<String>,
}

/// 有效分析码流驱动泵配置
#[derive(Debug, Clone)]
pub struct AnalysisPumpConfig {
    /// 目标分析抽帧率 (0 表示不限帧率全量抽帧)
    pub target_fps: u32,
    /// 完整运动门控配置；`None` 表示不启用门控
    pub motion_gate: Option<MotionGateConfig>,
}

/// 解码泵运行时所需的运动门控与动态空间规则句柄。
#[derive(Debug, Clone)]
pub struct MotionGateRuntimeConfig {
    pub gate: Option<MotionGateConfig>,
    pub rules: Arc<tokio::sync::RwLock<Vec<DetectionRule>>>,
    pub rules_version: Arc<std::sync::atomic::AtomicU64>,
}

impl Default for MotionGateRuntimeConfig {
    fn default() -> Self {
        Self {
            gate: None,
            rules: Arc::new(tokio::sync::RwLock::new(Vec::new())),
            rules_version: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }
}

impl Default for AnalysisPumpConfig {
    fn default() -> Self {
        Self {
            target_fps: 10,
            motion_gate: None,
        }
    }
}

/// 历史公共名称，保留源码兼容性；新的代码应使用 [`AnalysisPumpConfig`]。
pub type SubStreamPumpConfig = AnalysisPumpConfig;

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

    async fn reset(&mut self) -> Result<(), media::error::MediaError> {
        self.inner
            .as_mut()
            .expect("decoder owner must remain present")
            .reset()
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
    /// 累计因丢帧恢复而重放的视频包数
    pub replay_packets: AtomicU64,
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
    /// 因每路候选字节预算已满而被拒绝的峰值候选数。
    ///
    /// 该计数器上升说明结算将回退到当帧/环内证据（质量下降），是容量调优的唯一依据；
    /// 不暴露它就只能看到“抓拍变差”，看不到“为何变差”。
    pub candidate_budget_rejections: AtomicU64,
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

        let sample = match self.last_sampled_pts {
            None => true,
            Some(last_pts) => pts_ms < last_pts || (pts_ms - last_pts) >= self.interval_ms,
        };

        if sample {
            self.last_sampled_pts = Some(pts_ms);
        }
        sample
    }

    #[inline]
    pub fn target_fps(&self) -> u32 {
        self.target_fps
    }
}

/// 解码采样帧及其捕获时的 tracker 会话代际。
pub(crate) struct SampledFrame {
    pub(crate) frame: FrameRef,
    pub(crate) generation: u64,
}

/// 内部单槽采样传递队列 (Drop-Oldest 单槽缓冲)
pub(crate) struct SamplingSlot {
    pub(crate) frame: std::sync::Mutex<Option<SampledFrame>>,
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

/// 辅助执行单目标快照抓拍并统一累加指标与记录结构化日志
struct SnapshotRecorder<'a> {
    pipeline_mgr: &'a PipelineManager,
    camera_id: &'a str,
    algorithm_id: &'a str,
    timestamp: i64,
    analyzed_frame: FrameRef,
    infer_metrics: &'a InstanceMetrics,
    pump_metrics: &'a PumpMetrics,
}

impl<'a> SnapshotRecorder<'a> {
    async fn capture(
        &self,
        bbox: types::BoundingBox,
        context_desc: &str,
    ) -> Result<SnapshotResult, String> {
        match self
            .pipeline_mgr
            .trigger_snapshot_for_frame(
                self.camera_id,
                self.timestamp,
                Some(bbox),
                self.analyzed_frame.clone(),
            )
            .await
        {
            Ok(snapshot_res) => {
                self.infer_metrics
                    .snapshots_saved
                    .fetch_add(1, Ordering::Relaxed);
                self.pump_metrics
                    .snapshots_saved
                    .fetch_add(1, Ordering::Relaxed);
                tracing::info!(
                    camera_id = %self.camera_id,
                    algorithm_id = %self.algorithm_id,
                    target_pts = self.timestamp,
                    path = %snapshot_res.image_rel_path,
                    is_fallback = snapshot_res.is_sub_stream(),
                    "{context_desc}快照落地成功"
                );
                Ok(snapshot_res)
            }
            Err(err) => {
                let err_str = err.to_string();
                tracing::error!(
                    camera_id = %self.camera_id,
                    algorithm_id = %self.algorithm_id,
                    target_pts = self.timestamp,
                    error = %err,
                    "{context_desc}快照捕获失败"
                );
                Err(err_str)
            }
        }
    }
}

/// 消费识别类抓拍结算动作：留存峰值候选 / 结算并发射通行抓拍事件。
///
/// - `RetainCandidate`：复用当帧 `analyzed_frame` 编码峰值候选并把字节驻留内存（覆盖旧候选，软失败）；
/// - `Settle`：优先把内存候选一次性写入正式证据目录；写盘失败或本无候选时回退同帧快照或按
///   最后可见 PTS 取证。`tracked_object` 的 bbox 在候选路径下替换为峰值帧几何（INV-3）。
#[allow(clippy::too_many_arguments)]
async fn execute_capture_actions(
    pipeline_mgr: &PipelineManager,
    camera_id: &str,
    algorithm_id: &str,
    actions: Vec<CaptureAction>,
    analyzed_frame: &FrameRef,
    timestamp: i64,
    infer_metrics: &InstanceMetrics,
    pump_metrics: &PumpMetrics,
) {
    for action in actions {
        match action {
            CaptureAction::RetainCandidate(request) => {
                match pipeline_mgr
                    .retain_capture_candidate(
                        camera_id,
                        algorithm_id,
                        &request,
                        analyzed_frame.clone(),
                    )
                    .await
                {
                    Ok(RecordCandidateOutcome::Accepted)
                    | Ok(RecordCandidateOutcome::Dismissed) => {}
                    Ok(RecordCandidateOutcome::RejectedBudget) => {
                        pump_metrics
                            .candidate_budget_rejections
                            .fetch_add(1, Ordering::Relaxed);
                    }
                    Err(error) => {
                        tracing::debug!(
                            camera_id,
                            algorithm_id,
                            track_id = request.track_id,
                            pts = request.geometry.pts_ms,
                            error = %error,
                            "峰值候选编码留存失败 (软失败，不阻塞分析循环)"
                        );
                    }
                }
            }
            CaptureAction::Settle(settle) => {
                let settle = *settle;
                let mut candidate_geometry = None;
                let mut snapshot = None;
                if let Some(candidate) = settle.candidate.as_ref() {
                    // 结算即唯一一次落盘：此前候选仅以编码字节驻留内存。
                    match pipeline_mgr
                        .write_capture_candidate(camera_id, candidate)
                        .await
                    {
                        Ok(result) => {
                            snapshot = Some(result);
                            candidate_geometry = Some(candidate.geometry);
                        }
                        Err(error) => {
                            tracing::warn!(
                                camera_id,
                                algorithm_id,
                                track_id = settle.track_id,
                                error = %error,
                                "峰值候选写盘失败，回退当帧/环内取证"
                            );
                        }
                    }
                }

                if snapshot.is_none() {
                    let crop_bbox = settle
                        .tracked_object
                        .face_bbox()
                        .unwrap_or(settle.tracked_object.bbox);
                    let capture_result = if settle.target_in_current_frame
                        && settle.last_seen_pts_ms == timestamp
                    {
                        pipeline_mgr
                            .trigger_snapshot_for_frame(
                                camera_id,
                                timestamp,
                                Some(crop_bbox),
                                analyzed_frame.clone(),
                            )
                            .await
                    } else {
                        pipeline_mgr
                            .trigger_snapshot(camera_id, settle.last_seen_pts_ms, Some(crop_bbox))
                            .await
                    };
                    match capture_result {
                        Ok(result) => snapshot = Some(result),
                        Err(error) => {
                            tracing::error!(
                                camera_id,
                                algorithm_id,
                                track_id = settle.track_id,
                                reason = settle.reason.as_str(),
                                error = %error,
                                "通行抓拍结算证据生成失败"
                            );
                        }
                    }
                }

                if let Some(result) = &snapshot {
                    // INV-3 审计：只有「无候选 → 回溯/回退取证」这条路径可能取到另一刻的帧。
                    // 候选路径不能进这个比对：它的 `frame_pts_ms` 是**峰值帧**时标，与
                    // `last_seen_pts_ms`（最后一次触发）本就不同，且事件几何已同步替换为峰值帧
                    // 几何，属于设计预期而非不一致。
                    // 子码流回退帧与检测帧同轴，可直接比对；主码流取证帧位于另一条 PTS 轴
                    // （见 `EvidenceTarget`），跨轴比较无意义。
                    if candidate_geometry.is_none()
                        && result.is_sub_stream()
                        && result.frame_pts_ms != settle.last_seen_pts_ms
                    {
                        tracing::warn!(
                            camera_id,
                            algorithm_id,
                            track_id = settle.track_id,
                            evidence_pts = result.frame_pts_ms,
                            event_pts = settle.last_seen_pts_ms,
                            "证据图与事件几何不在同一刻（取证降级），记录已保留但需审计"
                        );
                    }
                    infer_metrics
                        .snapshots_saved
                        .fetch_add(1, Ordering::Relaxed);
                    pump_metrics.snapshots_saved.fetch_add(1, Ordering::Relaxed);
                    tracing::info!(
                        camera_id,
                        algorithm_id,
                        track_id = settle.track_id,
                        reason = settle.reason.as_str(),
                        target_pts = settle.last_seen_pts_ms,
                        path = %result.image_rel_path,
                        // 目标 3 可追溯性：结算事件必须留下所用模板的成熟度凭据，
                        // 否则事后无法分辨“抓拍差”是质量模型未成熟还是取帧降级。
                        fused_count = tracing::field::debug(
                            settle.tracked_object.face.as_ref().and_then(|face| face.fused_count)
                        ),
                        template_quality = tracing::field::debug(
                            settle.tracked_object.face.as_ref().and_then(|face| face.template_quality)
                        ),
                        template_mature = tracing::field::debug(
                            settle.tracked_object.face.as_ref().and_then(|face| face.template_mature)
                        ),
                        "通行抓拍结算完成"
                    );
                }

                let mut event_object = settle.tracked_object;
                if let Some(geometry) = candidate_geometry {
                    // INV-3：事件 bbox 必须与所存图像同帧（候选提升时替换为峰值帧几何）。
                    event_object.bbox = geometry.bbox;
                    if let Some(face) = event_object.face.as_mut() {
                        face.bbox = geometry.face_bbox.unwrap_or(geometry.bbox);
                    }
                }

                pipeline_mgr.publish_analysis_event(PipelineAnalysisEvent::Capture(Box::new(
                    PipelineCaptureEvent {
                        capture_id: uuid::Uuid::now_v7().to_string(),
                        camera_id: camera_id.to_string(),
                        algorithm_id: algorithm_id.to_string(),
                        tracked_object: event_object,
                        snapshot,
                        timestamp: settle.last_seen_pts_ms,
                    },
                )));
            }
        }
    }
}

/// 有效分析码流驱动泵
pub struct AnalysisPump {
    camera_id: String,
    cancel_token: CancellationToken,
    decode_handle: Option<tokio::task::JoinHandle<()>>,
    metrics: Arc<PumpMetrics>,
    control_slots: Arc<SharedControlSlots>,
}

impl std::fmt::Debug for AnalysisPump {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnalysisPump")
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

impl AnalysisPump {
    /// 启动有效分析码流驱动泵 (使用外部推理 Handle，向后兼容单 worker 模式)
    pub fn start(
        camera_id: impl Into<String>,
        session: Arc<CameraStreamSession>,
        decoder: Box<dyn VideoDecoder + Send>,
        worker: InferenceWorkerHandle,
        pipeline_mgr: Arc<PipelineManager>,
        config: AnalysisPumpConfig,
    ) -> Self {
        Self::start_multi_worker(
            camera_id,
            session,
            decoder,
            vec![WorkerInstanceConfig {
                algorithm_id: LEGACY_SINGLE_WORKER_ID.to_string(),
                algorithm_type: "detection".to_string(),
                target_fps: config.target_fps,
                config_json: None,
            }],
            vec![(LEGACY_SINGLE_WORKER_ID.to_string(), worker, None)],
            pipeline_mgr,
            MotionGateRuntimeConfig {
                gate: config.motion_gate,
                ..Default::default()
            },
        )
    }

    /// 启动有效分析码流驱动泵并由驱动泵托管 InferenceWorker 运行周期 (向后兼容)
    pub fn start_with_worker(
        camera_id: impl Into<String>,
        session: Arc<CameraStreamSession>,
        decoder: Box<dyn VideoDecoder + Send>,
        worker: infer::InferenceWorker,
        pipeline_mgr: Arc<PipelineManager>,
        config: AnalysisPumpConfig,
    ) -> Self {
        let handle = worker.handle();
        Self::start_multi_worker(
            camera_id,
            session,
            decoder,
            vec![WorkerInstanceConfig {
                algorithm_id: LEGACY_SINGLE_WORKER_ID.to_string(),
                algorithm_type: "detection".to_string(),
                target_fps: config.target_fps,
                config_json: None,
            }],
            vec![(LEGACY_SINGLE_WORKER_ID.to_string(), handle, Some(worker))],
            pipeline_mgr,
            MotionGateRuntimeConfig {
                gate: config.motion_gate,
                ..Default::default()
            },
        )
    }

    /// 启动多算法实例有效分析码流驱动泵
    pub fn start_multi_worker(
        camera_id: impl Into<String>,
        session: Arc<CameraStreamSession>,
        decoder: Box<dyn VideoDecoder + Send>,
        instance_configs: Vec<WorkerInstanceConfig>,
        workers: Vec<(String, InferenceWorkerHandle, Option<InferenceWorker>)>,
        pipeline_mgr: Arc<PipelineManager>,
        motion: MotionGateRuntimeConfig,
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

        let packet_subscription = match session
            .dispatcher
            .subscribe(format!("analysis:{camera_id}"), ConsumerKind::Analysis)
        {
            Ok(subscription) => Some(subscription),
            Err(error) => {
                tracing::error!(camera_id = %camera_id, error = %error, "分析消费者订阅失败，分析泵将退出");
                None
            }
        };

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
            let algorithm_type_infer = cfg.algorithm_type.clone();

            let infer_handle = tokio::spawn(async move {
                tracing::info!(
                    camera_id = %cam_id_infer,
                    algorithm_id = %algorithm_id_infer,
                    "多算法实例推理循环已启动"
                );

                loop {
                    tokio::select! {
                        biased;

                        _ = infer_cancel.cancelled() => {
                            tracing::info!(
                                camera_id = %cam_id_infer,
                                algorithm_id = %algorithm_id_infer,
                                "多算法实例推理循环收到关停信号"
                            );
                            break;
                        }

                        _ = infer_slot.notify.notified() => {
                            let maybe_frame = infer_slot.frame.lock().ok().and_then(|mut g| g.take());
                            if let Some(sampled_frame) = maybe_frame {
                                let timestamp = sampled_frame.frame.timestamp;
                                let generation = sampled_frame.generation;
                                let current_worker = infer_worker_holder.read().await.clone();
                                let analyzed_frame = sampled_frame.frame.clone();
                                match current_worker
                                    .submit_with_metadata(sampled_frame.frame)
                                    .await
                                {
                                    Ok(inference_result) => {
                                        let infer::InferenceResult {
                                            detections,
                                            embeddings,
                                        } = inference_result;
                                        infer_metrics
                                            .frames_inferred
                                            .fetch_add(1, Ordering::Relaxed);
                                        pump_metrics
                                            .frames_inferred
                                            .fetch_add(1, Ordering::Relaxed);

                                        // 驱动管线执行独立算法实例的航迹跟踪与几何规则判定 (保序执行)
                                        let outcome = pipeline_mgr_infer
                                            .process_detections_for_algo_with_embeddings_at_generation(
                                                &cam_id_infer,
                                                &algorithm_id_infer,
                                                &algorithm_type_infer,
                                                detections,
                                                embeddings,
                                                timestamp,
                                                generation,
                                            )
                                            .await;
                                        if !outcome.applied {
                                            tracing::debug!(
                                                camera_id = %cam_id_infer,
                                                algorithm_id = %algorithm_id_infer,
                                                timestamp,
                                                "迟到或被拒绝的推理结果未进入航迹与规则链"
                                            );
                                            continue;
                                        }

                                        // 广播航迹追踪事件 (供前端低延迟实时绘制元数据)
                                        pipeline_mgr_infer.publish_analysis_event(
                                            PipelineAnalysisEvent::Tracks(PipelineTrackEvent {
                                                camera_id: cam_id_infer.clone(),
                                                algorithm_id: algorithm_id_infer.clone(),
                                                timestamp,
                                                tracks: outcome
                                                    .tracked
                                                    .iter()
                                                    .map(TrackedObject::without_embedding)
                                                    .collect(),
                                            }),
                                        );

                                        // 识别类算法：通行抓拍处理 (不产生告警，直接落地 capture_records)
                                        let snapshot_recorder = SnapshotRecorder {
                                            pipeline_mgr: &pipeline_mgr_infer,
                                            camera_id: &cam_id_infer,
                                            algorithm_id: &algorithm_id_infer,
                                            timestamp,
                                            analyzed_frame: analyzed_frame.clone(),
                                            infer_metrics: &infer_metrics,
                                            pump_metrics: &pump_metrics,
                                        };

                                        if !outcome.capture_actions.is_empty() {
                                            execute_capture_actions(
                                                &pipeline_mgr_infer,
                                                &cam_id_infer,
                                                &algorithm_id_infer,
                                                outcome.capture_actions,
                                                &analyzed_frame,
                                                timestamp,
                                                &infer_metrics,
                                                &pump_metrics,
                                            )
                                            .await;
                                        }

                                        // 检测类算法：安全防范规则告警处理 (联动高清快照与 alarm.triggered 广播)
                                        if !outcome.alarms.is_empty() {
                                            infer_metrics
                                                .alarms_triggered
                                                .fetch_add(outcome.alarms.len() as u64, Ordering::Relaxed);
                                            pump_metrics
                                                .alarms_triggered
                                                .fetch_add(outcome.alarms.len() as u64, Ordering::Relaxed);
                                            for alarm in outcome.alarms {
                                                let event_id = uuid::Uuid::now_v7().to_string();
                                                let (snapshot, evidence_status, evidence_error) =
                                                    match snapshot_recorder
                                                        .capture(
                                                            alarm.tracked_object.bbox,
                                                            "规则引擎告警",
                                                        )
                                                        .await
                                                    {
                                                        Ok(res) => {
                                                            (Some(res), EvidenceStatus::Ready, None)
                                                        }
                                                        Err(err_str) => (
                                                            None,
                                                            EvidenceStatus::Failed,
                                                            Some(err_str),
                                                        ),
                                                    };

                                                pipeline_mgr_infer.publish_analysis_event(
                                                    PipelineAnalysisEvent::Alarm(Box::new(
                                                        PipelineAlarmEvent {
                                                            event_id,
                                                            camera_id: cam_id_infer.clone(),
                                                            algorithm_id: algorithm_id_infer.clone(),
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
                                        if pipeline_mgr_infer
                                            .expire_tracking_for_algo_at(
                                                &cam_id_infer,
                                                &algorithm_id_infer,
                                                timestamp,
                                            )
                                            .await
                                        {
                                            tracing::debug!(
                                                camera_id = %cam_id_infer,
                                                algorithm_id = %algorithm_id_infer,
                                                timestamp,
                                                "推理连续失败，已清理超时宿主航迹"
                                            );
                                        }
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
                    "多算法实例推理循环已平稳退出"
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
            let tracking_generation = pipeline_mgr_decode
                .get_or_create_context(&cam_id)
                .await
                .tracking_generation
                .clone();
            tracing::info!(
                camera_id = %cam_id,
                slot_count,
                "多算法驱动泵解码循环已启动"
            );

            let mut motion_gate = motion.gate.map(MotionGate::new);
            let motion_rules = motion.rules;
            let motion_rules_version = motion.rules_version;
            let mut applied_rules_version: u64 = 0;

            let mut replay_queue: VecDeque<Arc<types::EncodedPacket>> = VecDeque::new();

            loop {
                let (stream_item, is_replay) = if let Some(packet) = replay_queue.pop_front() {
                    (Some(StreamItem::Packet(packet)), true)
                } else {
                    let Some(subscription) = packet_subscription.as_ref() else {
                        break;
                    };
                    tokio::select! {
                        biased;
                        _ = decode_cancel.cancelled() => {
                            tracing::info!(camera_id = %cam_id, "分析码流解码驱动循环收到关停信号");
                            break;
                        }
                        item = subscription.recv() => (item, false),
                    }
                };

                let Some(stream_item) = stream_item else {
                    tracing::info!(camera_id = %cam_id, "分析消费者 mailbox 已关闭，解码循环退出");
                    break;
                };

                let pkt = match stream_item {
                    StreamItem::Packet(packet) => packet,
                    StreamItem::Replay(snapshot) => {
                        if let Err(error) = decoder.reset().await {
                            tracing::warn!(camera_id = %cam_id, error = %error, "分析解码器 Replay 重置失败");
                            continue;
                        }
                        replay_queue.extend(snapshot.packets.iter().cloned());
                        continue;
                    }
                    StreamItem::SourceReset { epoch } => {
                        replay_queue.clear();
                        if let Err(error) = decoder.reset().await {
                            tracing::warn!(camera_id = %cam_id, epoch, error = %error, "源流重建后分析解码器重置失败");
                        }
                        pipeline_mgr_decode.clear_tracking(&cam_id).await;
                        continue;
                    }
                };

                if pkt.stream_tag == StreamTag::Audio || !pkt.codec.is_video() {
                    continue;
                }

                if is_replay {
                    metrics_clone.replay_packets.fetch_add(1, Ordering::Relaxed);
                } else {
                    metrics_clone
                        .packets_received
                        .fetch_add(1, Ordering::Relaxed);
                }

                match decoder.decode_packet(&pkt.payload, pkt.pts_ms).await {
                    Ok(Some(frame)) => {
                        if !is_replay {
                            metrics_clone.frames_decoded.fetch_add(1, Ordering::Relaxed);

                            // 1. 实时更新管线保底快照与零解码直通候选帧
                            pipeline_mgr_decode
                                .update_decoded_frame(&cam_id, frame.clone())
                                .await;

                            // 2. 运动门控过滤：静止帧跳过所有槽位推理，节省算力
                            if let Some(gate) = motion_gate.as_mut() {
                                let current_rules_ver =
                                    motion_rules_version.load(Ordering::Acquire);
                                if current_rules_ver != applied_rules_version {
                                    let latest_rules = motion_rules.read().await.clone();
                                    gate.update_rules(
                                        &latest_rules,
                                        frame.width as usize,
                                        frame.height as usize,
                                    );
                                    applied_rules_version = current_rules_ver;
                                }

                                let decision = gate.evaluate_frame(&frame, frame.timestamp);
                                pipeline_mgr_decode
                                    .report_motion_telemetry(
                                        &cam_id,
                                        frame.timestamp,
                                        decision.motion_score,
                                        decision.should_skip,
                                    )
                                    .await;
                                if decision.should_skip {
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
                                        let generation =
                                            tracking_generation.load(Ordering::Acquire);
                                        if slot_guard
                                            .replace(SampledFrame {
                                                frame: frame.clone(),
                                                generation,
                                            })
                                            .is_some()
                                        {
                                            slot.metrics
                                                .frames_dropped
                                                .fetch_add(1, Ordering::Relaxed);
                                            metrics_clone
                                                .frames_dropped
                                                .fetch_add(1, Ordering::Relaxed);
                                        }
                                    }
                                    slot.sampling_slot.notify.notify_one();
                                }
                            }
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        metrics_clone.decode_errors.fetch_add(1, Ordering::Relaxed);
                        tracing::warn!(camera_id = %cam_id, error = %e, "解码分析码流数据包失败");
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
                    tracing::warn!(camera_id = %cam_id, error = %err, "分析码流解码器 flush 失败")
                }
                Err(_) => tracing::error!(
                    camera_id = %cam_id,
                    timeout_ms = media::decoder::DEFAULT_THREAD_SHUTDOWN_TIMEOUT.as_millis() as u64,
                    "分析码流解码器 flush 超时，隔离硬件句柄"
                ),
            }
            // 退出前清空已解码缓冲队列，提前释放硬件池租约 (DMA-BUF / 显存)
            pipeline_mgr_decode.clear_decoded_ring(&cam_id).await;
            pipeline_mgr_decode.clear_tracking(&cam_id).await;
            decoder.dispose().await;

            tracing::info!(camera_id = %cam_id, "分析码流驱动泵解码循环已完全停止并清理资源");
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
                    tracing::warn!(camera_id = %self.camera_id, error = %err, "分析码流解码任务异常退出")
                }
                Err(_) => tracing::error!(
                    camera_id = %self.camera_id,
                    timeout_ms = DEFAULT_PUMP_SHUTDOWN_TIMEOUT.as_millis() as u64,
                    "分析码流解码任务停止超时，保留后台句柄隔离"
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

impl Drop for AnalysisPump {
    fn drop(&mut self) {
        if !self.cancel_token.is_cancelled() {
            self.cancel_token.cancel();
        }
    }
}

/// 历史公共名称，保留源码兼容性；新的代码应使用 [`AnalysisPump`]。
pub type SubStreamAnalysisPump = AnalysisPump;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture_settle::{CandidateRetainRequest, FrameGeometry};
    use crate::test_support::{dir_entry_count, test_nv12_frame};
    use types::{BoundingBox, FaceDetail};

    fn capture_object(track_id: u64, bbox: BoundingBox, quality: f32) -> TrackedObject {
        TrackedObject {
            track_id,
            class_id: 0,
            label: "face".to_string(),
            confidence: 0.95,
            quality_score: Some(quality),
            embedding: None,
            bbox,
            face: Some(FaceDetail {
                bbox: BoundingBox::new(0.45, 0.35, 0.55, 0.5),
                confidence: 0.95,
                quality_score: Some(quality),
                fused_count: None,
                template_quality: None,
                template_mature: None,
                embedding: None,
            }),
            trajectory: vec![(0.5, 0.6)],
        }
    }

    /// 结算动作必须把内存候选一次性写入正式证据目录，并将事件 bbox 对齐到所存图像同帧（INV-3）。
    #[tokio::test]
    async fn settle_action_writes_memory_candidate_and_aligns_event_geometry() {
        let temp_dir = std::env::temp_dir().join(format!(
            "test_capture_actions_{}",
            uuid::Uuid::now_v7().simple()
        ));
        let manager = PipelineManager::with_all_options(
            temp_dir.clone(),
            crate::snapshot::SnapshotConfig::default(),
            2,
            1000,
        );
        let camera_id = "cam_action";
        let algorithm_id = "algo_face";
        let ctx = manager.get_or_create_context(camera_id).await;
        let mut events = manager.subscribe_analysis_events();

        // 峰值帧几何与当前帧刻意不同，用于验证事件几何被替换为峰值帧。
        let peak_bbox = BoundingBox::new(0.4, 0.3, 0.6, 0.6);
        let peak_face_bbox = BoundingBox::new(0.45, 0.35, 0.55, 0.5);
        let peak_object = capture_object(3, peak_bbox, 0.86);
        let current_bbox = BoundingBox::new(0.5, 0.4, 0.7, 0.7);
        let current_object = capture_object(3, current_bbox, 0.80);

        let _ = {
            let mut settle = ctx
                .capture_settle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            settle.observe(algorithm_id, std::slice::from_ref(&peak_object), &[], 1000)
        };
        manager
            .retain_capture_candidate(
                camera_id,
                algorithm_id,
                &CandidateRetainRequest {
                    track_id: 3,
                    geometry: FrameGeometry {
                        bbox: peak_bbox,
                        face_bbox: Some(peak_face_bbox),
                        pts_ms: 1000,
                        quality: 0.86,
                    },
                },
                test_nv12_frame(camera_id, 1000),
            )
            .await
            .expect("峰值候选留存失败");
        assert_eq!(dir_entry_count(&temp_dir), 0, "留存阶段不得产生盘上产物");

        let actions = {
            let mut settle = ctx
                .capture_settle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            settle.observe(
                algorithm_id,
                std::slice::from_ref(&current_object),
                &[],
                1600,
            )
        };
        assert_eq!(actions.len(), 1, "平台期应只产出结算动作");

        let infer_metrics = InstanceMetrics::default();
        let pump_metrics = PumpMetrics::default();
        execute_capture_actions(
            &manager,
            camera_id,
            algorithm_id,
            actions,
            &test_nv12_frame(camera_id, 1600),
            1600,
            &infer_metrics,
            &pump_metrics,
        )
        .await;

        match events.recv().await.expect("通行抓拍事件") {
            PipelineAnalysisEvent::Capture(event) => {
                assert_eq!(event.timestamp, 1600);
                assert_eq!(
                    event.tracked_object.bbox, peak_bbox,
                    "INV-3：事件 bbox 必须与所存图像同帧"
                );
                assert_eq!(
                    event.tracked_object.face.as_ref().expect("人脸细节").bbox,
                    peak_face_bbox,
                    "INV-3：人脸 bbox 同样需对齐峰值帧"
                );
                let snapshot = event.snapshot.expect("候选结算必须产出证据快照");
                assert!(snapshot
                    .image_rel_path
                    .starts_with(&format!("{camera_id}/")));
                assert!(temp_dir.join(&snapshot.image_rel_path).is_file());
                assert!(temp_dir.join(&snapshot.crop_image_rel_path).is_file());
            }
            other => panic!("期望通行抓拍事件，实际为 {other:?}"),
        }
        assert_eq!(infer_metrics.snapshots_saved.load(Ordering::Relaxed), 1);
        assert_eq!(
            dir_entry_count(&temp_dir.join(camera_id)),
            2,
            "结算只应写下全景与特写两份正式证据"
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    /// 无候选可用（弱帧播种后未生成候选）时，结算必须回退为同帧快照取证，不得丢失事件。
    #[tokio::test]
    async fn settle_action_without_candidate_falls_back_to_frame_snapshot() {
        let temp_dir = std::env::temp_dir().join(format!(
            "test_capture_fallback_{}",
            uuid::Uuid::now_v7().simple()
        ));
        let manager = PipelineManager::with_all_options(
            temp_dir.clone(),
            crate::snapshot::SnapshotConfig::default(),
            2,
            1000,
        );
        let camera_id = "cam_fallback";
        let algorithm_id = "algo_face";
        let ctx = manager.get_or_create_context(camera_id).await;
        let mut events = manager.subscribe_analysis_events();

        let object = capture_object(9, BoundingBox::new(0.4, 0.3, 0.6, 0.6), 0.86);
        // 只推进状态机、不执行 RetainCandidate → 控制器无候选可写盘。
        let _ = {
            let mut settle = ctx
                .capture_settle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            settle.observe(algorithm_id, std::slice::from_ref(&object), &[], 1000)
        };
        let actions = {
            let mut settle = ctx
                .capture_settle
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            settle.observe(algorithm_id, std::slice::from_ref(&object), &[], 1600)
        };

        let infer_metrics = InstanceMetrics::default();
        let pump_metrics = PumpMetrics::default();
        execute_capture_actions(
            &manager,
            camera_id,
            algorithm_id,
            actions,
            &test_nv12_frame(camera_id, 1600),
            1600,
            &infer_metrics,
            &pump_metrics,
        )
        .await;

        match events.recv().await.expect("通行抓拍事件") {
            PipelineAnalysisEvent::Capture(event) => {
                let snapshot = event.snapshot.expect("回退路径仍必须产出证据快照");
                assert!(snapshot
                    .image_rel_path
                    .starts_with(&format!("{camera_id}/")));
                assert!(temp_dir.join(&snapshot.image_rel_path).is_file());
            }
            other => panic!("期望通行抓拍事件，实际为 {other:?}"),
        }

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

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
