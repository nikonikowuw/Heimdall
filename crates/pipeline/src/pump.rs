//! 分析码流驱动泵 (Analysis Pump)
//!
//! 核心职责：
//! 1. 订阅已决议的分析码流广播包，维持 H.264/H.265 解码器完整参考帧链；
//! 2. 抽帧节流器 (Analysis FPS Governor) 按需抽帧，降低 NPU 负载；
//! 3. 每帧解码结果实时同步更新至管线快照直通与保底队列；
//! 4. 抽帧通过单槽 Drop-Oldest 缓冲区送入专用常驻推理线程池 (InferenceWorkerHandle)，防范超载；
//! 5. 串行将推理结果输送至 `PipelineManager::process_detections`，规则触发告警时自动闭环执行靶向高清快照落地。

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
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
    /// 因所属 Worker 代际已被替换而丢弃的迟到推理结果数
    pub stale_results: AtomicU64,
}

/// 单个算法实例的运行时控制面描述（增量收敛的当前值快照）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceDescriptor {
    pub instance_id: String,
    pub algorithm_id: String,
    /// 当前生效的抽帧目标帧率
    pub target_fps: u32,
    /// 当前生效的算法配置 JSON
    pub config_json: Option<String>,
}

/// 单个算法实例的多工配置
#[derive(Debug, Clone)]
pub struct WorkerInstanceConfig {
    /// 算法实例唯一标识（运行时寻址主键）
    pub instance_id: String,
    /// 算法包 ID（用于加载包与展示名称，不作为运行时寻址依据）
    pub algorithm_id: String,
    /// 算法类型 (如 "detection", "recognition")
    pub algorithm_type: String,
    /// 目标分析 FPS (0 表示不限帧率)
    pub target_fps: u32,
    /// 创建算法实例时使用的 JSON 配置
    pub config_json: Option<String>,
}

/// 算法实例槽位控制命令
///
/// 解码循环独占抽帧槽，因此拓扑变更必须交给它执行；命令在解码帧边界被消费，
/// 保证抽帧槽与结果代际不会出现半帧状态。
enum PumpCommand {
    /// 新增算法实例抽帧槽（Worker 与推理循环已在控制面就绪）
    AddInstance {
        slot: DecodeSlot,
        reply: tokio::sync::oneshot::Sender<bool>,
    },
    /// 移除算法实例抽帧槽
    RemoveInstance {
        instance_id: String,
        reply: tokio::sync::oneshot::Sender<bool>,
    },
    /// 在帧边界替换抽帧 governor
    SetAnalysisFps {
        instance_id: String,
        target_fps: u32,
        reply: tokio::sync::oneshot::Sender<bool>,
    },
}

/// 算法实例控制命令通道容量；满载时控制面得到明确的忙错误，不静默丢失配置命令。
pub(crate) const PUMP_COMMAND_CHANNEL_CAPACITY: usize = 16;

/// 单条拓扑命令在解码帧边界被消费的等待上限。
/// 超时说明解码循环已停止或卡死，调用方必须按未生效处理而不是假定成功。
const PUMP_COMMAND_ACK_TIMEOUT: Duration = Duration::from_secs(2);

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

/// 解码采样帧及其捕获时的 tracker 会话代际与所属槽位代际。
pub(crate) struct SampledFrame {
    pub(crate) frame: FrameRef,
    /// 管线跟踪代际（摄像机会话级栅栏）
    pub(crate) generation: u64,
    /// 算法实例槽位代际；Worker 被替换后旧代际的推理结果必须丢弃
    pub(crate) slot_generation: u64,
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
    instance_id: String,
    governor: AnalysisFpsGovernor,
    sampling_slot: Arc<SamplingSlot>,
    metrics: Arc<InstanceMetrics>,
    /// 与控制面共享的生效帧率；帧边界变更时同步写回，控制面据此做增量 diff
    target_fps: Arc<AtomicU32>,
    /// 与控制面共享的 Worker 代际；替换 Worker 时在控制面递增，此处仅在采样时读取
    worker_generation: Arc<AtomicU64>,
}

/// 控制面持有的算法实例槽（用于 Worker 热重载、优雅退出与指标查询）
struct ControlSlot {
    instance_id: String,
    algorithm_id: String,
    config_json: Option<String>,
    worker_holder: Arc<tokio::sync::RwLock<InferenceWorkerHandle>>,
    managed_worker: Option<InferenceWorker>,
    infer_handle: Option<tokio::task::JoinHandle<()>>,
    metrics: Arc<InstanceMetrics>,
    /// 生效抽帧帧率，与抽帧槽共享同一个原子量
    target_fps: Arc<AtomicU32>,
    /// 该实例 Worker 的代际，与抽帧槽共享同一个原子量
    worker_generation: Arc<AtomicU64>,
    /// 该实例推理循环的独立取消令牌（父令牌取消时同步取消）
    cancel: CancellationToken,
}

/// 单个算法实例推理运行时的构造产物
struct InstanceRuntimeParts {
    instance_id: String,
    algorithm_id: String,
    config_json: Option<String>,
    metrics: Arc<InstanceMetrics>,
    target_fps: Arc<AtomicU32>,
    worker_holder: Arc<tokio::sync::RwLock<InferenceWorkerHandle>>,
    worker_generation: Arc<AtomicU64>,
    cancel: CancellationToken,
    infer_handle: tokio::task::JoinHandle<()>,
}

impl InstanceRuntimeParts {
    fn into_control_slot(self, managed_worker: Option<InferenceWorker>) -> ControlSlot {
        ControlSlot {
            instance_id: self.instance_id,
            algorithm_id: self.algorithm_id,
            config_json: self.config_json,
            worker_holder: self.worker_holder,
            managed_worker,
            infer_handle: Some(self.infer_handle),
            metrics: self.metrics,
            target_fps: self.target_fps,
            worker_generation: self.worker_generation,
            cancel: self.cancel,
        }
    }
}

/// 创建并启动单算法实例的推理循环。
///
/// 冷启动与运行时增量新增共用同一条装配路径，避免两套生命周期实现漂移。
#[allow(clippy::too_many_arguments)]
fn spawn_instance_runtime(
    parent_cancel: &CancellationToken,
    camera_id: &str,
    instance_id: &str,
    algorithm_id: &str,
    algorithm_type: &str,
    handle: InferenceWorkerHandle,
    config_json: Option<String>,
    target_fps: u32,
    pipeline_mgr: Arc<PipelineManager>,
    pump_metrics: Arc<PumpMetrics>,
) -> (InstanceRuntimeParts, DecodeSlot) {
    let sampling_slot = Arc::new(SamplingSlot::new());
    let instance_metrics = Arc::new(InstanceMetrics::default());
    let worker_generation = Arc::new(AtomicU64::new(1));
    let shared_target_fps = Arc::new(AtomicU32::new(target_fps));

    let infer_cancel = parent_cancel.child_token();
    let loop_cancel = infer_cancel.clone();
    let infer_slot = sampling_slot.clone();
    let worker_holder = Arc::new(tokio::sync::RwLock::new(handle));
    let infer_worker_holder = worker_holder.clone();
    let infer_metrics = instance_metrics.clone();
    let pump_metrics_infer = pump_metrics.clone();
    let cam_id_infer = camera_id.to_string();
    let pipeline_mgr_infer = pipeline_mgr;
    let algorithm_id_infer = algorithm_id.to_string();
    let algorithm_type_infer = algorithm_type.to_string();
    let instance_id_infer = instance_id.to_string();
    let infer_generation = worker_generation.clone();

    let infer_handle = tokio::spawn(async move {
        tracing::info!(
            camera_id = %cam_id_infer,
            instance_id = %instance_id_infer,
            algorithm_id = %algorithm_id_infer,
            "多算法实例推理循环已启动"
        );

        loop {
            tokio::select! {
                biased;

                _ = loop_cancel.cancelled() => {
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
                        let slot_generation = sampled_frame.slot_generation;
                        let current_worker = infer_worker_holder.read().await.clone();
                        let analyzed_frame = sampled_frame.frame.clone();
                        match current_worker
                            .submit_with_metadata(sampled_frame.frame)
                            .await
                        {
                            Ok(inference_result) => {
                                // Worker 代际栅栏：替换期间旧 Worker 的在途结果必须丢弃，
                                // 否则旧模型的检测框会混入新配置的航迹与告警链。
                                if slot_generation != infer_generation.load(Ordering::Acquire) {
                                    infer_metrics
                                        .stale_results
                                        .fetch_add(1, Ordering::Relaxed);
                                    tracing::debug!(
                                        camera_id = %cam_id_infer,
                                        algorithm_id = %algorithm_id_infer,
                                        slot_generation,
                                        "丢弃已被替换 Worker 的迟到推理结果"
                                    );
                                    continue;
                                }

                                let infer::InferenceResult {
                                    detections,
                                    embeddings,
                                } = inference_result;
                                infer_metrics
                                    .frames_inferred
                                    .fetch_add(1, Ordering::Relaxed);
                                pump_metrics_infer
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
                                    pump_metrics: &pump_metrics_infer,
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
                                        &pump_metrics_infer,
                                    )
                                    .await;
                                }

                                // 检测类算法：安全防范规则告警处理 (联动高清快照与 alarm.triggered 广播)
                                if !outcome.alarms.is_empty() {
                                    infer_metrics
                                        .alarms_triggered
                                        .fetch_add(outcome.alarms.len() as u64, Ordering::Relaxed);
                                    pump_metrics_infer
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
                            Err(err) => {
                                infer_metrics
                                    .inference_errors
                                    .fetch_add(1, Ordering::Relaxed);
                                pump_metrics_infer
                                    .inference_errors
                                    .fetch_add(1, Ordering::Relaxed);
                                tracing::warn!(
                                    camera_id = %cam_id_infer,
                                    algorithm_id = %algorithm_id_infer,
                                    error = %err,
                                    "多算法实例推理执行失败"
                                );
                                pipeline_mgr_infer
                                    .expire_tracking_for_algo_at(
                                        &cam_id_infer,
                                        &algorithm_id_infer,
                                        timestamp,
                                    )
                                    .await;
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

    let parts = InstanceRuntimeParts {
        instance_id: instance_id.to_string(),
        algorithm_id: algorithm_id.to_string(),
        config_json,
        metrics: instance_metrics.clone(),
        target_fps: shared_target_fps.clone(),
        worker_holder,
        worker_generation: worker_generation.clone(),
        cancel: infer_cancel,
        infer_handle,
    };
    let decode_slot = DecodeSlot {
        instance_id: instance_id.to_string(),
        governor: AnalysisFpsGovernor::new(target_fps),
        sampling_slot,
        metrics: instance_metrics,
        target_fps: shared_target_fps,
        worker_generation,
    };
    (parts, decode_slot)
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

    #[allow(clippy::result_large_err)] // 冲突时需把槽位原样退回给调用方回收，不能吞掉所有权
    async fn push(&self, slot: ControlSlot) -> Result<(), ControlSlot> {
        let mut guard = self.inner.lock().await;
        if guard.iter().any(|s| s.instance_id == slot.instance_id) {
            return Err(slot);
        }
        guard.push(slot);
        Ok(())
    }

    async fn remove(&self, instance_id: &str) -> Option<ControlSlot> {
        let mut guard = self.inner.lock().await;
        let index = guard
            .iter()
            .position(|slot| slot.instance_id == instance_id)?;
        Some(guard.remove(index))
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
            .map(|s| (s.instance_id.clone(), s.metrics.clone()))
            .collect()
    }

    /// 列出当前挂载的算法实例控制面描述
    async fn instance_descriptors(&self) -> Vec<InstanceDescriptor> {
        let guard = self.inner.lock().await;
        guard
            .iter()
            .map(|s| InstanceDescriptor {
                instance_id: s.instance_id.clone(),
                algorithm_id: s.algorithm_id.clone(),
                target_fps: s.target_fps.load(Ordering::Acquire),
                config_json: s.config_json.clone(),
            })
            .collect()
    }

    /// 回写实例已生效配置（热更新成功后保持期望配置可查）
    async fn set_config_json(&self, instance_id: &str, config_json: Option<String>) -> bool {
        let mut guard = self.inner.lock().await;
        guard
            .iter_mut()
            .find(|slot| slot.instance_id == instance_id)
            .map(|slot| slot.config_json = config_json)
            .is_some()
    }

    /// 在当前 Worker 硬件上下文内应用新配置；返回错误原因时旧配置继续生效
    async fn update_instance_config(
        &self,
        instance_id: &str,
        config_json: &str,
    ) -> InstanceConfigUpdateOutcome {
        let holder = {
            let guard = self.inner.lock().await;
            match guard.iter().find(|slot| slot.instance_id == instance_id) {
                Some(slot) => slot.worker_holder.clone(),
                None => return InstanceConfigUpdateOutcome::NotFound,
            }
        };
        let handle = holder.read().await.clone();

        match handle.update_config(config_json.to_string()).await {
            Ok(()) => {
                self.set_config_json(instance_id, Some(config_json.to_string()))
                    .await;
                InstanceConfigUpdateOutcome::Applied
            }
            Err(infer::InferError::Unsupported { .. }) => InstanceConfigUpdateOutcome::Unsupported,
            Err(error) => InstanceConfigUpdateOutcome::Rejected {
                reason: error.to_string(),
            },
        }
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

/// 在解码帧边界应用一条拓扑控制命令
///
/// 只做短时内存操作：抽帧槽增删与 governor 替换均不涉及 FFI、Worker 创建/关闭或 `.await`，
/// 保证解码参考帧链不被控制面拖慢。
fn apply_pump_command(command: PumpCommand, decode_slots: &mut Vec<DecodeSlot>, camera_id: &str) {
    match command {
        PumpCommand::AddInstance { slot, reply } => {
            let instance_id = slot.instance_id.clone();
            let accepted = !decode_slots
                .iter()
                .any(|existing| existing.instance_id == instance_id);
            if accepted {
                decode_slots.push(slot);
            }
            tracing::info!(
                camera_id = %camera_id,
                instance_id = %instance_id,
                accepted,
                slot_count = decode_slots.len(),
                "解码循环已在帧边界挂载算法实例抽帧槽"
            );
            let _ = reply.send(accepted);
        }
        PumpCommand::RemoveInstance { instance_id, reply } => {
            let before = decode_slots.len();
            decode_slots.retain(|slot| slot.instance_id != instance_id);
            let removed = decode_slots.len() != before;
            tracing::info!(
                camera_id = %camera_id,
                instance_id = %instance_id,
                removed,
                slot_count = decode_slots.len(),
                "解码循环已在帧边界卸载算法实例抽帧槽"
            );
            let _ = reply.send(removed);
        }
        PumpCommand::SetAnalysisFps {
            instance_id,
            target_fps,
            reply,
        } => {
            let applied = decode_slots
                .iter_mut()
                .find(|slot| slot.instance_id == instance_id)
                .map(|slot| {
                    slot.governor = AnalysisFpsGovernor::new(target_fps);
                    slot.target_fps.store(target_fps, Ordering::Release);
                })
                .is_some();
            tracing::info!(
                camera_id = %camera_id,
                instance_id = %instance_id,
                target_fps,
                applied,
                "解码循环已在帧边界更新算法实例抽帧频率"
            );
            let _ = reply.send(applied);
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

/// 算法实例原地配置更新结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstanceConfigUpdateOutcome {
    /// 新配置已在目标 Worker 的硬件上下文内生效
    Applied,
    /// 插件不支持原地更新，调用方需回退到目标实例级 Worker 替换
    Unsupported,
    /// 目标实例槽不存在（尚未挂载或已被移除）
    NotFound,
    /// 插件拒绝或底层 FFI 失败，旧配置继续生效
    Rejected { reason: String },
}

/// 有效分析码流驱动泵
pub struct AnalysisPump {
    camera_id: String,
    cancel_token: CancellationToken,
    decode_handle: Option<tokio::task::JoinHandle<()>>,
    metrics: Arc<PumpMetrics>,
    control_slots: Arc<SharedControlSlots>,
    pipeline_mgr: Arc<PipelineManager>,
    command_tx: tokio::sync::mpsc::Sender<PumpCommand>,
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
                instance_id: LEGACY_SINGLE_WORKER_ID.to_string(),
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
                instance_id: LEGACY_SINGLE_WORKER_ID.to_string(),
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

        // 拓扑控制命令通道：控制面提交，解码循环在帧边界串行消费。
        let (command_tx, command_rx) =
            tokio::sync::mpsc::channel::<PumpCommand>(PUMP_COMMAND_CHANNEL_CAPACITY);

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
        for (cfg, (instance_id, handle, managed)) in instance_configs.into_iter().zip(workers) {
            let (parts, decode_slot) = spawn_instance_runtime(
                &cancel_token,
                &camera_id,
                &instance_id,
                &cfg.algorithm_id,
                &cfg.algorithm_type,
                handle,
                cfg.config_json,
                cfg.target_fps,
                pipeline_mgr.clone(),
                metrics.clone(),
            );
            decode_slot_vec.push(decode_slot);
            control_slot_vec.push(parts.into_control_slot(managed));
        }

        let slot_count = decode_slot_vec.len();
        let control_slots = Arc::new(SharedControlSlots::new(control_slot_vec));
        let mut decode_slots = decode_slot_vec;
        // 解码循环需要一份发送端存活保证（见 `_command_tx_guard`）。
        let command_tx_guard = command_tx.clone();

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
            // 自持一份发送端：控制面全部释放后仍能感知通道存活，避免 select 在 None 上空转。
            let _command_tx_guard = command_tx_guard;
            let mut command_rx = command_rx;
            let mut waiting_for_keyframe = true;

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
                        command = command_rx.recv() => {
                            match command {
                                Some(command) => {
                                    apply_pump_command(command, &mut decode_slots, &cam_id);
                                }
                                None => {
                                    tracing::warn!(camera_id = %cam_id, "分析泵控制命令通道已关闭，解码循环退出");
                                    break;
                                }
                            }
                            continue;
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
                        waiting_for_keyframe = true;
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

                if waiting_for_keyframe {
                    if pkt.is_keyframe {
                        waiting_for_keyframe = false;
                    } else {
                        continue;
                    }
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
                            let slot_generation_now = tracking_generation.load(Ordering::Acquire);
                            for slot in &mut decode_slots {
                                if slot.governor.should_sample(frame.timestamp) {
                                    slot.metrics.frames_sampled.fetch_add(1, Ordering::Relaxed);
                                    metrics_clone.frames_sampled.fetch_add(1, Ordering::Relaxed);

                                    // Drop-Oldest 单槽投递
                                    if let Ok(mut slot_guard) = slot.sampling_slot.frame.lock() {
                                        let slot_generation =
                                            slot.worker_generation.load(Ordering::Acquire);
                                        if slot_guard
                                            .replace(SampledFrame {
                                                frame: frame.clone(),
                                                generation: slot_generation_now,
                                                slot_generation,
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
                        tracing::warn!(camera_id = %cam_id, error = %e, "解码分析码流数据包失败，重置硬件解码器并等待下一关键帧对齐");
                        let _ = decoder.reset().await;
                        waiting_for_keyframe = true;
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
            pipeline_mgr,
            command_tx,
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

    /// 生成可脱离管线注册表锁长期持有的控制面句柄（crate 内部使用）
    pub(crate) fn control_handle(&self) -> PumpControlHandle {
        PumpControlHandle {
            camera_id: self.camera_id.clone(),
            cancel_token: self.cancel_token.clone(),
            metrics: self.metrics.clone(),
            control_slots: self.control_slots.clone(),
            pipeline_mgr: self.pipeline_mgr.clone(),
            command_tx: self.command_tx.clone(),
        }
    }

    /// 列出当前挂载的算法实例描述（instanceId / algorithmId / 生效配置）
    pub async fn instance_descriptors(&self) -> Vec<InstanceDescriptor> {
        self.control_handle().instance_descriptors().await
    }

    /// 按 `instanceId` 原地热更新目标实例配置。
    pub async fn update_instance_config(
        &self,
        instance_id: &str,
        config_json: &str,
    ) -> InstanceConfigUpdateOutcome {
        self.control_handle()
            .update_instance_config(instance_id, config_json)
            .await
    }

    /// 在帧边界更新目标实例的抽帧频率；不创建也不销毁 Worker。
    pub async fn set_instance_fps(&self, instance_id: &str, target_fps: u32) -> bool {
        self.control_handle()
            .set_instance_fps(instance_id, target_fps)
            .await
    }

    /// 向运行中的分析泵增量挂载一个算法实例。
    pub async fn add_instance(
        &self,
        config: WorkerInstanceConfig,
        worker: InferenceWorker,
    ) -> bool {
        self.control_handle().add_instance(config, worker).await
    }

    /// 从运行中的分析泵移除一个算法实例。
    pub async fn remove_instance(&self, instance_id: &str) -> bool {
        self.control_handle().remove_instance(instance_id).await
    }

    /// 替换目标实例的 Worker 并递增实例代际。
    pub async fn replace_instance_worker(
        &self,
        instance_id: &str,
        worker: InferenceWorker,
    ) -> bool {
        self.control_handle()
            .replace_instance_worker(instance_id, worker)
            .await
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
        new_worker: InferenceWorker,
    ) -> bool {
        let Some(instance_id) = self.resolve_instance_id_for_algorithm(algorithm_id).await else {
            tracing::warn!(
                camera_id = %self.camera_id,
                algorithm_id = %algorithm_id,
                "未找到目标算法实例槽，Worker 热替换失败"
            );
            shutdown_worker_blocking(new_worker).await;
            return false;
        };
        self.replace_instance_worker(&instance_id, new_worker).await
    }

    /// 在两帧间隙原子替换指定算法实例的推理 Worker 句柄
    pub async fn replace_worker(&self, algorithm_id: &str, new_worker: InferenceWorkerHandle) {
        let Some(instance_id) = self.resolve_instance_id_for_algorithm(algorithm_id).await else {
            tracing::warn!(
                camera_id = %self.camera_id,
                algorithm_id = %algorithm_id,
                "未找到目标算法实例槽，Worker 热替换失败"
            );
            return;
        };

        let (worker_holder, worker_generation) = {
            let guard = self.control_slots.inner.lock().await;
            let Some(slot) = guard.iter().find(|slot| slot.instance_id == instance_id) else {
                return;
            };
            (slot.worker_holder.clone(), slot.worker_generation.clone())
        };

        {
            let mut holder = worker_holder.write().await;
            *holder = new_worker;
        }
        worker_generation.fetch_add(1, Ordering::AcqRel);
    }

    /// 将算法 ID 解析为实例槽 ID（兼容单 Worker 与算法级热重载调用）
    async fn resolve_instance_id_for_algorithm(&self, algorithm_id: &str) -> Option<String> {
        let guard = self.control_slots.inner.lock().await;
        let matched: Vec<&ControlSlot> = guard
            .iter()
            .filter(|slot| slot.algorithm_id == algorithm_id)
            .collect();
        match matched.as_slice() {
            [slot] => Some(slot.instance_id.clone()),
            [] if algorithm_id == LEGACY_SINGLE_WORKER_ID && guard.len() == 1 => {
                Some(guard[0].instance_id.clone())
            }
            _ => None,
        }
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
        self.control_handle().instance_metrics().await
    }
}

impl Drop for AnalysisPump {
    fn drop(&mut self) {
        if !self.cancel_token.is_cancelled() {
            self.cancel_token.cancel();
        }
    }
}

/// 在专用阻塞线程关闭推理 Worker，避免硬件上下文与动态库析构阻塞 Tokio worker。
/// 返回 `false` 表示关停超时并已移入隔离池保活。
async fn shutdown_worker_blocking(mut worker: InferenceWorker) -> bool {
    let shutdown = tokio::task::spawn_blocking(move || worker.shutdown());
    match tokio::time::timeout(DEFAULT_PUMP_SHUTDOWN_TIMEOUT, shutdown).await {
        Ok(Ok(stopped)) => stopped,
        Ok(Err(error)) => {
            tracing::error!(error = %error, "推理 Worker 关停阻塞任务异常退出");
            false
        }
        Err(_) => {
            tracing::error!(
                timeout_ms = DEFAULT_PUMP_SHUTDOWN_TIMEOUT.as_millis() as u64,
                "推理 Worker 关停等待超时，已放弃等待并由隔离池保活"
            );
            false
        }
    }
}

/// 关停控制面槽位：停推理循环、回收 Worker 并释放该实例的硬件上下文。
async fn dispose_control_slot(mut slot: ControlSlot) -> bool {
    slot.cancel.cancel();
    if let Some(handle) = slot.infer_handle.take() {
        if tokio::time::timeout(DEFAULT_PUMP_SHUTDOWN_TIMEOUT, handle)
            .await
            .is_err()
        {
            tracing::error!(
                instance_id = %slot.instance_id,
                timeout_ms = DEFAULT_PUMP_SHUTDOWN_TIMEOUT.as_millis() as u64,
                "算法实例推理循环停止超时，已隔离后台句柄"
            );
        }
    }

    match slot.managed_worker.take() {
        Some(worker) => shutdown_worker_blocking(worker).await,
        None => true,
    }
}

/// 分析泵控制面句柄
///
/// 调用方按「克隆句柄 → 释放管线注册表锁 → 执行拓扑变更」的顺序使用本类型，
/// 避免在注册表读锁内执行 Worker 创建/关闭、FFI 或 `.await`。
#[derive(Clone)]
pub(crate) struct PumpControlHandle {
    camera_id: String,
    cancel_token: CancellationToken,
    metrics: Arc<PumpMetrics>,
    control_slots: Arc<SharedControlSlots>,
    pipeline_mgr: Arc<PipelineManager>,
    command_tx: tokio::sync::mpsc::Sender<PumpCommand>,
}

impl PumpControlHandle {
    /// 列出当前挂载的算法实例描述（instanceId / algorithmId / 抽帧频率 / 生效配置）
    pub async fn instance_descriptors(&self) -> Vec<InstanceDescriptor> {
        self.control_slots.instance_descriptors().await
    }

    /// 按 `instanceId` 原地热更新目标实例配置。
    pub async fn update_instance_config(
        &self,
        instance_id: &str,
        config_json: &str,
    ) -> InstanceConfigUpdateOutcome {
        self.control_slots
            .update_instance_config(instance_id, config_json)
            .await
    }

    /// 在帧边界更新目标实例的抽帧频率；不创建也不销毁 Worker。
    pub async fn set_instance_fps(&self, instance_id: &str, target_fps: u32) -> bool {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let command = PumpCommand::SetAnalysisFps {
            instance_id: instance_id.to_string(),
            target_fps,
            reply: reply_tx,
        };
        let applied = self.send_topology_command(command, reply_rx).await;
        if !applied {
            tracing::warn!(
                camera_id = %self.camera_id,
                instance_id = %instance_id,
                "分析泵抽帧频率变更未在帧边界生效"
            );
        }
        applied
    }

    /// 向运行中的分析泵增量挂载一个算法实例。
    ///
    /// 调用方必须已经完成 Worker 创建与资源准入；本方法只负责把实例接入解码抽帧
    /// 与推理循环，失败时不改变其他实例的运行时状态。
    pub async fn add_instance(
        &self,
        config: WorkerInstanceConfig,
        worker: InferenceWorker,
    ) -> bool {
        let handle = worker.handle();
        let instance_id = config.instance_id.clone();
        let (parts, decode_slot) = spawn_instance_runtime(
            &self.cancel_token,
            &self.camera_id,
            &config.instance_id,
            &config.algorithm_id,
            &config.algorithm_type,
            handle,
            config.config_json.clone(),
            config.target_fps,
            self.pipeline_mgr.clone(),
            self.metrics.clone(),
        );

        if let Err(rejected) = self
            .control_slots
            .push(parts.into_control_slot(Some(worker)))
            .await
        {
            tracing::warn!(
                camera_id = %self.camera_id,
                instance_id = %instance_id,
                "目标算法实例已存在于当前驱动泵，拒绝重复挂载"
            );
            dispose_control_slot(rejected).await;
            return false;
        }

        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let command = PumpCommand::AddInstance {
            slot: decode_slot,
            reply: reply_tx,
        };
        let added = self.send_topology_command(command, reply_rx).await;
        if added {
            tracing::info!(
                camera_id = %self.camera_id,
                instance_id = %instance_id,
                "算法实例已在帧边界增量挂载至分析泵"
            );
            true
        } else {
            tracing::error!(
                camera_id = %self.camera_id,
                instance_id = %instance_id,
                "算法实例抽帧槽挂载失败或超时，已回滚控制面槽位"
            );
            if let Some(slot) = self.control_slots.remove(&instance_id).await {
                dispose_control_slot(slot).await;
            }
            false
        }
    }

    /// 从运行中的分析泵移除一个算法实例，并归还 Worker 关停结果。
    ///
    /// `true` 表示抽帧槽已卸载且 Worker 已平稳回收；`false` 表示目标实例不存在、
    /// 控制命令未能送达，或 Worker 关停超时被隔离保活。
    pub async fn remove_instance(&self, instance_id: &str) -> bool {
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        let command = PumpCommand::RemoveInstance {
            instance_id: instance_id.to_string(),
            reply: reply_tx,
        };
        let slot_removed = self.send_topology_command(command, reply_rx).await;
        if !slot_removed {
            tracing::error!(
                camera_id = %self.camera_id,
                instance_id = %instance_id,
                "算法实例抽帧槽卸载未在帧边界确认，保留控制面槽位待重试"
            );
            return false;
        }

        let Some(slot) = self.control_slots.remove(instance_id).await else {
            return true;
        };
        let graceful = dispose_control_slot(slot).await;
        if !graceful {
            tracing::error!(
                camera_id = %self.camera_id,
                instance_id = %instance_id,
                "算法实例 Worker 关停超时，已隔离保活并停止发帧"
            );
        }
        graceful
    }

    /// 在解码帧边界安全下发并等待单条拓扑控制命令的响应
    async fn send_topology_command(
        &self,
        command: PumpCommand,
        reply_rx: tokio::sync::oneshot::Receiver<bool>,
    ) -> bool {
        if self.command_tx.send(command).await.is_err() {
            return false;
        }
        matches!(
            tokio::time::timeout(PUMP_COMMAND_ACK_TIMEOUT, reply_rx).await,
            Ok(Ok(true))
        )
    }

    /// 替换目标实例的 Worker 并递增实例代际。
    ///
    /// 先切换句柄再递增代际：宁可丢弃新 Worker 的首批结果，也不能让旧 Worker 的结果
    /// 被误认为新代际产出。
    pub async fn replace_instance_worker(
        &self,
        instance_id: &str,
        new_worker: InferenceWorker,
    ) -> bool {
        let replacement_handle = new_worker.handle();
        let (worker_holder, old_worker, worker_generation) = {
            let mut guard = self.control_slots.inner.lock().await;
            let Some(slot) = guard
                .iter_mut()
                .find(|slot| slot.instance_id == instance_id)
            else {
                drop(guard);
                tracing::warn!(
                    camera_id = %self.camera_id,
                    instance_id = %instance_id,
                    "未找到目标算法实例槽，Worker 增量替换失败"
                );
                shutdown_worker_blocking(new_worker).await;
                return false;
            };
            let old_worker = slot.managed_worker.replace(new_worker);
            (
                slot.worker_holder.clone(),
                old_worker,
                slot.worker_generation.clone(),
            )
        };

        {
            let mut holder = worker_holder.write().await;
            *holder = replacement_handle;
        }
        worker_generation.fetch_add(1, Ordering::AcqRel);

        if let Some(old_worker) = old_worker {
            shutdown_worker_blocking(old_worker).await;
        }
        true
    }

    /// 获取所有实例的运行监控指标快照
    pub async fn instance_metrics(&self) -> Vec<(String, Arc<InstanceMetrics>)> {
        self.control_slots.instance_metrics_snapshot().await
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
