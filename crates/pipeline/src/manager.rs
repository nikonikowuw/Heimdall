use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Mutex as TokioMutex;
use tokio::sync::RwLock as TokioRwLock;

use infer::{AlgoPackage, InferenceWorker, InferenceWorkerConfig};
use media::decoder::VideoDecoder;
use media::ring_buffer::{MainStreamRingBuffer, RingBufferConfig};
use media::{StreamClockAnchor, StreamItem, StreamSubscription};
use types::{
    AnalysisTask, BoundingBox, Camera, Detection, DetectionRule, EncodedPacket,
    EvidenceImageStream, FrameRef, TrackedObject,
};

use crate::capture_settle::{
    CandidateEvidence, CandidateRetainRequest, CaptureAction, CaptureSettleController,
    RecordCandidateOutcome,
};
use crate::decoded_ring::DecodedFrameRingBuffer;
use crate::error::PipelineError;
use crate::events::{
    PipelineAlarmEvent, PipelineAnalysisEvent, PipelineCaptureEvent,
    DEFAULT_ANALYSIS_EVENT_CHANNEL_CAPACITY,
};
use crate::pump::{AnalysisPump, AnalysisPumpConfig, MotionGateRuntimeConfig, PumpMetrics};
use crate::roi::RoiAffineMapper;
use crate::rules::{RuleEvaluator, TriggeredAlarm};
use crate::snapshot::{EvidenceTarget, SnapshotConfig, SnapshotEngine, SnapshotResult};
use crate::tracker::{ByteTrack, TrackUpdateStatus};

/// 跨流证据时标偏移超过该值时告警一次：说明此前的同轴比较会明显选错证据帧。
const CROSS_STREAM_OFFSET_WARN_MS: i64 = 40;

/// 一路摄像头的主码流/分析流接入时延锚点配对。
///
/// 成对承载，保证跨流换算不会读到「只更新了一半」的状态；告警去重标志也随本对一同
/// 重建，无需外部手工清位。
#[derive(Clone)]
pub(crate) struct StreamClockAnchors {
    /// 主码流证据流
    pub main: Arc<StreamClockAnchor>,
    /// 分析流（主码流分析模式下与 `main` 为同一实例）
    pub analysis: Arc<StreamClockAnchor>,
    /// 已就跨流证据时标偏移告警过一次，避免逐帧刷屏。
    warned_on_offset: Arc<AtomicBool>,
}

impl StreamClockAnchors {
    fn new(main: Arc<StreamClockAnchor>, analysis: Arc<StreamClockAnchor>) -> Self {
        Self {
            main,
            analysis,
            warned_on_offset: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// 取得结算控制器锁。
///
/// 控制器内部只有纯内存状态机（航迹表、时间戳），一次 panic 不会让状态失去一致性，
/// 因此锁中毒时沿用内部值，而不是把中毒传播成整路分析失败。
fn lock_capture_settle(
    ctx: &CameraPipelineContext,
) -> std::sync::MutexGuard<'_, CaptureSettleController> {
    ctx.capture_settle
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 单路摄像头管线运行时上下文
pub struct CameraPipelineContext {
    pub camera_id: String,
    /// 主码流高分辨率 NALU 内存环形队列 (2~3.5s GOP)
    pub ring_buffer: Arc<MainStreamRingBuffer>,
    /// 已解码原生高保真帧环形队列 (方案三：用于零解码瞬时直通抓拍与精确时标索引)
    pub decoded_ring: TokioRwLock<DecodedFrameRingBuffer>,
    /// 是否为主码流直接进行 AI 分析 (StreamMode::Main 模式)
    pub is_main_stream_analysis: AtomicBool,
    /// 最近一帧已解码候选帧（兼容旧的子码流保底字段名）
    pub sub_stream_fallback: TokioRwLock<Option<FrameRef>>,
    /// 是否有活跃的 AI 分析规则订阅
    pub ai_active: AtomicBool,
    /// 活跃的实时预览客户端计数
    pub preview_count: AtomicUsize,
    /// 最近一次主码流按需成功解码的高保真证据帧缓存（避免单帧多告警重复前向解码同一 GOP）
    pub last_on_demand_frame: TokioRwLock<Option<FrameRef>>,
    /// 主码流/分析流的接入时延锚点配对，供跨流 PTS 轴换算。
    ///
    /// 未注入或未标定时，跨流取证自动退回检测流自身的帧。
    pub(crate) stream_clock_anchors: TokioRwLock<Option<StreamClockAnchors>>,
    /// 主码流专用的按需快拍解码器实例 (惰性分配)
    pub snapshot_decoder: TokioMutex<Option<Box<dyn VideoDecoder + Send>>>,
    /// 纯 CPU ByteTrack 跟踪器 (兼容单算法入口)
    pub tracker: TokioMutex<ByteTrack>,
    /// 多算法独立 ByteTrack 跟踪器映射表 (algorithm_id -> ByteTrack)
    pub trackers: TokioMutex<HashMap<String, ByteTrack>>,
    /// 识别类通行抓拍结算状态机 (algorithm_id × track_id 维度, 纯同步计算)。
    ///
    /// 与 tracker 锁的固定获取顺序：先 `trackers` 后 `capture_settle`，禁止反向嵌套。
    pub capture_settle: std::sync::Mutex<CaptureSettleController>,
    /// 多算法 tracker 与清理操作之间的会话代际。
    pub(crate) tracking_generation: Arc<AtomicU64>,
    /// 每路摄像头按算法实例维护的最新活跃航迹快照 (algorithm_id -> Vec<TrackedObject>)
    pub current_tracks: TokioRwLock<HashMap<String, Vec<TrackedObject>>>,
    /// 局部特写预裁剪仿射变换映射器
    pub roi_mapper: TokioRwLock<RoiAffineMapper>,
    /// 任务级空间几何规则
    pub rules: Arc<TokioRwLock<Vec<DetectionRule>>>,
    /// 规则版本递增计数器，供解码泵做无锁变更探测
    pub rules_version: Arc<std::sync::atomic::AtomicU64>,
    /// 统一空间几何规则引擎
    pub rule_evaluator: RuleEvaluator,
}

impl std::fmt::Debug for CameraPipelineContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CameraPipelineContext")
            .field("camera_id", &self.camera_id)
            .field("ring_buffer", &self.ring_buffer)
            .field("ai_active", &self.ai_active.load(Ordering::Relaxed))
            .field("preview_count", &self.preview_count.load(Ordering::Relaxed))
            .finish()
    }
}

impl CameraPipelineContext {
    pub fn new(camera_id: impl Into<String>) -> Self {
        Self {
            camera_id: camera_id.into(),
            ring_buffer: Arc::new(MainStreamRingBuffer::new(RingBufferConfig::default())),
            decoded_ring: TokioRwLock::new(DecodedFrameRingBuffer::default()),
            is_main_stream_analysis: AtomicBool::new(false),
            sub_stream_fallback: TokioRwLock::new(None),
            ai_active: AtomicBool::new(false),
            preview_count: AtomicUsize::new(0),
            last_on_demand_frame: TokioRwLock::new(None),
            stream_clock_anchors: TokioRwLock::new(None),
            snapshot_decoder: TokioMutex::new(None),
            tracker: TokioMutex::new(ByteTrack::new()),
            trackers: TokioMutex::new(HashMap::new()),
            capture_settle: std::sync::Mutex::new(CaptureSettleController::new()),
            tracking_generation: Arc::new(AtomicU64::new(0)),
            current_tracks: TokioRwLock::new(HashMap::new()),
            roi_mapper: TokioRwLock::new(RoiAffineMapper::identity()),
            rules: Arc::new(TokioRwLock::new(Vec::new())),
            rules_version: Arc::new(std::sync::atomic::AtomicU64::new(1)),
            rule_evaluator: RuleEvaluator::new(),
        }
    }

    /// 当前是否需要保持硬件解码器运行 (有 AI 分析或实时预览推流)
    pub fn is_decoder_needed(&self) -> bool {
        self.ai_active.load(Ordering::Relaxed) || self.preview_count.load(Ordering::Relaxed) > 0
    }

    /// 若无活跃预览与 AI 任务，按需释放解码器会话与显存
    pub async fn release_decoder_if_idle(&self) {
        if !self.is_decoder_needed() {
            let mut dec_guard = self.snapshot_decoder.lock().await;
            if dec_guard.is_some() {
                *dec_guard = None;
                tracing::info!(
                    camera_id = %self.camera_id,
                    "摄像头无活跃预览与 AI 任务，按需进入 0% 负载静默状态，解码器显存已完全释放"
                );
            }
        }
    }
}

/// 默认全局允许的最大并发硬件抓拍解码器会话数 (针对 RK3588 / 昇腾 310B VPU 规格)
pub const DEFAULT_MAX_CONCURRENT_SNAPSHOT_DECODERS: usize = 4;

/// 全局抓拍解码借调超时阈值 (毫秒)
pub const DEFAULT_SNAPSHOT_PERMIT_TIMEOUT_MS: u64 = 100;

/// 全局多路视频分析与快照管线调度控制器
#[derive(Debug)]
pub struct PipelineManager {
    tasks: Arc<TokioRwLock<HashMap<String, AnalysisTask>>>,
    pipelines: Arc<TokioRwLock<HashMap<String, Arc<CameraPipelineContext>>>>,
    pumps: Arc<TokioRwLock<HashMap<String, AnalysisPump>>>,
    snapshot_engine: Arc<SnapshotEngine>,
    snapshot_semaphore: Arc<tokio::sync::Semaphore>,
    permit_timeout_ms: u64,
    analysis_event_tx: tokio::sync::broadcast::Sender<PipelineAnalysisEvent>,
    pending_alarm_events: Arc<Mutex<VecDeque<PipelineAlarmEvent>>>,
    dropped_pending_alarm_events: AtomicU64,
    pending_capture_events: Arc<Mutex<VecDeque<PipelineCaptureEvent>>>,
    dropped_pending_capture_events: AtomicU64,
}

impl Default for PipelineManager {
    fn default() -> Self {
        Self::new()
    }
}

/// 算法分析单帧处理结果
#[derive(Debug, Clone, Default)]
pub struct AnalysisOutcome {
    /// 本次输入是否真正推进了 tracker 和规则状态；迟到/拒绝结果为 false。
    pub applied: bool,
    /// 当前活跃航迹对象
    pub tracked: Vec<TrackedObject>,
    /// 触发的违规告警集合 (针对 detection 类防范算法)
    pub alarms: Vec<TriggeredAlarm>,
    /// 通行抓拍结算动作 (针对 recognition 类识别算法: 峰值候选留存 / 结算)
    pub capture_actions: Vec<CaptureAction>,
}

#[inline]
fn push_bounded<T>(
    queue: &Mutex<VecDeque<T>>,
    dropped_counter: &AtomicU64,
    item: T,
    item_desc: &str,
) {
    let mut pending = queue
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if pending.len() >= DEFAULT_ANALYSIS_EVENT_CHANNEL_CAPACITY {
        pending.pop_front();
        dropped_counter.fetch_add(1, Ordering::Relaxed);
        tracing::error!(
            capacity = DEFAULT_ANALYSIS_EVENT_CHANNEL_CAPACITY,
            "待持久化{item_desc}队列已满，丢弃最旧{item_desc}并记录溢出计数"
        );
    }
    pending.push_back(item);
}

#[inline]
fn drain_bounded<T>(queue: &Mutex<VecDeque<T>>, max_events: usize) -> Vec<T> {
    if max_events == 0 {
        return Vec::new();
    }
    let mut pending = queue
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let take = max_events.min(pending.len());
    pending.drain(..take).collect()
}

#[inline]
fn bounded_len<T>(queue: &Mutex<VecDeque<T>>) -> usize {
    queue
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .len()
}

/// 证据抓拍保底候选帧（消除 `(FrameRef, bool)` 基本类型偏执）
#[derive(Debug, Clone)]
pub(crate) struct FallbackCandidate {
    frame: FrameRef,
    is_sub_stream: bool,
}

impl FallbackCandidate {
    pub(crate) fn sub_stream(frame: FrameRef) -> Self {
        Self {
            frame,
            is_sub_stream: true,
        }
    }

    pub(crate) fn frame(&self) -> &FrameRef {
        &self.frame
    }

    pub(crate) fn is_sub_stream(&self) -> bool {
        self.is_sub_stream
    }

    pub(crate) fn into_frame(self) -> FrameRef {
        self.frame
    }
}

impl PipelineManager {
    pub fn new() -> Self {
        Self::with_evidence_dir(crate::DEFAULT_EVIDENCE_DIR)
    }

    pub fn with_evidence_dir(dir: impl Into<PathBuf>) -> Self {
        Self::with_evidence_dir_and_snapshot_config(dir, SnapshotConfig::default())
    }

    /// 使用自定义证据存储路径与快照抓拍配置构建管线管理器
    pub fn with_evidence_dir_and_snapshot_config(
        dir: impl Into<PathBuf>,
        config: SnapshotConfig,
    ) -> Self {
        Self::with_all_options(
            dir,
            config,
            DEFAULT_MAX_CONCURRENT_SNAPSHOT_DECODERS,
            DEFAULT_SNAPSHOT_PERMIT_TIMEOUT_MS,
        )
    }

    /// 全功能参数构造管线管理器 (支持自定义并发 VPU 通道上限与超时阈值)
    pub fn with_all_options(
        dir: impl Into<PathBuf>,
        config: SnapshotConfig,
        max_concurrent_decoders: usize,
        permit_timeout_ms: u64,
    ) -> Self {
        let (analysis_event_tx, _) =
            tokio::sync::broadcast::channel(DEFAULT_ANALYSIS_EVENT_CHANNEL_CAPACITY);
        Self {
            tasks: Arc::new(TokioRwLock::new(HashMap::new())),
            pipelines: Arc::new(TokioRwLock::new(HashMap::new())),
            pumps: Arc::new(TokioRwLock::new(HashMap::new())),
            snapshot_engine: Arc::new(SnapshotEngine::with_config(dir, config)),
            snapshot_semaphore: Arc::new(tokio::sync::Semaphore::new(
                max_concurrent_decoders.max(1),
            )),
            permit_timeout_ms,
            analysis_event_tx,
            pending_alarm_events: Arc::new(Mutex::new(VecDeque::with_capacity(
                DEFAULT_ANALYSIS_EVENT_CHANNEL_CAPACITY,
            ))),
            dropped_pending_alarm_events: AtomicU64::new(0),
            pending_capture_events: Arc::new(Mutex::new(VecDeque::with_capacity(
                DEFAULT_ANALYSIS_EVENT_CHANNEL_CAPACITY,
            ))),
            dropped_pending_capture_events: AtomicU64::new(0),
        }
    }

    /// 获取快照抓拍引擎句柄
    pub fn snapshot_engine(&self) -> &SnapshotEngine {
        &self.snapshot_engine
    }

    /// 获取全局 VPU 快照信号量句柄 (供测试与外部配额状态观察)
    pub fn snapshot_semaphore(&self) -> &tokio::sync::Semaphore {
        &self.snapshot_semaphore
    }

    /// 获取当前快照配置拷贝
    pub fn snapshot_config(&self) -> SnapshotConfig {
        self.snapshot_engine.config()
    }

    /// 热更新快照配置
    pub fn update_snapshot_config(&self, new_config: SnapshotConfig) -> Result<(), String> {
        self.snapshot_engine.update_config(new_config)
    }

    /// 订阅管线统一分析事件（实时航迹与规则告警）
    pub fn subscribe_analysis_events(
        &self,
    ) -> tokio::sync::broadcast::Receiver<PipelineAnalysisEvent> {
        self.analysis_event_tx.subscribe()
    }

    /// 向管线分析事件通道发布事件。
    ///
    /// Tracks 走实时 broadcast；Alarm 与 Capture 额外进入有界内存补偿缓冲（有界 1024 槽位），
    /// 仅用于在下游持久化 Worker 启动前或广播 Lagged 时提供无损补偿读取，不包含任何数据库或外部持久化逻辑。
    pub fn publish_analysis_event(&self, event: PipelineAnalysisEvent) {
        match &event {
            PipelineAnalysisEvent::Alarm(alarm) => {
                push_bounded(
                    &self.pending_alarm_events,
                    &self.dropped_pending_alarm_events,
                    (**alarm).clone(),
                    "告警",
                );
            }
            PipelineAnalysisEvent::Capture(capture) => {
                push_bounded(
                    &self.pending_capture_events,
                    &self.dropped_pending_capture_events,
                    (**capture).clone(),
                    "抓拍",
                );
            }
            PipelineAnalysisEvent::Tracks(_) | PipelineAnalysisEvent::Telemetry(_) => {}
        }

        let _ = self.analysis_event_tx.send(event);
    }

    /// 取出待持久化告警。返回值有界，调用方应在独立 Worker 中处理。
    pub fn drain_pending_alarm_events(&self, max_events: usize) -> Vec<PipelineAlarmEvent> {
        drain_bounded(&self.pending_alarm_events, max_events)
    }

    /// 当前待持久化告警数量。
    pub fn pending_alarm_event_count(&self) -> usize {
        bounded_len(&self.pending_alarm_events)
    }

    /// 累计因待持久化队列满而淘汰的告警数量。
    pub fn dropped_pending_alarm_event_count(&self) -> u64 {
        self.dropped_pending_alarm_events.load(Ordering::Relaxed)
    }

    /// 取出待持久化客观通行抓拍。返回值有界，调用方应在独立 Capture Worker 中处理。
    pub fn drain_pending_capture_events(&self, max_events: usize) -> Vec<PipelineCaptureEvent> {
        drain_bounded(&self.pending_capture_events, max_events)
    }

    /// 当前待持久化通行抓拍数量。
    pub fn pending_capture_event_count(&self) -> usize {
        bounded_len(&self.pending_capture_events)
    }

    /// 累计因待持久化队列满而淘汰的通行抓拍数量。
    pub fn dropped_pending_capture_event_count(&self) -> u64 {
        self.dropped_pending_capture_events.load(Ordering::Relaxed)
    }

    /// 注册或获取某路摄像头的分析管线上下文
    pub async fn get_or_create_context(&self, camera_id: &str) -> Arc<CameraPipelineContext> {
        if let Some(ctx) = self.get_pipeline_context(camera_id).await {
            return ctx;
        }

        let mut pipelines = self.pipelines.write().await;
        pipelines
            .entry(camera_id.to_string())
            .or_insert_with(|| Arc::new(CameraPipelineContext::new(camera_id)))
            .clone()
    }

    /// 获取某路摄像头的分析管线上下文（若不存在返回 None）
    pub async fn get_pipeline_context(
        &self,
        camera_id: &str,
    ) -> Option<Arc<CameraPipelineContext>> {
        let pipelines = self.pipelines.read().await;
        pipelines.get(camera_id).cloned()
    }

    /// 在没有任务、pump、AI 或预览引用时移除摄像头上下文。
    pub async fn remove_pipeline_context_if_idle(&self, camera_id: &str) -> bool {
        // 固定锁顺序为 tasks -> pumps -> pipelines，且持有只读守卫直到删除完成，
        // 防止检查与删除之间并发挂载新任务或 pump。
        let tasks = self.tasks.read().await;
        if tasks.contains_key(camera_id) {
            return false;
        }
        let pumps = self.pumps.read().await;
        if pumps.contains_key(camera_id) {
            return false;
        }

        let mut pipelines = self.pipelines.write().await;
        let is_idle = pipelines
            .get(camera_id)
            .map(|ctx| !ctx.is_decoder_needed())
            .unwrap_or(false);
        if !is_idle {
            return false;
        }

        pipelines.remove(camera_id).is_some()
    }

    /// 向摄像机主码流环形队列压入压缩 NALU 包（严格仅接收视频包，忽略音频）
    pub async fn push_main_packet(&self, camera_id: &str, packet: Arc<EncodedPacket>) {
        if !packet.codec.is_video() || packet.stream_tag == types::StreamTag::Audio {
            return;
        }
        let ctx = self.get_or_create_context(camera_id).await;
        ctx.ring_buffer.push(packet);
    }

    /// 更新最新解码帧 (统一用于保底快照与方案三零解码直通缓存)
    pub async fn update_decoded_frame(&self, camera_id: &str, frame: FrameRef) {
        let ctx = self.get_or_create_context(camera_id).await;
        {
            let mut ring = ctx.decoded_ring.write().await;
            ring.push(frame.clone());
        }
        {
            let mut fallback = ctx.sub_stream_fallback.write().await;
            *fallback = Some(frame);
        }
    }

    /// 兼容旧命名接口：更新子码流最新解码帧
    #[inline]
    pub async fn update_sub_stream_frame(&self, camera_id: &str, frame: FrameRef) {
        self.update_decoded_frame(camera_id, frame).await;
    }

    /// 上报运动门控热度与状态遥测数据 (只在有预览客户端时广播)
    pub async fn report_motion_telemetry(
        &self,
        camera_id: &str,
        timestamp: i64,
        motion_score: f32,
        is_motion_gated: bool,
    ) {
        if !self.has_preview_subscribers(camera_id).await {
            return;
        }

        let event = types::CameraTelemetryEvent {
            camera_id: camera_id.to_string(),
            timestamp,
            active_tracks: 0,
            person_count: 0,
            car_count: 0,
            motion_score: motion_score.clamp(0.0, 1.0),
            is_motion_gated,
        };
        self.publish_analysis_event(PipelineAnalysisEvent::Telemetry(Box::new(event)));
    }

    /// 清空某路摄像头的已解码帧环形队列并释放显存/DMA-BUF 租约
    pub async fn clear_decoded_ring(&self, camera_id: &str) {
        if let Some(ctx) = self.get_pipeline_context(camera_id).await {
            {
                let mut ring = ctx.decoded_ring.write().await;
                ring.clear();
            }
            {
                let mut fallback = ctx.sub_stream_fallback.write().await;
                *fallback = None;
            }
        }
    }

    /// 清理一条摄像头会话的全部宿主航迹，并向订阅方发送空快照。
    pub async fn clear_tracking(&self, camera_id: &str) {
        let Some(ctx) = self.get_pipeline_context(camera_id).await else {
            return;
        };

        // 偶数代际可写，奇数代际表示清理进行中。CAS 失败说明其他清理已接管。
        let generation = ctx.tracking_generation.load(Ordering::Acquire);
        if generation & 1 != 0
            || ctx
                .tracking_generation
                .compare_exchange(
                    generation,
                    generation + 1,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_err()
        {
            return;
        }
        let algorithm_ids: Vec<String> = {
            let mut trackers = ctx.trackers.lock().await;
            for tracker in trackers.values_mut() {
                tracker.clear();
            }
            ctx.tracker.lock().await.clear();
            {
                let mut settle = lock_capture_settle(&ctx);
                settle.clear();
            }
            let mut current = ctx.current_tracks.write().await;
            current.drain().map(|(id, _)| id).collect()
        };
        ctx.tracking_generation.fetch_add(1, Ordering::Release);

        let timestamp = chrono::Utc::now().timestamp_millis();
        for algorithm_id in algorithm_ids {
            self.publish_analysis_event(PipelineAnalysisEvent::Tracks(
                crate::events::PipelineTrackEvent {
                    camera_id: camera_id.to_string(),
                    algorithm_id,
                    timestamp,
                    tracks: Vec::new(),
                },
            ));
        }
    }

    /// 在连续推理失败时按源帧 PTS 清除已经超过 Lost 保留时长的算法航迹。
    pub async fn expire_tracking_for_algo_at(
        &self,
        camera_id: &str,
        algorithm_id: &str,
        timestamp_ms: i64,
    ) -> bool {
        let Some(ctx) = self.get_pipeline_context(camera_id).await else {
            return false;
        };
        let generation = ctx.tracking_generation.load(Ordering::Acquire);
        if generation & 1 != 0 {
            return false;
        }
        let expired = {
            let mut trackers = ctx.trackers.lock().await;
            if generation != ctx.tracking_generation.load(Ordering::Acquire) {
                false
            } else {
                let expired = trackers
                    .get_mut(algorithm_id)
                    .is_some_and(|tracker| tracker.expire_if_stale_at(timestamp_ms));
                if expired {
                    lock_capture_settle(&ctx).clear_algorithm(algorithm_id);
                }
                expired
            }
        };
        if !expired || generation != ctx.tracking_generation.load(Ordering::Acquire) {
            return false;
        }

        let removed = {
            let mut current = ctx.current_tracks.write().await;
            if generation != ctx.tracking_generation.load(Ordering::Acquire) {
                return false;
            }
            current.remove(algorithm_id).is_some()
        };
        if removed {
            self.publish_analysis_event(PipelineAnalysisEvent::Tracks(
                crate::events::PipelineTrackEvent {
                    camera_id: camera_id.to_string(),
                    algorithm_id: algorithm_id.to_string(),
                    timestamp: timestamp_ms,
                    tracks: Vec::new(),
                },
            ));
        }
        removed
    }

    /// 留存峰值候选证据（`pump.rs` 分析循环消费 `CaptureAction::RetainCandidate`）。
    ///
    /// 复用当帧 `analyzed_frame` 编码为内存字节并登记到结算控制器；超预算或轨道已结算时
    /// 丢弃本次产物。推理帧与请求时间戳不一致时拒绝编码。
    pub async fn retain_capture_candidate(
        &self,
        camera_id: &str,
        algorithm_id: &str,
        request: &CandidateRetainRequest,
        analyzed_frame: FrameRef,
    ) -> Result<RecordCandidateOutcome, PipelineError> {
        let geometry = request.geometry;
        if analyzed_frame.camera_id != camera_id || analyzed_frame.timestamp != geometry.pts_ms {
            return Err(PipelineError::Snapshot(format!(
                "候选帧与请求不一致 (camera={}, framePts={}, reqPts={})",
                analyzed_frame.camera_id, analyzed_frame.timestamp, geometry.pts_ms
            )));
        }
        let Some(ctx) = self.get_pipeline_context(camera_id).await else {
            return Err(PipelineError::PipelineNotFound {
                camera_id: camera_id.to_string(),
            });
        };
        // 冻结编码时刻的码流来源：结算可能在一个结算窗口之后才发生，
        // 届时重新采样 `is_main_stream_analysis` 可能已换流，记录来源就会失真。
        let stream = if ctx.is_main_stream_analysis.load(Ordering::Acquire) {
            EvidenceImageStream::Main
        } else {
            EvidenceImageStream::Sub
        };
        let crop_bbox = geometry.face_bbox.unwrap_or(geometry.bbox);
        let encoded = self
            .snapshot_engine
            .encode_candidate_async(camera_id, analyzed_frame, Some(crop_bbox), stream)
            .await?;
        let evidence = CandidateEvidence {
            full_jpeg: encoded.full_jpeg.into(),
            crop_jpeg: encoded.crop_jpeg.into(),
            width: encoded.width,
            height: encoded.height,
            geometry,
            stream,
        };
        let outcome = {
            let mut settle = lock_capture_settle(&ctx);
            settle.record_candidate(algorithm_id, request.track_id, evidence)
        };
        match outcome {
            RecordCandidateOutcome::Accepted => {}
            RecordCandidateOutcome::Dismissed => {
                tracing::debug!(
                    camera_id,
                    algorithm_id,
                    track_id = request.track_id,
                    "候选对应轨道已结算，本次编码产物已丢弃"
                );
            }
            RecordCandidateOutcome::RejectedBudget => {
                let (pending_bytes, rejections) = {
                    let settle = lock_capture_settle(&ctx);
                    (settle.pending_bytes(), settle.budget_rejections())
                };
                tracing::warn!(
                    camera_id,
                    algorithm_id,
                    track_id = request.track_id,
                    pending_bytes,
                    rejections,
                    "候选字节预算已满，本次候选丢弃（结算将回退当帧快照）"
                );
            }
        }
        Ok(outcome)
    }

    /// 将结算携带的内存候选一次性写入正式证据目录（唯一一次落盘）。
    ///
    /// 写盘走 `SnapshotEngine` 的专用编码线程队列，这是全进程唯一允许碰平台编码器/
    /// 写证据文件的线程；不占用 Tokio worker，也不另开 `spawn_blocking`。
    /// 第二个文件失败时由线程内的 `write_candidate_evidence` 回滚首个文件。
    pub async fn write_capture_candidate(
        &self,
        camera_id: &str,
        evidence: &CandidateEvidence,
    ) -> Result<SnapshotResult, PipelineError> {
        self.snapshot_engine
            .write_candidate_async(camera_id, evidence.clone())
            .await
    }

    /// 标记该路摄像头是否为主码流直接进行 AI 分析 (StreamMode::Main 模式)
    pub async fn set_main_stream_analysis(&self, camera_id: &str, is_main: bool) {
        let ctx = self.get_or_create_context(camera_id).await;
        ctx.is_main_stream_analysis
            .store(is_main, Ordering::Release);
        tracing::info!(
            camera_id = %camera_id,
            is_main,
            "摄像头主码流分析模式状态已更新"
        );
    }

    /// 注入主码流与分析流的接入时延锚点，用于跨流 PTS 轴换算。
    ///
    /// 每次启动管线都必须重新注入；主码流分析模式下两路为同一物理连接，
    /// 应传入同一 `Arc`，此时换算恒等（偏移 0），无需标定即可成立。
    pub async fn set_stream_clock_anchors(
        &self,
        camera_id: &str,
        main: Arc<StreamClockAnchor>,
        analysis: Arc<StreamClockAnchor>,
    ) {
        let ctx = self.get_or_create_context(camera_id).await;
        *ctx.stream_clock_anchors.write().await = Some(StreamClockAnchors::new(main, analysis));
    }

    /// 清除接入时延锚点，使跨流取证退回安全路径。
    ///
    /// 不创建上下文：停止路径上上下文可能已经不存在。
    pub async fn clear_stream_clock_anchors(&self, camera_id: &str) {
        if let Some(ctx) = self.get_pipeline_context(camera_id).await {
            *ctx.stream_clock_anchors.write().await = None;
        }
    }

    /// 把检测流的时标换算到主码流证据轴。
    ///
    /// 返回 `None` 表示跨流时延尚未标定或锚点未注入：调用方必须退回检测流自身的帧，
    /// **不得**假定偏移为 0，否则等于继续在错位的轴上做比较，重新引入错帧证据。
    pub(crate) async fn resolve_main_axis_pts(
        &self,
        ctx: &CameraPipelineContext,
        camera_id: &str,
        detection_pts_ms: i64,
    ) -> Option<i64> {
        let Some(anchors) = ctx.stream_clock_anchors.read().await.clone() else {
            tracing::debug!(
                camera_id = %camera_id,
                detection_pts = detection_pts_ms,
                "跨流时标锚点未注入，本次取证退回检测流自身帧"
            );
            return None;
        };

        let Some(main_axis_pts) = anchors
            .analysis
            .convert_pts_to(&anchors.main, detection_pts_ms)
        else {
            tracing::debug!(
                camera_id = %camera_id,
                detection_pts = detection_pts_ms,
                main_samples = anchors.main.sample_count(),
                analysis_samples = anchors.analysis.sample_count(),
                "跨流接入时延尚未标定，本次取证退回检测流自身帧（不检索主码流证据环）"
            );
            return None;
        };

        let offset_ms = main_axis_pts - detection_pts_ms;
        let main_lag_ms = anchors.main.lag_ms();
        let analysis_lag_ms = anchors.analysis.lag_ms();
        tracing::debug!(
            camera_id = %camera_id,
            detection_pts = detection_pts_ms,
            main_axis_pts,
            offset_ms,
            ?main_lag_ms,
            ?analysis_lag_ms,
            "跨流证据时标已换算至主码流轴"
        );

        if offset_ms.abs() > CROSS_STREAM_OFFSET_WARN_MS
            && !anchors.warned_on_offset.swap(true, Ordering::Relaxed)
        {
            tracing::warn!(
                camera_id = %camera_id,
                offset_ms,
                ?main_lag_ms,
                ?analysis_lag_ms,
                "主码流与子码流 PTS 轴存在显著偏移，此前按同轴比较取回的证据帧会与推理帧错位"
            );
        }

        Some(main_axis_pts)
    }

    /// 标记 AI 分析激活状态（按需解码开关）
    pub async fn set_ai_active(&self, camera_id: &str, active: bool) {
        let ctx = self.get_or_create_context(camera_id).await;
        ctx.ai_active.store(active, Ordering::Relaxed);
        tracing::info!(
            camera_id = %camera_id,
            active,
            is_decoder_needed = ctx.is_decoder_needed(),
            "摄像头 AI 活跃状态已切换"
        );
    }

    /// 增加实时预览推流计数
    pub async fn increment_preview(&self, camera_id: &str) {
        let ctx = self.get_or_create_context(camera_id).await;
        ctx.preview_count.fetch_add(1, Ordering::Relaxed);
    }

    /// 减少实时预览推流计数
    pub async fn decrement_preview(&self, camera_id: &str) {
        let pipelines = self.pipelines.read().await;
        if let Some(ctx) = pipelines.get(camera_id) {
            let _ = ctx
                .preview_count
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |cnt| {
                    Some(cnt.saturating_sub(1))
                });
            ctx.release_decoder_if_idle().await;
        }
    }

    /// 查询指定摄像头当前是否有活跃的实时预览客户端 (用于按需节流与零开销静默)
    pub async fn has_preview_subscribers(&self, camera_id: &str) -> bool {
        self.pipelines
            .read()
            .await
            .get(camera_id)
            .is_some_and(|ctx| ctx.preview_count.load(Ordering::Relaxed) > 0)
    }

    /// 获取指定摄像头当前活跃的所有算法实例航迹快照
    pub async fn get_current_tracks(&self, camera_id: &str) -> Vec<TrackedObject> {
        let pipelines = self.pipelines.read().await;
        if let Some(ctx) = pipelines.get(camera_id) {
            let tracks_map = ctx.current_tracks.read().await;
            tracks_map.values().flatten().cloned().collect()
        } else {
            Vec::new()
        }
    }

    /// 挂载主码流消费者，持续将压缩 NALU 包压入 RingBuffer
    pub fn attach_main_stream(
        &self,
        camera_id: &str,
        subscription: StreamSubscription,
    ) -> tokio::task::JoinHandle<()> {
        let pipelines = self.pipelines.clone();
        let camera_id = camera_id.to_string();

        tokio::spawn(async move {
            let ctx = {
                let mut p = pipelines.write().await;
                p.entry(camera_id.clone())
                    .or_insert_with(|| Arc::new(CameraPipelineContext::new(camera_id.clone())))
                    .clone()
            };

            let mut awaiting_keyframe = false;
            loop {
                let Some(item) = subscription.recv().await else {
                    tracing::warn!(camera_id = %camera_id, "主码流消费者 mailbox 已关闭，RingBuffer attach 退出");
                    break;
                };
                match item {
                    StreamItem::Packet(pkt) => {
                        // 严格仅接收视频包，忽略音频包与非视频数据
                        if !pkt.codec.is_video() || pkt.stream_tag == types::StreamTag::Audio {
                            continue;
                        }
                        if awaiting_keyframe && !pkt.is_keyframe {
                            continue;
                        }
                        awaiting_keyframe = false;
                        ctx.ring_buffer.push(pkt);
                    }
                    StreamItem::Replay(snapshot) => {
                        ctx.ring_buffer.clear();
                        awaiting_keyframe = false;
                        for packet in snapshot.packets.iter().cloned() {
                            if packet.codec.is_video()
                                && packet.stream_tag != types::StreamTag::Audio
                            {
                                ctx.ring_buffer.push(packet);
                            }
                        }
                    }
                    StreamItem::SourceReset { epoch } => {
                        ctx.ring_buffer.clear();
                        awaiting_keyframe = true;
                        tracing::info!(camera_id = %camera_id, epoch, "主码流源流 epoch 重建，清空残缺 GOP");
                    }
                }
            }
        })
    }

    /// 触发靶向快拍抽帧与证据图片落地。
    ///
    /// `target_pts_ms` 位于**检测流（分析流）轴**；子码流分析模式下函数内部会先把它换算到
    /// 主码流证据轴，再检索主码流证据环。
    pub async fn trigger_snapshot(
        &self,
        camera_id: &str,
        target_pts_ms: i64,
        bbox: Option<BoundingBox>,
    ) -> Result<SnapshotResult, PipelineError> {
        self.trigger_snapshot_internal(camera_id, target_pts_ms, bbox, None)
            .await
    }

    /// 使用推理实际消费的原生帧生成证据。
    ///
    /// 主码流分析时这条路径绕过 `decoded_ring` 和按需 GOP 解码，保证检测框与落盘图片
    /// 共享同一个 `FrameRef`。子码流分析仍将该帧作为同刻保底，优先尝试主码流 GOP 高清取证。
    ///
    /// `target_pts_ms` 位于**检测流（分析流）轴**且必须与 `analyzed_frame.timestamp` 一致，
    /// 否则直接拒绝抓拍。
    pub async fn trigger_snapshot_for_frame(
        &self,
        camera_id: &str,
        target_pts_ms: i64,
        bbox: Option<BoundingBox>,
        analyzed_frame: FrameRef,
    ) -> Result<SnapshotResult, PipelineError> {
        self.trigger_snapshot_internal(camera_id, target_pts_ms, bbox, Some(analyzed_frame))
            .await
    }

    async fn trigger_snapshot_internal(
        &self,
        camera_id: &str,
        detection_pts_ms: i64,
        bbox: Option<BoundingBox>,
        analyzed_frame: Option<FrameRef>,
    ) -> Result<SnapshotResult, PipelineError> {
        let ctx = self.get_or_create_context(camera_id).await;
        let is_main_stream = ctx.is_main_stream_analysis.load(Ordering::Acquire);

        // 推理框只能与产生该框的源帧配对。调用方应从同一帧派生 timestamp；若边界数据
        // 不一致，拒绝抓拍比保存一张看似成功但时空错配的证据更安全。
        let analyzed_fallback = if let Some(frame) = analyzed_frame {
            if frame.camera_id != camera_id || frame.timestamp != detection_pts_ms {
                return Err(PipelineError::Snapshot(format!(
                    "推理帧与抓拍目标不一致 (camera={}, framePts={}, targetPts={})",
                    frame.camera_id, frame.timestamp, detection_pts_ms
                )));
            }

            if is_main_stream {
                tracing::debug!(
                    camera_id = %camera_id,
                    detection_pts = detection_pts_ms,
                    frame_pts = frame.timestamp,
                    width = frame.width,
                    height = frame.height,
                    "使用推理实际消费的主码流 FrameRef 生成同帧证据"
                );
                return self
                    .snapshot_engine
                    .save_snapshot_async(camera_id, frame, bbox, EvidenceImageStream::Main)
                    .await;
            }
            Some(FallbackCandidate::sub_stream(frame))
        } else {
            None
        };

        // 没有显式携带推理帧时，仍可复用环中 PTS 对齐的主流帧。主码流分析模式不维护
        // 压缩包证据环，未命中即失败，绝不退化成按需 GOP 追帧。
        if is_main_stream {
            let matched_opt = {
                let ring = ctx.decoded_ring.read().await;
                ring.find_by_pts(
                    detection_pts_ms,
                    ring.window_duration_ms()
                        .min(crate::snapshot::SnapshotEngine::MAX_TARGET_FRAME_DIFF_MS),
                )
            };

            if let Some((frame, diff_ms)) = matched_opt {
                tracing::debug!(
                    camera_id = %camera_id,
                    detection_pts = detection_pts_ms,
                    frame_pts = frame.timestamp,
                    diff_ms,
                    width = frame.width,
                    height = frame.height,
                    "零解码直通命中，复用主码流常驻解码高保真帧"
                );
                return self
                    .snapshot_engine
                    .save_snapshot_async(camera_id, frame, bbox, EvidenceImageStream::Main)
                    .await;
            }

            tracing::warn!(
                camera_id = %camera_id,
                detection_pts = detection_pts_ms,
                "主码流分析模式未命中同刻已解码帧，拒绝按需 GOP 追帧取证"
            );
            return Err(PipelineError::PipelineNotFound {
                camera_id: format!("{camera_id} (主码流分析模式未命中同刻已解码帧)"),
            });
        }

        // 以下仅子码流双流分析路径可达：主码流已在上面收敛返回。
        //
        // 检测帧的 PTS 位于子码流轴，而主码流证据环与按需解码帧位于主码流轴；两条轴的原点由
        // 各自的 RTSP PLAY 应答决定，**不可直接比较**。必须先换算到主码流轴再检索/比较，
        // 否则取回的证据帧与推理帧不是同一时刻；未标定时换算返回 None，直接退回检测流自身的帧。
        let target = EvidenceTarget {
            detection_pts_ms,
            main_axis_pts_ms: self
                .resolve_main_axis_pts(&ctx, camera_id, detection_pts_ms)
                .await,
        };

        // 优先复用本推理周期中同一时标已按需解码成功的主码流高分辨率帧，
        // 彻底避免单帧触发多规则告警或多目标通行抓拍时对同一 GOP 重复执行高开销前向硬解！
        // 缓存帧位于主码流轴，必须与换算后的目标时标同轴比较。
        let cached_on_demand = ctx.last_on_demand_frame.read().await.clone();
        if let (Some(main_axis_pts), Some(cached)) = (target.main_axis_pts_ms, cached_on_demand) {
            let diff_ms = cached.timestamp.abs_diff(main_axis_pts);
            if diff_ms <= crate::snapshot::SnapshotEngine::MAX_TARGET_FRAME_DIFF_MS as u64 {
                tracing::debug!(
                    camera_id = %camera_id,
                    detection_pts = detection_pts_ms,
                    main_axis_pts,
                    frame_pts = cached.timestamp,
                    diff_ms,
                    "复用同刻已解码的主码流按需帧，避免多告警重复前向解码 GOP"
                );
                return self
                    .snapshot_engine
                    .save_snapshot_async(camera_id, cached, bbox, EvidenceImageStream::Main)
                    .await;
            }
        }

        // 显式传入的推理帧是最可靠的保底；没有它才查询兼容环。
        // 所有候选帧均严格受 MAX_TARGET_FRAME_DIFF_MS 约束，杜绝过期帧与当前 bbox 拼接。
        let fallback_frame: Option<FallbackCandidate> = if let Some(frame) = analyzed_fallback {
            Some(frame)
        } else {
            let max_diff = crate::snapshot::SnapshotEngine::MAX_TARGET_FRAME_DIFF_MS;
            let ring_fallback = {
                let ring = ctx.decoded_ring.read().await;
                ring.find_by_pts(detection_pts_ms, max_diff)
                    .map(|(f, _)| FallbackCandidate::sub_stream(f))
            };
            match ring_fallback {
                Some(f) => Some(f),
                None => {
                    let guard = ctx.sub_stream_fallback.read().await;
                    guard
                        .as_ref()
                        .filter(|f| f.timestamp.abs_diff(detection_pts_ms) <= max_diff as u64)
                        .cloned()
                        .map(FallbackCandidate::sub_stream)
                }
            }
        };

        // 工业级全局 VPU 抓拍通道配额管控：仅在需要按码流取证时借用按需解码器。
        let permit_res = tokio::time::timeout(
            std::time::Duration::from_millis(self.permit_timeout_ms),
            self.snapshot_semaphore.acquire(),
        )
        .await;

        let (frame_to_process, is_sub_stream_frame) = match permit_res {
            Ok(Ok(permit)) => {
                let res = {
                    let mut decoder_guard = ctx.snapshot_decoder.lock().await;
                    if decoder_guard.is_none()
                        && target.main_axis_pts_ms.is_some()
                        && !ctx.ring_buffer.is_empty()
                    {
                        let codec = ctx
                            .ring_buffer
                            .latest_codec()
                            .unwrap_or(types::CodecType::H264);
                        *decoder_guard = Some(media::create_decoder(camera_id, codec));
                    }

                    self.snapshot_engine
                        .decode_frame(
                            camera_id,
                            target,
                            Some(&ctx.ring_buffer),
                            fallback_frame.as_ref().map(|f| f.frame()),
                            decoder_guard.as_deref_mut(),
                        )
                        .await?
                };
                drop(permit);
                let (frame, used_fallback) = res;
                let is_sub_fallback = if used_fallback {
                    fallback_frame
                        .as_ref()
                        .map(|f| f.is_sub_stream())
                        .unwrap_or(true)
                } else {
                    // 成功按需解码出主码流高分辨率帧，缓存供同刻后续多告警/多抓拍零解码直通复用
                    *ctx.last_on_demand_frame.write().await = Some(frame.clone());
                    false
                };
                (frame, is_sub_fallback)
            }
            _ => {
                // 配额满载时只允许复用经过 PTS 严格校验的候选帧；主流无候选则失败。
                if let Some(fallback_src) = fallback_frame {
                    let is_sub_stream = fallback_src.is_sub_stream();
                    let fallback = fallback_src.into_frame();
                    let diff_ms = fallback.timestamp.abs_diff(detection_pts_ms);
                    if diff_ms <= crate::snapshot::SnapshotEngine::MAX_TARGET_FRAME_DIFF_MS as u64 {
                        tracing::warn!(
                            camera_id = %camera_id,
                            detection_pts = detection_pts_ms,
                            frame_pts = fallback.timestamp,
                            diff_ms,
                            timeout_ms = self.permit_timeout_ms,
                            "全局 VPU 抓拍配额满载，复用同刻候选帧"
                        );
                        (fallback, is_sub_stream)
                    } else {
                        return Err(PipelineError::Snapshot(format!(
                            "全局 VPU 抓拍通道配额耗尽且候选帧已过期 ({camera_id})"
                        )));
                    }
                } else {
                    return Err(PipelineError::Snapshot(format!(
                        "全局 VPU 抓拍通道配额耗尽且无同刻证据帧 ({camera_id})"
                    )));
                }
            }
        };

        self.snapshot_engine
            .save_snapshot_async(
                camera_id,
                frame_to_process,
                bbox,
                if is_sub_stream_frame {
                    EvidenceImageStream::Sub
                } else {
                    EvidenceImageStream::Main
                },
            )
            .await
    }

    /// 启动或更新某路摄像头的分析任务
    pub async fn start_task(
        &self,
        camera: &Camera,
        task: AnalysisTask,
    ) -> Result<(), PipelineError> {
        let rules = task.rules.clone();
        {
            let mut tasks = self.tasks.write().await;
            tasks.insert(camera.camera_id.clone(), task);
        }

        // 注册管线并激活按需解码
        self.set_ai_active(&camera.camera_id, true).await;

        let codec = if camera.last_codec.eq_ignore_ascii_case("h265")
            || camera.last_codec.eq_ignore_ascii_case("hevc")
        {
            types::CodecType::H265
        } else {
            types::CodecType::H264
        };

        // 按需拉起硬件解码器实例
        let ctx = self.get_or_create_context(&camera.camera_id).await;

        // 同步任务定义的空间几何布防规则至管线上下文
        *ctx.rules.write().await = rules;
        ctx.rules_version.fetch_add(1, Ordering::Release);

        // 同步设置主码流分析模式状态 (方案三：零解码瞬时直通)
        // 注意：生产环境统一由 TaskRuntimeCoordinator 依据网络动态探活决议生效模式；
        // 此处为独立管理任务或单测运行提供基于摄像头静态契约的初始同步。
        self.set_main_stream_analysis(&camera.camera_id, camera.is_main_stream_analysis())
            .await;

        let mut dec_guard = ctx.snapshot_decoder.lock().await;
        if dec_guard.is_none() {
            *dec_guard = Some(media::create_decoder(&camera.camera_id, codec));
            tracing::info!(camera_id = %camera.camera_id, ?codec, "已按需初始化硬件解码器实例");
        }

        tracing::info!(camera_id = %camera.camera_id, "分析管线任务已启动/更新");
        Ok(())
    }

    /// 配置摄像头的局部特写 Pre-crop ROI 映射区域
    pub async fn set_camera_roi(&self, camera_id: &str, roi: Option<BoundingBox>) {
        let ctx = self.get_or_create_context(camera_id).await;
        let mut mapper = ctx.roi_mapper.write().await;
        *mapper = RoiAffineMapper::new(roi);
        tracing::info!(camera_id = %camera_id, ?roi, "已配置摄像头局部 Pre-crop ROI 映射");
    }

    /// 配置摄像头的空间几何布防规则集合
    pub async fn set_camera_rules(&self, camera_id: &str, rules: Vec<DetectionRule>) {
        let ctx = self.get_or_create_context(camera_id).await;
        let mut r = ctx.rules.write().await;
        *r = rules;
        ctx.rules_version.fetch_add(1, Ordering::Release);
        tracing::info!(camera_id = %camera_id, count = r.len(), "已更新摄像头空间几何布防规则");
    }

    /// 统一处理算法推理输出的检测结果并执行指定算法实例的航迹跟踪与几何规则判定：
    /// 1. 执行 Pre-crop ROI 线性仿射坐标还原（将局部归一化 [0,1] 映射至全景大图 [0,1]）；
    /// 2. 独立算法实例的航迹关联更新（按 algorithm_id 隔离 Tracker，避免航迹冲刷）；
    /// 3. 根据 algorithm_kind 自动区分责任流向：
    ///    - `AlgorithmKind::Recognition`：客观通行抓拍流，默认全屏捕获/ROI/Line判定，绝不误报入侵；
    ///    - `AlgorithmKind::Detection`：安全防范告警流，触犯空间规则（或全屏布防）触发告警；
    /// 4. 返回包含活跃航迹、违规告警与客观抓拍的 AnalysisOutcome。
    pub async fn process_detections_for_algo(
        &self,
        camera_id: &str,
        algorithm_id: &str,
        algorithm_kind: impl Into<types::AlgorithmKind>,
        detections: Vec<Detection>,
        timestamp_ms: i64,
    ) -> AnalysisOutcome {
        let embeddings = (0..detections.len()).map(|_| None).collect();
        self.process_detections_for_algo_with_embeddings(
            camera_id,
            algorithm_id,
            algorithm_kind,
            detections,
            embeddings,
            timestamp_ms,
        )
        .await
    }

    /// 处理检测结果并转移 C ABI 低频特征 sidecar。
    pub async fn process_detections_for_algo_with_embeddings(
        &self,
        camera_id: &str,
        algorithm_id: &str,
        algorithm_kind: impl Into<types::AlgorithmKind>,
        detections: Vec<Detection>,
        embeddings: Vec<Option<types::FaceEmbedding>>,
        timestamp_ms: i64,
    ) -> AnalysisOutcome {
        self.process_detections_for_algo_with_embeddings_internal(
            camera_id,
            algorithm_id,
            algorithm_kind.into(),
            detections,
            embeddings,
            timestamp_ms,
            None,
        )
        .await
    }

    /// 使用抽帧时捕获的会话代际处理结果，拒绝 SourceReset 前已在途的旧帧。
    #[allow(clippy::too_many_arguments)]
    pub async fn process_detections_for_algo_with_embeddings_at_generation(
        &self,
        camera_id: &str,
        algorithm_id: &str,
        algorithm_kind: impl Into<types::AlgorithmKind>,
        detections: Vec<Detection>,
        embeddings: Vec<Option<types::FaceEmbedding>>,
        timestamp_ms: i64,
        expected_generation: u64,
    ) -> AnalysisOutcome {
        self.process_detections_for_algo_with_embeddings_internal(
            camera_id,
            algorithm_id,
            algorithm_kind.into(),
            detections,
            embeddings,
            timestamp_ms,
            Some(expected_generation),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn process_detections_for_algo_with_embeddings_internal(
        &self,
        camera_id: &str,
        algorithm_id: &str,
        algorithm_kind: types::AlgorithmKind,
        mut detections: Vec<Detection>,
        embeddings: Vec<Option<types::FaceEmbedding>>,
        timestamp_ms: i64,
        expected_generation: Option<u64>,
    ) -> AnalysisOutcome {
        let ctx = self.get_or_create_context(camera_id).await;
        let generation = ctx.tracking_generation.load(Ordering::Acquire);
        if generation & 1 != 0 || expected_generation.is_some_and(|expected| expected != generation)
        {
            return AnalysisOutcome::default();
        }

        // 1. 局部仿射映射至全景坐标系 (原地变换，避免多余堆分配)
        let mapper = *ctx.roi_mapper.read().await;
        for det in &mut detections {
            det.bbox = mapper.map_bbox(&det.bbox);
            if let Some(f) = &mut det.face {
                f.bbox = mapper.map_bbox(&f.bbox);
            }
        }
        let global_detections = detections;
        let rules = ctx.rules.read().await.clone();

        // 2. 独立算法实例的航迹关联更新。所有锁内工作均为同步计算，避免持锁跨 await。
        let (tracked_objects, alarms, capture_actions) = {
            let mut trackers = ctx.trackers.lock().await;
            if generation != ctx.tracking_generation.load(Ordering::Acquire) {
                return AnalysisOutcome::default();
            }

            let tracker = match trackers.get_mut(algorithm_id) {
                Some(tracker) => tracker,
                None => trackers.entry(algorithm_id.to_string()).or_default(),
            };
            let update = tracker.update_with_embeddings_at_result(
                global_detections,
                embeddings,
                timestamp_ms,
            );
            if update.status != TrackUpdateStatus::Applied {
                return AnalysisOutcome::default();
            }

            let tracked_objects = update.objects;
            let (alarms, capture_actions) = if algorithm_kind.is_recognition() {
                // 识别类：抓拍结算状态机推进（峰值跟踪/候选留存/结算判定）。
                // 冷却与结算时机由控制器统一管理，不再在进入 ROI 的首帧直接产事件。
                let actions = {
                    let mut settle = lock_capture_settle(&ctx);
                    settle.observe(algorithm_id, &tracked_objects, &rules, timestamp_ms)
                };
                (Vec::new(), actions)
            } else {
                let alarms = ctx.rule_evaluator.evaluate(
                    &rules,
                    &tracked_objects,
                    tracker,
                    timestamp_ms,
                    5000,
                );
                (alarms, Vec::new())
            };
            (tracked_objects, alarms, capture_actions)
        };

        // 清理/换流可能在 tracker 锁释放后发生，代际校验阻止旧结果回写公共快照。
        if generation != ctx.tracking_generation.load(Ordering::Acquire) {
            return AnalysisOutcome::default();
        }

        // 同步更新最新航迹快照 (PRD R1.1: 维护活跃航迹快照)。高频快照不携带
        // backend-only embedding，识别抓拍仍使用下方 `tracked_objects` 原值。
        let public_tracks: Vec<TrackedObject> = tracked_objects
            .iter()
            .map(TrackedObject::without_embedding)
            .collect();
        let mut current = ctx.current_tracks.write().await;
        if generation != ctx.tracking_generation.load(Ordering::Acquire) {
            return AnalysisOutcome::default();
        }
        if public_tracks.is_empty() {
            current.remove(algorithm_id);
        } else if let Some(existing) = current.get_mut(algorithm_id) {
            *existing = public_tracks;
        } else {
            current.insert(algorithm_id.to_string(), public_tracks);
        }

        AnalysisOutcome {
            applied: true,
            tracked: tracked_objects,
            alarms,
            capture_actions,
        }
    }

    /// 驱动管线执行航迹跟踪与几何规则判定 (向后兼容单算法入口)
    pub async fn process_detections(
        &self,
        camera_id: &str,
        detections: Vec<Detection>,
        timestamp_ms: i64,
    ) -> (Vec<TrackedObject>, Vec<TriggeredAlarm>) {
        let outcome = self
            .process_detections_for_algo(
                camera_id,
                "default",
                types::AlgorithmKind::Detection,
                detections,
                timestamp_ms,
            )
            .await;
        (outcome.tracked, outcome.alarms)
    }

    async fn mount_pump(&self, camera_id: &str, pump: AnalysisPump) {
        let old_pump = {
            let mut pumps = self.pumps.write().await;
            pumps.remove(camera_id)
        };
        if let Some(mut old) = old_pump {
            old.stop().await;
            self.clear_tracking(camera_id).await;
        }
        self.pumps.write().await.insert(camera_id.to_string(), pump);
    }

    /// 启动某路摄像头的有效分析码流驱动泵
    pub async fn start_analysis_pump(
        self: &Arc<Self>,
        camera_id: &str,
        session: Arc<media::CameraStreamSession>,
        decoder: Box<dyn VideoDecoder + Send>,
        worker: infer::InferenceWorkerHandle,
        config: AnalysisPumpConfig,
    ) {
        let pump = AnalysisPump::start(camera_id, session, decoder, worker, self.clone(), config);
        self.mount_pump(camera_id, pump).await;
        tracing::info!(camera_id = %camera_id, "分析码流驱动泵已挂载至管线管理器");
    }

    /// 启动某路摄像头的有效分析码流驱动泵 (全量托管 InferenceWorker 运行周期)
    pub async fn start_analysis_pump_with_worker(
        self: &Arc<Self>,
        camera_id: &str,
        session: Arc<media::CameraStreamSession>,
        decoder: Box<dyn VideoDecoder + Send>,
        worker: infer::InferenceWorker,
        config: AnalysisPumpConfig,
    ) {
        let pump = AnalysisPump::start_with_worker(
            camera_id,
            session,
            decoder,
            worker,
            self.clone(),
            config,
        );
        self.mount_pump(camera_id, pump).await;
        tracing::info!(camera_id = %camera_id, "分析码流驱动泵 (含常驻工作线程) 已挂载至管线管理器");
    }

    /// 启动某路摄像头的多算法实例有效分析码流驱动泵
    pub async fn start_analysis_pump_multi_worker(
        self: &Arc<Self>,
        camera_id: &str,
        session: Arc<media::CameraStreamSession>,
        decoder: Box<dyn VideoDecoder + Send>,
        instance_configs: Vec<crate::pump::WorkerInstanceConfig>,
        workers: Vec<(
            String,
            infer::InferenceWorkerHandle,
            Option<infer::InferenceWorker>,
        )>,
        motion_gate: Option<types::MotionGateConfig>,
    ) {
        let ctx = self.get_or_create_context(camera_id).await;
        let rules = ctx.rules.clone();
        let rules_version = ctx.rules_version.clone();
        let pump = AnalysisPump::start_multi_worker(
            camera_id,
            session,
            decoder,
            instance_configs,
            workers,
            self.clone(),
            MotionGateRuntimeConfig {
                gate: motion_gate,
                rules,
                rules_version,
            },
        );
        self.mount_pump(camera_id, pump).await;
        tracing::info!(camera_id = %camera_id, "多算法分析驱动泵已挂载至管线管理器");
    }

    /// 停止某路摄像头的有效分析码流驱动泵
    pub async fn stop_analysis_pump(&self, camera_id: &str) -> bool {
        let old_pump = {
            let mut pumps = self.pumps.write().await;
            pumps.remove(camera_id)
        };
        let stopped = if let Some(mut pump) = old_pump {
            pump.stop().await;
            self.clear_decoded_ring(camera_id).await;
            tracing::info!(camera_id = %camera_id, "分析码流驱动泵已停止并从管理器注销");
            true
        } else {
            false
        };
        self.clear_tracking(camera_id).await;
        stopped
    }

    /// 停止所有摄像头的分析驱动泵并等待回收
    pub async fn stop_all_pumps(&self) {
        let old_pumps = {
            let mut pumps = self.pumps.write().await;
            pumps.drain().collect::<Vec<_>>()
        };
        for (cam_id, mut pump) in old_pumps {
            pump.stop().await;
            self.clear_decoded_ring(&cam_id).await;
            self.clear_tracking(&cam_id).await;
        }
        tracing::info!("已停止所有分析驱动泵并回收资源");
    }

    /// 查询某路摄像头的驱动泵是否正在运行
    pub async fn is_analysis_pump_running(&self, camera_id: &str) -> bool {
        let pumps = self.pumps.read().await;
        pumps
            .get(camera_id)
            .map(|p| p.is_running())
            .unwrap_or(false)
    }

    /// 原子热替换指定摄像头的推理 Worker 句柄 (不断流、零中断)
    /// 原子替换某路摄像头指定算法实例的子码流推理 Worker 句柄
    pub async fn replace_pump_worker_for_algo(
        &self,
        camera_id: &str,
        algorithm_id: &str,
        new_worker: infer::InferenceWorkerHandle,
    ) -> Result<(), PipelineError> {
        let pumps = self.pumps.read().await;
        if let Some(pump) = pumps.get(camera_id) {
            pump.replace_worker(algorithm_id, new_worker).await;
            tracing::info!(camera_id = %camera_id, algorithm_id = %algorithm_id, "已在两帧间隙原子完成子码流推理 Worker 优雅热重载");
            Ok(())
        } else {
            Err(PipelineError::PipelineNotFound {
                camera_id: camera_id.to_string(),
            })
        }
    }

    /// 使用已创建且由目标驱动泵托管的 Worker 原子替换算法实例。
    pub async fn replace_pump_worker_for_algo_with_owner(
        &self,
        camera_id: &str,
        algorithm_id: &str,
        new_worker: InferenceWorker,
    ) -> Result<bool, PipelineError> {
        let pumps = self.pumps.read().await;
        if let Some(pump) = pumps.get(camera_id) {
            Ok(pump
                .replace_worker_with_owner(algorithm_id, new_worker)
                .await)
        } else {
            Err(PipelineError::PipelineNotFound {
                camera_id: camera_id.to_string(),
            })
        }
    }
    /// 向后兼容：原子替换单 Worker 或首个算法 Worker
    pub async fn replace_pump_worker(
        &self,
        camera_id: &str,
        new_worker: infer::InferenceWorkerHandle,
    ) -> Result<(), PipelineError> {
        self.replace_pump_worker_for_algo(
            camera_id,
            crate::pump::LEGACY_SINGLE_WORKER_ID,
            new_worker,
        )
        .await
    }

    /// 全局热重载：为每个匹配的驱动泵创建并托管独立推理 Worker。
    pub async fn reload_algorithm_on_pumps(
        &self,
        algorithm_id: &str,
        package: Arc<AlgoPackage>,
    ) -> usize {
        let targets = {
            let pumps = self.pumps.read().await;
            let mut targets = Vec::new();
            for (camera_id, pump) in pumps.iter() {
                if let Some(config_json) = pump.worker_config_for_algorithm(algorithm_id).await {
                    targets.push((camera_id.clone(), config_json));
                }
            }
            targets
        };

        let mut count = 0;
        for (camera_id, config_json) in targets {
            let package = package.clone();
            let instance_id = format!("hot-reload-{algorithm_id}-{camera_id}");
            let worker_camera_id = camera_id.clone();
            let worker_algorithm_id = algorithm_id.to_string();
            let worker_result = tokio::task::spawn_blocking(move || {
                let worker_config = InferenceWorkerConfig {
                    worker_name: format!(
                        "infer-worker-hot-reload-{worker_algorithm_id}-{worker_camera_id}"
                    ),
                    ..Default::default()
                };
                package.create_worker(&instance_id, config_json.as_deref(), worker_config)
            })
            .await;

            let worker = match worker_result {
                Ok(Ok(worker)) => worker,
                Ok(Err(err)) => {
                    tracing::error!(
                        camera_id = %camera_id,
                        algorithm_id = %algorithm_id,
                        error = %err,
                        "算法版本热重载创建推理 Worker 失败"
                    );
                    continue;
                }
                Err(err) => {
                    tracing::error!(
                        camera_id = %camera_id,
                        algorithm_id = %algorithm_id,
                        error = %err,
                        "算法版本热重载阻塞任务异常退出"
                    );
                    continue;
                }
            };

            match self
                .replace_pump_worker_for_algo_with_owner(&camera_id, algorithm_id, worker)
                .await
            {
                Ok(true) => {
                    tracing::info!(
                        camera_id = %camera_id,
                        algorithm_id = %algorithm_id,
                        "已在两帧间隙热切换并托管推理 Worker"
                    );
                    count += 1;
                }
                Ok(false) => {}
                Err(err) => tracing::warn!(
                    camera_id = %camera_id,
                    algorithm_id = %algorithm_id,
                    error = %err,
                    "算法版本热重载时目标驱动泵已退出"
                ),
            }
        }
        count
    }

    /// 获取某路摄像头的驱动泵运行指标
    pub async fn get_analysis_pump_metrics(&self, camera_id: &str) -> Option<Arc<PumpMetrics>> {
        let pumps = self.pumps.read().await;
        pumps.get(camera_id).map(|p| p.metrics().clone())
    }

    /// 获取某路摄像头各算法实例的运行指标快照（按 instanceId 维度）
    pub async fn get_instance_metrics(
        &self,
        camera_id: &str,
    ) -> Vec<(String, Arc<crate::pump::InstanceMetrics>)> {
        let Some(handle) = self.pump_control_handle(camera_id).await else {
            return Vec::new();
        };
        handle.instance_metrics().await
    }

    /// 列出某路摄像头当前挂载的算法实例描述（instanceId / algorithmId / 生效配置）
    pub async fn get_instance_descriptors(
        &self,
        camera_id: &str,
    ) -> Vec<crate::pump::InstanceDescriptor> {
        let Some(handle) = self.pump_control_handle(camera_id).await else {
            return Vec::new();
        };
        handle.instance_descriptors().await
    }

    /// 向运行中的分析泵增量挂载算法实例（Worker 已创建并完成资源准入）
    pub async fn add_pump_instance(
        &self,
        camera_id: &str,
        config: crate::pump::WorkerInstanceConfig,
        worker: infer::InferenceWorker,
    ) -> Result<bool, PipelineError> {
        let handle = self.require_pump_control_handle(camera_id).await?;
        Ok(handle.add_instance(config, worker).await)
    }

    /// 从运行中的分析泵移除算法实例并回收其 Worker
    pub async fn remove_pump_instance(
        &self,
        camera_id: &str,
        instance_id: &str,
    ) -> Result<bool, PipelineError> {
        let handle = self.require_pump_control_handle(camera_id).await?;
        Ok(handle.remove_instance(instance_id).await)
    }

    /// 在帧边界更新目标实例的抽帧频率
    pub async fn set_pump_instance_fps(
        &self,
        camera_id: &str,
        instance_id: &str,
        target_fps: u32,
    ) -> Result<bool, PipelineError> {
        let handle = self.require_pump_control_handle(camera_id).await?;
        Ok(handle.set_instance_fps(instance_id, target_fps).await)
    }

    /// 在目标实例的 Worker 硬件上下文内原地更新配置
    pub async fn update_pump_instance_config(
        &self,
        camera_id: &str,
        instance_id: &str,
        config_json: &str,
    ) -> Result<crate::pump::InstanceConfigUpdateOutcome, PipelineError> {
        let handle = self.require_pump_control_handle(camera_id).await?;
        Ok(handle
            .update_instance_config(instance_id, config_json)
            .await)
    }

    /// 替换目标实例的 Worker（模型/算法版本变更路径）
    pub async fn replace_pump_instance_worker(
        &self,
        camera_id: &str,
        instance_id: &str,
        worker: infer::InferenceWorker,
    ) -> Result<bool, PipelineError> {
        let handle = self.require_pump_control_handle(camera_id).await?;
        Ok(handle.replace_instance_worker(instance_id, worker).await)
    }

    /// 获取指定摄像头分析泵的控制面句柄，未运行时返回 PipelineNotFound 错误
    async fn require_pump_control_handle(
        &self,
        camera_id: &str,
    ) -> Result<crate::pump::PumpControlHandle, PipelineError> {
        self.pump_control_handle(camera_id)
            .await
            .ok_or_else(|| PipelineError::PipelineNotFound {
                camera_id: camera_id.to_string(),
            })
    }

    /// 克隆指定摄像头分析泵的控制面句柄（在注册表锁内完成克隆后立即释放锁）
    async fn pump_control_handle(&self, camera_id: &str) -> Option<crate::pump::PumpControlHandle> {
        let pumps = self.pumps.read().await;
        pumps.get(camera_id).map(|pump| pump.control_handle())
    }

    /// 停止某路摄像头的分析任务
    pub async fn stop_task(&self, camera_id: &str) -> Result<(), PipelineError> {
        let removed = {
            let mut tasks = self.tasks.write().await;
            tasks.remove(camera_id).is_some()
        };

        if removed {
            // 级联停用对应的分析驱动泵
            self.stop_analysis_pump(camera_id).await;

            self.set_ai_active(camera_id, false).await;

            let ctx = {
                let pipelines = self.pipelines.read().await;
                pipelines.get(camera_id).cloned()
            };
            if let Some(ctx) = ctx {
                ctx.release_decoder_if_idle().await;
            }

            tracing::info!(camera_id = %camera_id, "分析管线任务已停止");
            Ok(())
        } else {
            Err(PipelineError::PipelineNotFound {
                camera_id: camera_id.to_string(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture_settle::FrameGeometry;
    use crate::test_support::{dir_entry_count, test_nv12_frame};
    use bytes::Bytes;
    use types::{CodecType, FrameHandle, PixelFormat, StrideInfo};

    /// 构造已标定的接入时延锚点（低分位数需足量样本才收敛）。
    fn calibrated_anchor(lag_ms: i64) -> Arc<StreamClockAnchor> {
        let anchor = Arc::new(StreamClockAnchor::new());
        for _ in 0..64 {
            anchor.observe(lag_ms);
        }
        assert!(anchor.lag_ms().is_some());
        anchor
    }

    /// 跨流换算必须把检测时标投到主码流轴；未标定/未注入时必须返回 None，
    /// 让调用方退回检测流自身的帧，而不是假定偏移为 0。
    #[tokio::test]
    async fn test_resolve_main_axis_pts_requires_calibration() {
        let temp_dir =
            std::env::temp_dir().join(format!("test_cross_axis_{}", uuid::Uuid::now_v7().simple()));
        let manager = PipelineManager::with_evidence_dir(&temp_dir);
        let cam_id = "cam_cross_axis";
        let ctx = manager.get_or_create_context(cam_id).await;

        // 主码流滞后 220ms、子码流滞后 120ms：同一真实时刻在主码流轴上数值小 100ms。
        manager
            .set_stream_clock_anchors(cam_id, calibrated_anchor(220), calibrated_anchor(120))
            .await;
        assert_eq!(
            manager.resolve_main_axis_pts(&ctx, cam_id, 1180).await,
            Some(1080)
        );

        // 任一侧未标定 → 拒绝跨流取证。
        manager
            .set_stream_clock_anchors(
                cam_id,
                Arc::new(StreamClockAnchor::new()),
                calibrated_anchor(120),
            )
            .await;
        assert_eq!(
            manager.resolve_main_axis_pts(&ctx, cam_id, 1180).await,
            None
        );

        // 锚点未注入 → 同样拒绝。
        manager.clear_stream_clock_anchors(cam_id).await;
        assert_eq!(
            manager.resolve_main_axis_pts(&ctx, cam_id, 1180).await,
            None
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_pipeline_manager_on_demand_and_snapshot() {
        let temp_dir = std::env::temp_dir().join(format!(
            "test_pipe_evidence_{}",
            uuid::Uuid::now_v7().simple()
        ));
        let manager = PipelineManager::with_evidence_dir(&temp_dir);

        let cam_id = "cam_dual_stream_001";
        let ctx = manager.get_or_create_context(cam_id).await;

        // 初始状态：无 AI 无预览，不需要解码
        assert!(!ctx.is_decoder_needed());

        // 增加预览，按需解码开启
        manager.increment_preview(cam_id).await;
        assert!(ctx.is_decoder_needed());

        // 减少预览，按需解码关闭
        manager.decrement_preview(cam_id).await;
        assert!(!ctx.is_decoder_needed());

        // 再次测试连续多次扣减不会发生 underflow 变成 usize::MAX
        manager.decrement_preview(cam_id).await;
        manager.decrement_preview(cam_id).await;
        assert_eq!(ctx.preview_count.load(Ordering::Relaxed), 0);
        assert!(!ctx.is_decoder_needed());

        // 推入主码流 NALU
        let dummy_pkt = Arc::new(EncodedPacket {
            pts_ms: 1741100050000,
            is_keyframe: true,
            codec: CodecType::H264,
            payload: Bytes::from_static(b"\x00\x00\x00\x01\x67fake"),
            ..Default::default()
        });
        manager.push_main_packet(cam_id, dummy_pkt).await;
        assert_eq!(ctx.ring_buffer.len(), 1);

        // 设置子码流降级帧
        let dummy_frame = FrameRef::new(
            cam_id.to_string(),
            1741100050000,
            640,
            360,
            StrideInfo::new(640, 360),
            PixelFormat::Nv12,
            FrameHandle::Host(vec![128u8; 640 * 360 * 3 / 2].into()),
        );
        manager.update_sub_stream_frame(cam_id, dummy_frame).await;

        // 清空环形缓冲区以模拟主流未就绪，测试无主流解码器时平滑降级至子流帧
        ctx.ring_buffer.clear();

        // 触发抓拍 (无主流解码器时平滑降级至子流帧)
        let bbox = BoundingBox::new(0.1, 0.1, 0.5, 0.5);
        let snapshot = manager
            .trigger_snapshot(cam_id, 1741100050000, Some(bbox))
            .await
            .expect("抓拍应成功");

        assert!(snapshot.is_sub_stream());
        assert_eq!(snapshot.width, 640);
        assert_eq!(snapshot.height, 360);

        // 清理测试目录
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_on_demand_decoder_lifecycle() {
        let temp_dir =
            std::env::temp_dir().join(format!("test_lifecycle_{}", uuid::Uuid::now_v7().simple()));
        let manager = PipelineManager::with_evidence_dir(&temp_dir);
        let cam_id = "cam_lifecycle_001";

        let camera = Camera {
            id: 1,
            camera_id: cam_id.to_string(),
            name: "Test Cam".to_string(),
            protocol: "rtsp".to_string(),
            rtsp_url: "rtsp://127.0.0.1/live/main".to_string(),
            sub_rtsp_url: "".to_string(),
            stream_mode: types::StreamMode::Auto,
            remark: "".to_string(),
            transport_policy: types::TransportPolicy::Auto,
            last_probe_status: types::ProbeStatus::Healthy,
            last_probe_at: None,
            last_probe_error_code: "".to_string(),
            last_success_at: None,
            last_codec: "h264".to_string(),
            last_width: 1920,
            last_height: 1080,
            last_fps: 25.0,
            gb28181_device_id: None,
            gb28181_channel_id: None,
            created_at: 0,
            updated_at: 0,
        };

        let task = AnalysisTask {
            camera_id: cam_id.to_string(),
            name: "task_001".to_string(),
            desired_enabled: true,
            actual_status: types::TaskStatus::Running,
            status_message: "".to_string(),
            algorithm_id: "general_detection".to_string(),
            analysis_fps: 15,
            algo_params: types::task::default_algo_params(),
            rules: vec![],
            motion_gate: types::MotionGateConfig::default(),
            last_frame_at: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let ctx = manager.get_or_create_context(cam_id).await;
        assert!(ctx.snapshot_decoder.lock().await.is_none());

        // 1. 启动任务，自动按需拉起解码器
        manager
            .start_task(&camera, task)
            .await
            .expect("启动任务应成功");
        assert!(ctx.ai_active.load(Ordering::Relaxed));
        assert!(ctx.snapshot_decoder.lock().await.is_some());

        // 2. 停止任务且无活跃预览，自动销毁解码会话释放显存
        manager.stop_task(cam_id).await.expect("停止任务应成功");
        assert!(!ctx.ai_active.load(Ordering::Relaxed));
        assert!(ctx.snapshot_decoder.lock().await.is_none());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_pipeline_manager_tracking_and_rules_evaluation() {
        let temp_dir =
            std::env::temp_dir().join(format!("test_eval_{}", uuid::Uuid::now_v7().simple()));
        let manager = PipelineManager::with_evidence_dir(&temp_dir);
        let cam_id = "cam_eval_001";

        // 配置 Pre-crop ROI (右半区域 [0.5, 0.0, 1.0, 1.0])
        manager
            .set_camera_roi(cam_id, Some(BoundingBox::new(0.5, 0.0, 1.0, 1.0)))
            .await;

        // 配置入侵布防规则
        manager
            .set_camera_rules(
                cam_id,
                vec![types::DetectionRule {
                    role: types::DetectionRuleRole::Roi,
                    line_direction: types::DetectionLineDirection::Both,
                    points: vec![
                        types::DetectionPoint::new(0.5, 0.0),
                        types::DetectionPoint::new(1.0, 0.0),
                        types::DetectionPoint::new(1.0, 1.0),
                        types::DetectionPoint::new(0.5, 1.0),
                    ],
                }],
            )
            .await;

        // 模拟算法输出局部检测框 [0.2, 0.2, 0.4, 0.4]
        // 经仿射变换后映射为全景坐标: x1 = 0.5 + 0.2*0.5 = 0.6, y1 = 0.2, x2 = 0.7, y2 = 0.4
        let local_det1 = vec![Detection {
            class_id: 0,
            label: "person".to_string(),
            confidence: 0.95,
            quality_score: None,
            bbox: BoundingBox::new(0.2, 0.2, 0.4, 0.4),
            face: None,
        }];

        let (tracked1, alarms1) = manager.process_detections(cam_id, local_det1, 1000).await;
        assert_eq!(tracked1.len(), 1);
        assert_eq!(alarms1.len(), 1, "侵入全景布防区必须触发报警");
        let tid = tracked1[0].track_id;

        // 验证全景坐标映射正确性
        assert!((tracked1[0].bbox.x1 - 0.6).abs() < 1e-4);

        // 第 2 帧微移，测试航迹 ID 连续性与 5 秒防刷屏冷却
        let local_det2 = vec![Detection {
            class_id: 0,
            label: "person".to_string(),
            confidence: 0.96,
            quality_score: None,
            bbox: BoundingBox::new(0.21, 0.21, 0.41, 0.41),
            face: None,
        }];

        let (tracked2, alarms2) = manager.process_detections(cam_id, local_det2, 2000).await;
        assert_eq!(tracked2.len(), 1);
        assert_eq!(tracked2[0].track_id, tid, "Track ID 必须在帧间保持连续");
        assert_eq!(alarms2.len(), 0, "5 秒防刷屏冷却期内不应重复报警");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_vpu_concurrency_limiter_and_graceful_fallback() {
        let temp_dir =
            std::env::temp_dir().join(format!("test_vpu_limit_{}", uuid::Uuid::now_v7().simple()));
        let manager =
            PipelineManager::with_all_options(&temp_dir, SnapshotConfig::default(), 1, 10);
        let cam_id = "cam_vpu_limit_test";

        // 占满唯一的全局信号量配额
        let held_permit = manager
            .snapshot_semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("获取测试许可应成功");

        // 提供子码流备用帧 (640x360)
        let fallback_nv12 = vec![128u8; (640 * 360 * 3 / 2) as usize].into();
        let fallback_frame = FrameRef::new(
            cam_id.to_string(),
            1000,
            640,
            360,
            types::StrideInfo::new(640, 360),
            types::PixelFormat::Nv12,
            types::FrameHandle::Host(fallback_nv12),
        );
        manager
            .update_sub_stream_frame(cam_id, fallback_frame)
            .await;

        // 触发抓拍：因配额已被占满且超过 10ms，自动自适应降级复用子码流，杜绝崩溃或死锁
        let target = types::BoundingBox::new(0.1, 0.1, 0.3, 0.3);
        let snapshot = manager
            .trigger_snapshot(cam_id, 1000, Some(target))
            .await
            .expect("配额超限自适应降级抓拍应成功");

        assert!(snapshot.is_sub_stream());
        assert_eq!(snapshot.width, 640);
        assert_eq!(snapshot.height, 360);

        drop(held_permit);
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_main_stale_decoded_ring_frame_is_rejected() {
        let temp_dir = std::env::temp_dir().join(format!(
            "test_main_ring_fallback_{}",
            uuid::Uuid::now_v7().simple()
        ));
        let manager =
            PipelineManager::with_all_options(&temp_dir, SnapshotConfig::default(), 1, 10);
        let cam_id = "cam_main_ring_fallback";

        let held_permit = manager
            .snapshot_semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("获取测试许可应成功");
        manager.set_main_stream_analysis(cam_id, true).await;

        let frame = FrameRef::new(
            cam_id.to_string(),
            1000,
            1920,
            1080,
            types::StrideInfo::new(1920, 1080),
            types::PixelFormat::Nv12,
            types::FrameHandle::Host(vec![128u8; 1920 * 1080 * 3 / 2].into()),
        );
        manager.update_decoded_frame(cam_id, frame).await;

        // 目标偏差超过证据允许范围，配额占满后不得复用主流环中的过期帧。
        let result = manager
            .trigger_snapshot(
                cam_id,
                2000,
                Some(types::BoundingBox::new(0.1, 0.1, 0.3, 0.3)),
            )
            .await;

        assert!(result.is_err(), "过期主流环帧不能与当前检测框拼接");

        drop(held_permit);
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_main_analysis_uses_the_exact_inference_frame() {
        let temp_dir = std::env::temp_dir().join(format!(
            "test_main_exact_inference_frame_{}",
            uuid::Uuid::now_v7().simple()
        ));
        let manager = PipelineManager::with_evidence_dir(&temp_dir);
        let cam_id = "cam_main_exact_inference";
        let target_pts = 2_000;

        manager.set_main_stream_analysis(cam_id, true).await;
        let frame = FrameRef::new(
            cam_id.to_string(),
            target_pts,
            64,
            64,
            types::StrideInfo::new(64, 64),
            types::PixelFormat::Nv12,
            types::FrameHandle::Host(vec![128u8; 64 * 64 * 3 / 2].into()),
        );

        // 不提供 decoded_ring、主流 GOP 或按需解码器；成功只能来自推理实际消费的 FrameRef。
        let snapshot = manager
            .trigger_snapshot_for_frame(
                cam_id,
                target_pts,
                Some(types::BoundingBox::new(0.1, 0.1, 0.3, 0.3)),
                frame,
            )
            .await
            .expect("主流推理帧直通抓拍应成功");

        assert!(!snapshot.is_sub_stream());
        assert_eq!(snapshot.width, 64);
        assert_eq!(snapshot.height, 64);
        assert!(temp_dir.join(&snapshot.image_rel_path).is_file());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
    #[tokio::test]
    async fn test_pipeline_manager_pump_lifecycle_and_cascade_stop() {
        use async_trait::async_trait;
        use infer::InferenceBackend;
        use media::decoders::MockDecoder;
        use types::TransportPolicy;

        #[derive(Debug)]
        struct DummyInferBackend;
        #[async_trait(?Send)]
        impl InferenceBackend for DummyInferBackend {
            fn name(&self) -> &'static str {
                "DummyInfer"
            }
            async fn detect(
                &self,
                _frame: &FrameRef,
            ) -> Result<Vec<types::Detection>, infer::InferError> {
                Ok(Vec::new())
            }
        }

        let manager = Arc::new(PipelineManager::new());
        let cam_id = "cam_pump_lifecycle_test";

        let session =
            media::CameraStreamSession::mock(cam_id, "rtsp://dummy/sub", TransportPolicy::Tcp);
        session.active_viewers.store(1, Ordering::SeqCst);
        session.ai_task_enabled.store(true, Ordering::SeqCst);

        let decoder = Box::new(MockDecoder::new(cam_id, CodecType::H264, 640, 360));
        let worker = infer::InferenceWorker::new(Arc::new(DummyInferBackend));

        assert!(!manager.is_analysis_pump_running(cam_id).await);

        manager
            .start_analysis_pump(
                cam_id,
                session,
                decoder,
                worker.handle(),
                AnalysisPumpConfig::default(),
            )
            .await;

        assert!(manager.is_analysis_pump_running(cam_id).await);
        let metrics = manager.get_analysis_pump_metrics(cam_id).await;
        assert!(metrics.is_some());

        // 停止驱动泵
        let stopped = manager.stop_analysis_pump(cam_id).await;
        assert!(stopped);
        assert!(!manager.is_analysis_pump_running(cam_id).await);
    }

    #[tokio::test]
    async fn test_pipeline_manager_stop_all_pumps() {
        use async_trait::async_trait;
        use infer::InferenceBackend;
        use media::decoders::MockDecoder;
        use types::TransportPolicy;

        #[derive(Debug)]
        struct DummyInfer;
        #[async_trait(?Send)]
        impl InferenceBackend for DummyInfer {
            fn name(&self) -> &'static str {
                "DummyInfer"
            }
            async fn detect(
                &self,
                _frame: &FrameRef,
            ) -> Result<Vec<types::Detection>, infer::InferError> {
                Ok(Vec::new())
            }
        }

        let manager = Arc::new(PipelineManager::new());
        for i in 1..=2 {
            let cam_id = format!("cam_all_{i}");
            let session =
                media::CameraStreamSession::mock(&cam_id, "rtsp://dummy/sub", TransportPolicy::Tcp);

            let decoder = Box::new(MockDecoder::new(&cam_id, CodecType::H264, 640, 360));
            let worker = infer::InferenceWorker::new(Arc::new(DummyInfer));

            manager
                .start_analysis_pump(
                    &cam_id,
                    session,
                    decoder,
                    worker.handle(),
                    AnalysisPumpConfig::default(),
                )
                .await;

            assert!(manager.is_analysis_pump_running(&cam_id).await);
        }

        // 停止所有驱动泵
        manager.stop_all_pumps().await;

        assert!(!manager.is_analysis_pump_running("cam_all_1").await);
        assert!(!manager.is_analysis_pump_running("cam_all_2").await);
    }

    #[tokio::test]
    async fn test_attach_main_stream_lagged_handling() {
        use std::time::Duration;

        let manager = Arc::new(PipelineManager::new());
        let cam_id = "cam_lagged_test";

        let hub = media::StreamHub::new();
        let session = hub
            .get_or_create_session(
                cam_id,
                "rtsp://127.0.0.1:8554/test",
                types::TransportPolicy::Tcp,
            )
            .await;
        let subscription = hub
            .subscribe_existing_session(session.clone(), media::ConsumerKind::MainStreamEvidence)
            .await
            .expect("main stream subscription");
        let attach_handle = manager.attach_main_stream(cam_id, subscription);

        let make_pkt = |pts: i64, key: bool| {
            Arc::new(EncodedPacket {
                pts_ms: pts,
                is_keyframe: key,
                codec: CodecType::H264,
                payload: if key {
                    Bytes::from_static(b"\x00\x00\x00\x01\x67\x42\x00\x1e\x00\x00\x00\x01\x68\xce\x00\x00\x00\x01\x65idr")
                } else {
                    Bytes::from_static(b"\x00\x00\x00\x01\x41p")
                },
                ..Default::default()
            })
        };

        // 1. 发送第 1 包并等待进入 RingBuffer
        session.dispatcher.publish(make_pkt(1000, true));
        tokio::task::yield_now().await;
        assert!(!attach_handle.is_finished());
        tokio::time::sleep(Duration::from_millis(50)).await;
        let ctx = manager
            .get_pipeline_context(cam_id)
            .await
            .expect("context must exist");
        assert_eq!(ctx.ring_buffer.len(), 1);

        // 2. 模拟源流 epoch 重建：消费者必须清空残缺 GOP，等待 Replay。
        session.dispatcher.source_reset();
        // 在新 epoch 中发送完整关键帧，dispatcher 以 Replay 方式恢复 RingBuffer。
        session.dispatcher.publish(make_pkt(2000, true));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!ctx.ring_buffer.is_empty());
        let newest = ctx.ring_buffer.newest_pts().expect("newest pts must exist");
        assert_eq!(newest, 2000);

        attach_handle.abort();
        let _ = attach_handle.await;
    }

    #[tokio::test]
    async fn test_pending_alarm_events_and_drain() {
        use crate::events::{EvidenceStatus, PipelineAlarmEvent};
        use crate::rules::TriggeredAlarm;
        use types::DetectionRuleRole;

        let manager = Arc::new(PipelineManager::new());
        let cam_id = "cam_alarm_queue_test";

        assert_eq!(manager.pending_alarm_event_count(), 0);

        let dummy_alarm = TriggeredAlarm {
            rule_index: 0,
            role: DetectionRuleRole::Line,
            tracked_object: types::TrackedObject {
                track_id: 1,
                class_id: 0,
                label: "person".to_string(),
                confidence: 0.9,
                quality_score: None,
                embedding: None,
                bbox: BoundingBox::new(0.1, 0.1, 0.4, 0.4),
                face: None,
                trajectory: vec![],
            },
            occurred_at_ms: 1000,
        };

        let event = PipelineAlarmEvent {
            event_id: "evt_123".to_string(),
            camera_id: cam_id.to_string(),
            algorithm_id: "test_algo".to_string(),
            alarm: dummy_alarm,
            snapshot: None,
            evidence_status: EvidenceStatus::Ready,
            evidence_error: None,
            timestamp: 1000,
        };

        manager.publish_analysis_event(PipelineAnalysisEvent::Alarm(Box::new(event)));

        assert_eq!(manager.pending_alarm_event_count(), 1);

        let drained = manager.drain_pending_alarm_events(10);
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].event_id, "evt_123");
        assert_eq!(manager.pending_alarm_event_count(), 0);
    }

    #[tokio::test]
    async fn test_pending_capture_events_and_drain() {
        use crate::events::PipelineCaptureEvent;

        let manager = Arc::new(PipelineManager::new());
        let cam_id = "cam_capture_queue_test";

        assert_eq!(manager.pending_capture_event_count(), 0);

        let event = PipelineCaptureEvent {
            capture_id: "cap_123".to_string(),
            camera_id: cam_id.to_string(),
            algorithm_id: "face_recognition".to_string(),
            tracked_object: types::TrackedObject {
                track_id: 10,
                class_id: 0,
                label: "face".to_string(),
                confidence: 0.98,
                quality_score: Some(0.85),
                embedding: None,
                bbox: types::BoundingBox::new(0.2, 0.2, 0.5, 0.5),
                face: None,
                trajectory: vec![],
            },
            snapshot: None,
            timestamp: 2000,
        };

        manager.publish_analysis_event(PipelineAnalysisEvent::Capture(Box::new(event)));

        assert_eq!(manager.pending_capture_event_count(), 1);

        let drained = manager.drain_pending_capture_events(10);
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].capture_id, "cap_123");
        assert_eq!(manager.pending_capture_event_count(), 0);
    }

    #[tokio::test]
    async fn test_remove_pipeline_context_if_idle() {
        let manager = Arc::new(PipelineManager::new());
        let cam_id = "cam_context_idle_test";

        let _ctx = manager.get_or_create_context(cam_id).await;
        assert!(manager.get_pipeline_context(cam_id).await.is_some());

        // 初始状态下无 task、无 pump、无 AI、无预览，属于闲置状态，可成功回收
        let removed = manager.remove_pipeline_context_if_idle(cam_id).await;
        assert!(removed);
        assert!(manager.get_pipeline_context(cam_id).await.is_none());

        // 当开启 AI 时，不可移除
        let _ctx = manager.get_or_create_context(cam_id).await;
        manager.set_ai_active(cam_id, true).await;
        let removed = manager.remove_pipeline_context_if_idle(cam_id).await;
        assert!(!removed);
        assert!(manager.get_pipeline_context(cam_id).await.is_some());

        // 关闭 AI 后可移除
        manager.set_ai_active(cam_id, false).await;
        let removed = manager.remove_pipeline_context_if_idle(cam_id).await;
        assert!(removed);
        assert!(manager.get_pipeline_context(cam_id).await.is_none());
    }

    #[tokio::test]
    async fn test_has_preview_subscribers() {
        let manager = Arc::new(PipelineManager::new());
        let cam_id = "cam_preview_test";

        assert!(!manager.has_preview_subscribers(cam_id).await);

        manager.increment_preview(cam_id).await;
        assert!(manager.has_preview_subscribers(cam_id).await);

        manager.increment_preview(cam_id).await;
        assert!(manager.has_preview_subscribers(cam_id).await);

        manager.decrement_preview(cam_id).await;
        assert!(manager.has_preview_subscribers(cam_id).await);

        manager.decrement_preview(cam_id).await;
        assert!(!manager.has_preview_subscribers(cam_id).await);
    }

    /// 候选全生命周期：留存仅编码驻留内存、覆盖写保持单份、结算时唯一一次写盘、清轨释放。
    #[tokio::test]
    async fn capture_candidate_encodes_in_memory_and_writes_once_at_settle() {
        let temp_dir = std::env::temp_dir().join(format!(
            "test_capture_settle_{}",
            uuid::Uuid::now_v7().simple()
        ));
        let manager =
            PipelineManager::with_all_options(temp_dir.clone(), SnapshotConfig::default(), 2, 1000);
        let camera_id = "cam_settle_lifecycle";
        let algorithm_id = "algo_face";
        let ctx = manager.get_or_create_context(camera_id).await;
        let rules: Vec<DetectionRule> = Vec::new();

        let object = TrackedObject {
            track_id: 7,
            class_id: 0,
            label: "face".to_string(),
            confidence: 0.95,
            quality_score: Some(0.80),
            embedding: None,
            bbox: BoundingBox::new(0.4, 0.3, 0.6, 0.6),
            face: None,
            trajectory: vec![(0.5, 0.6)],
        };

        // 首帧进入 ROI：挂起并产出首张候选留存动作（不再直接抓拍）
        let actions = {
            let mut settle = lock_capture_settle(&ctx);
            settle.observe(algorithm_id, std::slice::from_ref(&object), &rules, 1000)
        };
        assert!(matches!(
            actions[0],
            crate::capture_settle::CaptureAction::RetainCandidate(_)
        ));

        // 1. 留存 = 仅编码驻留内存：不产生任何盘上产物
        let first = CandidateRetainRequest {
            track_id: 7,
            geometry: FrameGeometry {
                bbox: object.bbox,
                face_bbox: None,
                pts_ms: 1000,
                quality: 0.80,
            },
        };
        manager
            .retain_capture_candidate(
                camera_id,
                algorithm_id,
                &first,
                test_nv12_frame(camera_id, 1000),
            )
            .await
            .expect("候选留存失败");
        let first_bytes = {
            let settle = lock_capture_settle(&ctx);
            settle.pending_bytes()
        };
        assert!(first_bytes > 0, "候选必须驻留内存");
        assert_eq!(
            dir_entry_count(&temp_dir),
            0,
            "候选留存阶段不得创建盘上产物"
        );

        // 2. 覆盖写：驻留替换为单份新候选（JPEG 字节随轨道条目释放，不累积）
        let second = CandidateRetainRequest {
            track_id: 7,
            geometry: FrameGeometry {
                bbox: BoundingBox::new(0.35, 0.25, 0.65, 0.65),
                face_bbox: Some(BoundingBox::new(0.45, 0.35, 0.55, 0.5)),
                pts_ms: 1200,
                quality: 0.85,
            },
        };
        manager
            .retain_capture_candidate(
                camera_id,
                algorithm_id,
                &second,
                test_nv12_frame(camera_id, 1200),
            )
            .await
            .expect("覆盖候选失败");
        let replaced_bytes = {
            let settle = lock_capture_settle(&ctx);
            settle.pending_bytes()
        };
        assert!(
            replaced_bytes <= 2 * first_bytes,
            "覆盖写后驻留仍应为单份候选 (first={first_bytes}, replaced={replaced_bytes})"
        );
        assert_eq!(dir_entry_count(&temp_dir), 0, "覆盖写不得产生盘上产物");

        // 3. 平台期结算：候选随结算动作携出，写入正式证据目录（唯一一次落盘）
        let actions = {
            let mut settle = lock_capture_settle(&ctx);
            settle.observe(algorithm_id, std::slice::from_ref(&object), &rules, 1600)
        };
        let candidate = match &actions[0] {
            crate::capture_settle::CaptureAction::Settle(request) => request
                .candidate
                .clone()
                .expect("平台期结算必须携带驻留候选"),
            other => panic!("期望结算动作，实际为 {other:?}"),
        };
        assert_eq!(candidate.geometry.pts_ms, 1200);
        let written = manager
            .write_capture_candidate(camera_id, &candidate)
            .await
            .expect("结算写盘失败");
        assert!(
            written.image_rel_path.starts_with(&format!("{camera_id}/")),
            "结算产物必须落在正式证据目录"
        );
        assert!(temp_dir.join(&written.image_rel_path).is_file());
        assert!(temp_dir.join(&written.crop_image_rel_path).is_file());
        assert_eq!(written.width, 320);
        assert_eq!(written.height, 240);
        assert_eq!(
            written.image_source,
            types::EvidenceImageSource::PeakCandidate,
            "结算写盘产物必须标记为峰值候选帧"
        );
        assert_eq!(
            written.image_stream,
            types::EvidenceImageStream::Sub,
            "候选帧来自分析码流：测试上下文未启主码流常驻分析，即子码流"
        );
        assert_eq!(written.frame_pts_ms, candidate.geometry.pts_ms);
        assert_eq!(
            written.comparable_frame_pts_ms(),
            written.frame_pts_ms,
            "峰值候选帧与检测帧同轴，PTS 必须可直接比对"
        );

        // 4. 冷却后重入并留存：清轨必须同步释放内存候选，且不产生额外盘上产物
        let actions = {
            let mut settle = lock_capture_settle(&ctx);
            settle.observe(algorithm_id, std::slice::from_ref(&object), &rules, 7000)
        };
        assert!(matches!(
            actions[0],
            crate::capture_settle::CaptureAction::RetainCandidate(_)
        ));
        let third = CandidateRetainRequest {
            track_id: 7,
            geometry: FrameGeometry {
                bbox: object.bbox,
                face_bbox: None,
                pts_ms: 7000,
                quality: 0.80,
            },
        };
        manager
            .retain_capture_candidate(
                camera_id,
                algorithm_id,
                &third,
                test_nv12_frame(camera_id, 7000),
            )
            .await
            .expect("重入候选留存失败");
        manager.clear_tracking(camera_id).await;
        {
            let settle = lock_capture_settle(&ctx);
            assert_eq!(settle.pending_bytes(), 0, "清轨必须释放内存候选");
        }
        assert_eq!(
            dir_entry_count(&temp_dir.join(camera_id)),
            2,
            "盘上只应有结算写下的两份正式证据"
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    /// 端到端：识别类算法走过生产入口 `process_detections_for_algo_*`，
    /// 抓拍结算动作必须由真实分析路径产出（而不是只在测试里直接敲状态机），
    /// 且 RetainCandidate 携带的几何必须与当帧检测同源。
    #[tokio::test]
    async fn recognition_settle_actions_flow_through_process_detections() {
        let manager = Arc::new(PipelineManager::new());
        let camera_id = "cam_settle_e2e";
        let algorithm_id = "algo_face_e2e";
        let face_bbox = BoundingBox::new(0.45, 0.35, 0.55, 0.5);
        let object_bbox = BoundingBox::new(0.4, 0.3, 0.6, 0.6);

        let detection = |quality: f32, with_face: bool| Detection {
            class_id: 0,
            label: "person".to_string(),
            confidence: 0.95,
            quality_score: Some(quality),
            bbox: object_bbox,
            face: with_face.then_some(types::FaceDetail {
                bbox: face_bbox,
                confidence: 0.95,
                quality_score: Some(quality),
                fused_count: None,
                template_quality: None,
                template_mature: None,
                embedding: None,
            }),
        };

        // 首帧：挂起 + 产出候选留存（几何取自当帧检测）。
        let outcome = manager
            .process_detections_for_algo(
                camera_id,
                algorithm_id,
                types::AlgorithmKind::Recognition,
                vec![detection(0.80, true)],
                1000,
            )
            .await;
        assert!(outcome.applied);
        assert_eq!(
            outcome.capture_actions.len(),
            1,
            "识别类首帧必须产出候选留存动作"
        );
        let CaptureAction::RetainCandidate(request) = &outcome.capture_actions[0] else {
            panic!(
                "期望 RetainCandidate，实际 {:?}",
                outcome.capture_actions[0]
            );
        };
        assert_eq!(request.geometry.pts_ms, 1000, "候选时标必须等于当帧时标");
        assert_eq!(request.geometry.bbox, object_bbox);
        assert_eq!(request.geometry.face_bbox, Some(face_bbox));
        assert_eq!(request.geometry.quality, 0.80);

        // 瞬时丢脸：宽限期内不得结算，避免烧掉冷却并丢掉峰值候选。
        let transient = manager
            .process_detections_for_algo(
                camera_id,
                algorithm_id,
                types::AlgorithmKind::Recognition,
                vec![detection(0.80, false)],
                1040,
            )
            .await;
        assert!(
            transient.capture_actions.is_empty(),
            "瞬时丢脸不得直接离场结算：{:?}",
            transient.capture_actions
        );

        // 人脸恢复并进入平台期：结算动作必须从同一入口产出。
        let settled = manager
            .process_detections_for_algo(
                camera_id,
                algorithm_id,
                types::AlgorithmKind::Recognition,
                vec![detection(0.78, true)],
                1400,
            )
            .await;
        let settle = settled
            .capture_actions
            .iter()
            .find_map(|action| match action {
                CaptureAction::Settle(request) => Some(request.as_ref()),
                CaptureAction::RetainCandidate(_) => None,
            })
            .expect("平台期必须产出结算动作");
        assert_eq!(
            settle.reason,
            crate::capture_settle::SettleReason::QualityPlateau
        );
        assert_eq!(settle.track_id, outcome.tracked[0].track_id);
        assert!(settle.candidate.is_none(), "未经编码留存时不得伪造候选");
    }

    #[tokio::test]
    async fn test_get_current_tracks() {
        let manager = Arc::new(PipelineManager::new());
        let cam_id = "cam_current_tracks_test";

        assert!(manager.get_current_tracks(cam_id).await.is_empty());

        let det = types::Detection {
            class_id: 0,
            label: "person".to_string(),
            confidence: 0.95,
            quality_score: None,
            bbox: types::BoundingBox::new(0.1, 0.2, 0.3, 0.4),
            face: None,
        };

        let outcome = manager
            .process_detections_for_algo(cam_id, "algo_1", "detection", vec![det], 1000)
            .await;
        assert_eq!(outcome.tracked.len(), 1);

        let current = manager.get_current_tracks(cam_id).await;
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].label, "person");

        // 空帧更新该算法，应自动清除
        manager
            .process_detections_for_algo(cam_id, "algo_1", "detection", vec![], 2000)
            .await;
        assert!(manager.get_current_tracks(cam_id).await.is_empty());

        // 测试 recognition / face_recognition 算法产出 captures 而非 alarms
        let face_det = Detection {
            class_id: 0,
            label: "face".to_string(),
            confidence: 0.95,
            quality_score: Some(0.88),
            bbox: types::BoundingBox::new(0.2, 0.2, 0.4, 0.4),
            face: None,
        };
        let rec_outcome = manager
            .process_detections_for_algo(
                cam_id,
                "face_algo",
                "face_recognition",
                vec![face_det],
                3000,
            )
            .await;
        assert_eq!(
            rec_outcome.capture_actions.len(),
            1,
            "face_recognition 类别算法必须产生峰值候选留存动作"
        );
        assert!(matches!(
            rec_outcome.capture_actions[0],
            crate::capture_settle::CaptureAction::RetainCandidate(_)
        ));
        assert!(
            rec_outcome.alarms.is_empty(),
            "face_recognition 类别算法绝不产生违规告警"
        );
    }

    #[tokio::test]
    async fn test_tracking_generation_and_expiry_clear_state() {
        let manager = Arc::new(PipelineManager::new());
        let cam_id = "cam_tracking_generation_test";
        let mut events = manager.subscribe_analysis_events();
        let det = types::Detection {
            class_id: 0,
            label: "person".to_string(),
            confidence: 0.95,
            quality_score: None,
            bbox: types::BoundingBox::new(0.1, 0.2, 0.3, 0.4),
            face: None,
        };

        let first = manager
            .process_detections_for_algo(cam_id, "algo_1", "detection", vec![det.clone()], 1000)
            .await;
        assert!(first.applied);
        assert_eq!(manager.get_current_tracks(cam_id).await.len(), 1);

        let stale = manager
            .process_detections_for_algo(cam_id, "algo_1", "detection", vec![det.clone()], 900)
            .await;
        assert!(!stale.applied);
        assert_eq!(manager.get_current_tracks(cam_id).await.len(), 1);

        let ctx = manager.get_or_create_context(cam_id).await;
        let old_generation = ctx.tracking_generation.load(Ordering::Acquire);
        manager.clear_tracking(cam_id).await;
        assert!(manager.get_current_tracks(cam_id).await.is_empty());
        match events.recv().await.expect("tracking clear event") {
            PipelineAnalysisEvent::Tracks(event) => assert!(event.tracks.is_empty()),
            other => panic!("unexpected event: {other:?}"),
        }

        let in_flight_old_result = manager
            .process_detections_for_algo_with_embeddings_at_generation(
                cam_id,
                "algo_1",
                "detection",
                vec![det.clone()],
                vec![None],
                1_050,
                old_generation,
            )
            .await;
        assert!(!in_flight_old_result.applied);
        assert!(manager.get_current_tracks(cam_id).await.is_empty());

        let second = manager
            .process_detections_for_algo(cam_id, "algo_1", "detection", vec![det], 1100)
            .await;
        assert!(second.applied);
        assert_eq!(manager.get_current_tracks(cam_id).await.len(), 1);

        assert!(
            manager
                .expire_tracking_for_algo_at(cam_id, "algo_1", 4_101)
                .await
        );
        assert!(manager.get_current_tracks(cam_id).await.is_empty());
        match events.recv().await.expect("tracking expiry event") {
            PipelineAnalysisEvent::Tracks(event) => assert!(event.tracks.is_empty()),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_decoded_frame_update_and_clear_lifecycle() {
        let manager = Arc::new(PipelineManager::new());
        let cam_id = "cam_ring_lifecycle_test";

        let frame = FrameRef::new(
            cam_id.to_string(),
            1000,
            1920,
            1080,
            types::StrideInfo::new(1920, 1080),
            types::PixelFormat::Nv12,
            types::FrameHandle::Host(vec![0u8; 100].into()),
        );

        manager.update_decoded_frame(cam_id, frame.clone()).await;

        let ctx = manager.get_or_create_context(cam_id).await;
        {
            let ring = ctx.decoded_ring.read().await;
            assert_eq!(ring.len(), 1);
            assert_eq!(
                ring.latest_frame()
                    .expect("ring should have latest frame")
                    .timestamp,
                1000
            );
        }
        {
            let fallback = ctx.sub_stream_fallback.read().await;
            assert!(fallback.is_some());
            assert_eq!(
                fallback
                    .as_ref()
                    .expect("fallback frame should exist")
                    .timestamp,
                1000
            );
        }

        manager.set_main_stream_analysis(cam_id, true).await;
        assert!(ctx.is_main_stream_analysis.load(Ordering::Relaxed));

        // 清空环形队列并回收资源
        manager.clear_decoded_ring(cam_id).await;
        {
            let ring = ctx.decoded_ring.read().await;
            assert!(ring.is_empty());
        }
        {
            let fallback = ctx.sub_stream_fallback.read().await;
            assert!(fallback.is_none());
        }

        manager.set_main_stream_analysis(cam_id, false).await;
        assert!(!ctx.is_main_stream_analysis.load(Ordering::Relaxed));
    }
}
