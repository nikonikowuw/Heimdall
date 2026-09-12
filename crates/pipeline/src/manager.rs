use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Mutex as TokioMutex;
use tokio::sync::RwLock as TokioRwLock;

use infer::{AlgoPackage, InferenceWorker};
use media::decoder::VideoDecoder;
use media::ring_buffer::{MainStreamRingBuffer, RingBufferConfig};
use types::{
    AnalysisTask, BoundingBox, Camera, Detection, DetectionRule, EncodedPacket, FrameRef,
    TrackedObject,
};

use crate::decoded_ring::DecodedFrameRingBuffer;
use crate::error::PipelineError;
use crate::events::{
    PipelineAlarmEvent, PipelineAnalysisEvent, PipelineCaptureEvent,
    DEFAULT_ANALYSIS_EVENT_CHANNEL_CAPACITY,
};
use crate::pump::{AnalysisPump, AnalysisPumpConfig, PumpMetrics};
use crate::roi::RoiAffineMapper;
use crate::rules::{RuleEvaluator, TriggeredAlarm};
use crate::snapshot::{SnapshotConfig, SnapshotEngine, SnapshotResult};
use crate::tracker::SimpleTracker;

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
    /// 主码流专用的按需快拍解码器实例 (惰性分配)
    pub snapshot_decoder: TokioMutex<Option<Box<dyn VideoDecoder + Send>>>,
    /// 纯 Rust 航迹关联跟踪器 (兼容单算法入口)
    pub tracker: TokioMutex<SimpleTracker>,
    /// 多算法独立航迹关联跟踪器映射表 (algorithm_id -> SimpleTracker)
    pub trackers: TokioMutex<HashMap<String, SimpleTracker>>,
    /// 每路摄像头按算法实例维护的最新活跃航迹快照 (algorithm_id -> Vec<TrackedObject>)
    pub current_tracks: TokioRwLock<HashMap<String, Vec<TrackedObject>>>,
    /// 局部特写预裁剪仿射变换映射器
    pub roi_mapper: TokioRwLock<RoiAffineMapper>,
    /// 任务级空间几何规则
    pub rules: TokioRwLock<Vec<DetectionRule>>,
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
            snapshot_decoder: TokioMutex::new(None),
            tracker: TokioMutex::new(SimpleTracker::new()),
            trackers: TokioMutex::new(HashMap::new()),
            current_tracks: TokioRwLock::new(HashMap::new()),
            roi_mapper: TokioRwLock::new(RoiAffineMapper::identity()),
            rules: TokioRwLock::new(Vec::new()),
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
    /// 当前活跃航迹对象
    pub tracked: Vec<TrackedObject>,
    /// 触发的违规告警集合 (针对 detection 类防范算法)
    pub alarms: Vec<TriggeredAlarm>,
    /// 触发的客观通行抓拍目标 (针对 recognition 类识别算法)
    pub captures: Vec<TrackedObject>,
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
            PipelineAnalysisEvent::Tracks(_) => {}
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
        if let Some(ctx) = pipelines.get(camera_id) {
            return ctx.clone();
        }

        let ctx = Arc::new(CameraPipelineContext::new(camera_id));
        pipelines.insert(camera_id.to_string(), ctx.clone());
        ctx
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

    /// 向摄像机主码流环形队列压入压缩 NALU 包
    pub async fn push_main_packet(&self, camera_id: &str, packet: Arc<EncodedPacket>) {
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

    /// 挂载主码流广播通道，持续将压缩 NALU 包压入 RingBuffer
    pub fn attach_main_stream(
        &self,
        camera_id: &str,
        mut packet_rx: tokio::sync::broadcast::Receiver<Arc<EncodedPacket>>,
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
                match packet_rx.recv().await {
                    Ok(pkt) => {
                        if awaiting_keyframe && !pkt.is_keyframe {
                            continue;
                        }
                        awaiting_keyframe = false;
                        ctx.ring_buffer.push(pkt);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        // 丢包后旧 GOP 已不再可靠，清空并等待新的关键帧恢复证据链。
                        ctx.ring_buffer.clear();
                        awaiting_keyframe = true;
                        tracing::warn!(
                            camera_id = %camera_id,
                            skipped,
                            "主码流 RingBuffer attach 发生 Lagged，清空残缺 GOP 并等待新关键帧"
                        );
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        tracing::warn!(camera_id = %camera_id, "主码流广播已关闭，RingBuffer attach 退出");
                        break;
                    }
                }
            }
        })
    }

    /// 触发靶向快拍抽帧与证据图片落地 (全局有界 VPU 通道配额与细粒度锁隔离)
    pub async fn trigger_snapshot(
        &self,
        camera_id: &str,
        target_pts_ms: i64,
        bbox: Option<BoundingBox>,
    ) -> Result<SnapshotResult, PipelineError> {
        let ctx = self.get_or_create_context(camera_id).await;
        let is_main_stream = ctx.is_main_stream_analysis.load(Ordering::Acquire);

        // 【方案三：零解码瞬时直通】
        // 若当前摄像头采用主码流分析模式，分析泵已在常驻解码 1080P/4K 高清主码流！
        // 优先在 decoded_ring 中以时标检索目标帧（容差对齐环形缓冲的时间窗口上限，覆盖推理全过程），
        // 命中即可零解码直通，杜绝大 GOP 从 I 帧重新追解 98+ 包造成的 VPU 争抢与算力雪崩！
        if is_main_stream {
            let matched_opt = {
                let ring = ctx.decoded_ring.read().await;
                ring.find_by_pts(target_pts_ms, ring.window_duration_ms())
            };

            if let Some((frame, diff_ms)) = matched_opt {
                tracing::debug!(
                    camera_id = %camera_id,
                    target_pts = target_pts_ms,
                    frame_pts = frame.timestamp,
                    diff_ms,
                    width = frame.width,
                    height = frame.height,
                    "零解码直通命中，复用主码流常驻解码高保真帧"
                );
                return self
                    .snapshot_engine
                    .save_snapshot_async(camera_id, frame, bbox, false)
                    .await;
            }
        }

        // 次选与保底帧获取：优先在已解码环中查找最接近目标时标的候选帧。
        // 元组中的布尔值记录候选帧是否来自子码流，避免主流 decoded_ring 帧被误标为子流降级。
        let fallback_frame = {
            let ring = ctx.decoded_ring.read().await;
            ring.find_by_pts(target_pts_ms, 500)
                .map(|(f, _)| (f, !is_main_stream))
                .or_else(|| ring.latest_frame().map(|f| (f, !is_main_stream)))
        };
        let fallback_frame = match fallback_frame {
            Some(f) => Some(f),
            None => ctx
                .sub_stream_fallback
                .read()
                .await
                .clone()
                .map(|frame| (frame, !is_main_stream)),
        };

        // 工业级全局 VPU 抓拍通道配额管控：
        // 尝试在限时内获取全局 VPU 硬解信号量许可，若瞬时并发告警超限或排队超时，
        // 自动无缝降级使用已解码备用帧，彻底防止瞬时并发告警打爆硬件 VPU 通道上限！
        let permit_res = tokio::time::timeout(
            std::time::Duration::from_millis(self.permit_timeout_ms),
            self.snapshot_semaphore.acquire(),
        )
        .await;

        let (frame_to_process, is_fallback) = match permit_res {
            Ok(Ok(permit)) => {
                // 成功获得硬件解码配额通道，仅在解码阶段持有解码器互斥锁，解码完成立刻释放
                let res = {
                    let mut decoder_guard = ctx.snapshot_decoder.lock().await;
                    if decoder_guard.is_none() && !ctx.ring_buffer.is_empty() {
                        let codec = ctx
                            .ring_buffer
                            .latest_codec()
                            .unwrap_or(types::CodecType::H264);
                        *decoder_guard = Some(media::create_decoder(camera_id, codec));
                    }

                    self.snapshot_engine
                        .decode_frame(
                            camera_id,
                            target_pts_ms,
                            Some(&ctx.ring_buffer),
                            fallback_frame.as_ref().map(|(frame, _)| frame),
                            decoder_guard.as_deref_mut(),
                        )
                        .await?
                };
                drop(permit); // 解码完成后显式归还配额
                let (frame, used_fallback) = res;
                let is_sub_fallback = if used_fallback {
                    fallback_frame
                        .as_ref()
                        .map(|(_, is_sub_stream)| *is_sub_stream)
                        .unwrap_or(true)
                } else {
                    false
                };
                (frame, is_sub_fallback)
            }
            _ => {
                // 配额满载或获取超时，自适应降级复用已解码候选帧
                if let Some((fallback, is_sub_stream)) = fallback_frame {
                    tracing::warn!(
                        camera_id = %camera_id,
                        target_pts = target_pts_ms,
                        timeout_ms = self.permit_timeout_ms,
                        "全局 VPU 硬件抓拍解码配额满载或等待超时，自适应无缝复用已解码候选帧"
                    );
                    (fallback, is_sub_stream)
                } else {
                    return Err(PipelineError::Snapshot(format!(
                        "全局 VPU 抓拍通道配额耗尽且无有效备用帧 ({camera_id})"
                    )));
                }
            }
        };

        // 将色彩转换、抠图裁切与 JPEG 写盘卸载至专用 blocking 线程池
        self.snapshot_engine
            .save_snapshot_async(camera_id, frame_to_process, bbox, is_fallback)
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
        let ctx = self.get_or_create_context(camera_id).await;

        // 1. 局部仿射映射至全景坐标系
        let mapper = *ctx.roi_mapper.read().await;
        let global_detections: Vec<Detection> = detections
            .into_iter()
            .map(|mut det| {
                det.bbox = mapper.map_bbox(&det.bbox);
                det
            })
            .collect();

        // 2. 独立算法实例的航迹关联更新
        let mut trackers = ctx.trackers.lock().await;
        let tracker = trackers
            .entry(algorithm_id.to_string())
            .or_insert_with(SimpleTracker::new);
        let tracked_objects = tracker.update(global_detections);

        // 同步更新最新航迹快照 (PRD R1.1: 维护活跃航迹快照)
        {
            let mut current = ctx.current_tracks.write().await;
            if tracked_objects.is_empty() {
                current.remove(algorithm_id);
            } else {
                current.insert(algorithm_id.to_string(), tracked_objects.clone());
            }
        }

        // 3. 根据 algorithm_kind 区分责任流向
        let rules = ctx.rules.read().await;
        let kind = algorithm_kind.into();

        let (alarms, captures) = if kind.is_recognition() {
            let captures = ctx.rule_evaluator.evaluate_captures(
                &rules,
                &tracked_objects,
                tracker,
                timestamp_ms,
                5000,
            );
            (Vec::new(), captures)
        } else {
            let alarms =
                ctx.rule_evaluator
                    .evaluate(&rules, &tracked_objects, tracker, timestamp_ms, 5000);
            (alarms, Vec::new())
        };

        AnalysisOutcome {
            tracked: tracked_objects,
            alarms,
            captures,
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
        motion_gate_enabled: bool,
    ) {
        let pump = AnalysisPump::start_multi_worker(
            camera_id,
            session,
            decoder,
            instance_configs,
            workers,
            self.clone(),
            motion_gate_enabled,
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
        if let Some(mut pump) = old_pump {
            pump.stop().await;
            self.clear_decoded_ring(camera_id).await;
            tracing::info!(camera_id = %camera_id, "分析码流驱动泵已停止并从管理器注销");
            true
        } else {
            false
        }
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
            let worker_result = tokio::task::spawn_blocking(move || {
                let instance = package.create_instance(&instance_id, config_json.as_deref())?;
                Ok::<_, infer::InferError>(InferenceWorker::new(Arc::new(instance)))
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
    use bytes::Bytes;
    use types::{CodecType, FrameHandle, PixelFormat, StrideInfo};

    #[tokio::test]
    async fn test_pipeline_manager_on_demand_and_snapshot() {
        let temp_dir = std::env::temp_dir().join(format!(
            "test_pipe_evidence_{}",
            uuid::Uuid::new_v4().simple()
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

        assert!(snapshot.is_fallback_sub_stream);
        assert_eq!(snapshot.width, 640);
        assert_eq!(snapshot.height, 360);

        // 清理测试目录
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_on_demand_decoder_lifecycle() {
        let temp_dir =
            std::env::temp_dir().join(format!("test_lifecycle_{}", uuid::Uuid::new_v4().simple()));
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
            std::env::temp_dir().join(format!("test_eval_{}", uuid::Uuid::new_v4().simple()));
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
            bbox: BoundingBox::new(0.2, 0.2, 0.4, 0.4),
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
            bbox: BoundingBox::new(0.21, 0.21, 0.41, 0.41),
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
            std::env::temp_dir().join(format!("test_vpu_limit_{}", uuid::Uuid::new_v4().simple()));
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

        assert!(snapshot.is_fallback_sub_stream);
        assert_eq!(snapshot.width, 640);
        assert_eq!(snapshot.height, 360);

        drop(held_permit);
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_main_decoded_ring_fallback_is_not_marked_as_sub_stream() {
        let temp_dir = std::env::temp_dir().join(format!(
            "test_main_ring_fallback_{}",
            uuid::Uuid::new_v4().simple()
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

        // 目标偏差超过精确匹配容差，配额占满后复用主流环中的最近帧。
        let snapshot = manager
            .trigger_snapshot(
                cam_id,
                2000,
                Some(types::BoundingBox::new(0.1, 0.1, 0.3, 0.3)),
            )
            .await
            .expect("主流环帧复用应成功");

        assert!(!snapshot.is_fallback_sub_stream);
        assert_eq!(snapshot.width, 1920);
        assert_eq!(snapshot.height, 1080);

        drop(held_permit);
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
        #[async_trait]
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
        #[async_trait]
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

        // 创建较小容量广播通道以便测试 Lagged
        let (tx, rx) = tokio::sync::broadcast::channel(2);
        let attach_handle = manager.attach_main_stream(cam_id, rx);

        let make_pkt = |pts: i64, key: bool| {
            Arc::new(EncodedPacket {
                pts_ms: pts,
                is_keyframe: key,
                codec: CodecType::H264,
                payload: Bytes::from_static(b"\x00\x00\x00\x01\x65idr"),
            })
        };

        // 1. 发送第 1 包并等待进入 RingBuffer
        let _ = tx.send(make_pkt(1000, true));
        tokio::time::sleep(Duration::from_millis(50)).await;
        let ctx = manager
            .get_pipeline_context(cam_id)
            .await
            .expect("context must exist");
        assert_eq!(ctx.ring_buffer.len(), 1);

        // 2. 连续发送超过容量，制造 Lagged 溢出
        for i in 1..=5 {
            let _ = tx.send(make_pkt(1000 + i * 40, false));
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(ctx.ring_buffer.is_empty(), "关键帧恢复前不得写入残缺 GOP");

        // 3. 再次发送新的完整关键帧，RingBuffer 在清空残缺 GOP 后应正常恢复接收
        let _ = tx.send(make_pkt(2000, true));
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
                bbox: BoundingBox::new(0.1, 0.1, 0.4, 0.4),
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
                bbox: types::BoundingBox::new(0.2, 0.2, 0.5, 0.5),
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

    #[tokio::test]
    async fn test_get_current_tracks() {
        let manager = Arc::new(PipelineManager::new());
        let cam_id = "cam_current_tracks_test";

        assert!(manager.get_current_tracks(cam_id).await.is_empty());

        let det = types::Detection {
            class_id: 0,
            label: "person".to_string(),
            confidence: 0.95,
            bbox: types::BoundingBox::new(0.1, 0.2, 0.3, 0.4),
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
            bbox: types::BoundingBox::new(0.2, 0.2, 0.4, 0.4),
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
            rec_outcome.captures.len(),
            1,
            "face_recognition 类别算法必须产生抓拍凭证"
        );
        assert!(
            rec_outcome.alarms.is_empty(),
            "face_recognition 类别算法绝不产生违规告警"
        );
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
