use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;
use tokio::sync::RwLock as TokioRwLock;

use media::decoder::VideoDecoder;
use media::ring_buffer::{MainStreamRingBuffer, RingBufferConfig};
use types::{
    AnalysisTask, BoundingBox, Camera, Detection, DetectionRule, EncodedPacket, FrameRef,
    TrackedObject,
};

use crate::error::PipelineError;
use crate::pump::{PumpMetrics, SubStreamAnalysisPump, SubStreamPumpConfig};
use crate::roi::RoiAffineMapper;
use crate::rules::{RuleEvaluator, TriggeredAlarm};
use crate::snapshot::{SnapshotConfig, SnapshotEngine, SnapshotResult};
use crate::tracker::SimpleTracker;

/// 单路摄像头管线运行时上下文
pub struct CameraPipelineContext {
    pub camera_id: String,
    /// 主码流高分辨率 NALU 内存环形队列 (2~3.5s GOP)
    pub ring_buffer: Arc<MainStreamRingBuffer>,
    /// 子码流最新解码帧（保底快照候选）
    pub sub_stream_fallback: TokioRwLock<Option<FrameRef>>,
    /// 是否有活跃的 AI 分析规则订阅
    pub ai_active: AtomicBool,
    /// 活跃的实时预览客户端计数
    pub preview_count: AtomicUsize,
    /// 主码流专用的按需快拍解码器实例 (惰性分配)
    pub snapshot_decoder: TokioMutex<Option<Box<dyn VideoDecoder + Send>>>,
    /// 纯 Rust 航迹关联跟踪器
    pub tracker: TokioMutex<SimpleTracker>,
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
            sub_stream_fallback: TokioRwLock::new(None),
            ai_active: AtomicBool::new(false),
            preview_count: AtomicUsize::new(0),
            snapshot_decoder: TokioMutex::new(None),
            tracker: TokioMutex::new(SimpleTracker::new()),
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
    pumps: Arc<TokioRwLock<HashMap<String, SubStreamAnalysisPump>>>,
    snapshot_engine: Arc<SnapshotEngine>,
    snapshot_semaphore: Arc<tokio::sync::Semaphore>,
    permit_timeout_ms: u64,
}

impl Default for PipelineManager {
    fn default() -> Self {
        Self::new()
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
        Self {
            tasks: Arc::new(TokioRwLock::new(HashMap::new())),
            pipelines: Arc::new(TokioRwLock::new(HashMap::new())),
            pumps: Arc::new(TokioRwLock::new(HashMap::new())),
            snapshot_engine: Arc::new(SnapshotEngine::with_config(dir, config)),
            snapshot_semaphore: Arc::new(tokio::sync::Semaphore::new(
                max_concurrent_decoders.max(1),
            )),
            permit_timeout_ms,
        }
    }

    /// 获取快照抓拍引擎句柄
    pub fn snapshot_engine(&self) -> &SnapshotEngine {
        &self.snapshot_engine
    }

    /// 注册或获取某路摄像头的分析管线上下文
    pub async fn get_or_create_context(&self, camera_id: &str) -> Arc<CameraPipelineContext> {
        let mut pipelines = self.pipelines.write().await;
        if let Some(ctx) = pipelines.get(camera_id) {
            return ctx.clone();
        }

        let ctx = Arc::new(CameraPipelineContext::new(camera_id));
        pipelines.insert(camera_id.to_string(), ctx.clone());
        ctx
    }

    /// 向摄像机主码流环形队列压入压缩 NALU 包
    pub async fn push_main_packet(&self, camera_id: &str, packet: Arc<EncodedPacket>) {
        let ctx = self.get_or_create_context(camera_id).await;
        ctx.ring_buffer.push(packet);
    }

    /// 更新子码流最新解码帧 (用作平滑降级快拍源)
    pub async fn update_sub_stream_frame(&self, camera_id: &str, frame: FrameRef) {
        let ctx = self.get_or_create_context(camera_id).await;
        let mut fallback = ctx.sub_stream_fallback.write().await;
        *fallback = Some(frame);
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

            while let Ok(pkt) = packet_rx.recv().await {
                ctx.ring_buffer.push(pkt);
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
        let fallback_frame = ctx.sub_stream_fallback.read().await.clone();

        // 工业级全局 VPU 抓拍通道配额管控：
        // 尝试在限时内获取全局 VPU 硬解信号量许可，若瞬时并发告警超限或排队超时，
        // 自动无缝降级使用子码流当前帧，彻底防止瞬时并发告警打爆硬件 VPU 通道上限！
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
                            fallback_frame.as_ref(),
                            decoder_guard.as_deref_mut(),
                        )
                        .await?
                };
                drop(permit); // 解码完成后显式归还配额
                res
            }
            _ => {
                // 配额满载或获取超时，自适应降级复用子码流帧
                if let Some(fallback) = fallback_frame {
                    tracing::warn!(
                        camera_id = %camera_id,
                        target_pts = target_pts_ms,
                        timeout_ms = self.permit_timeout_ms,
                        "全局 VPU 硬件抓拍解码配额满载或等待超时，自适应无缝降级复用子码流解码帧"
                    );
                    (fallback, true)
                } else {
                    return Err(PipelineError::Snapshot(format!(
                        "全局 VPU 抓拍通道配额耗尽且子码流无有效备用帧 ({camera_id})"
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
        let mut tasks = self.tasks.write().await;
        tasks.insert(camera.camera_id.clone(), task);

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

    /// 统一处理算法推理输出的检测结果：
    /// 1. 执行 Pre-crop ROI 线性仿射坐标还原（将局部归一化 [0,1] 映射至全景大图 [0,1]）；
    /// 2. 纯 Rust 航迹关联跟踪器更新（维护连续全局 TrackID 与历史移动轨迹）；
    /// 3. 统一几何规则引擎判定（Mask 区域静默过滤、ROI 入侵、绊线越界及 5 秒防重复报警冷却）；
    /// 4. 返回当前活跃 TrackedObject 与触发的 TriggeredAlarm 集合。
    pub async fn process_detections(
        &self,
        camera_id: &str,
        detections: Vec<Detection>,
        timestamp_ms: i64,
    ) -> (Vec<TrackedObject>, Vec<TriggeredAlarm>) {
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

        // 2. 航迹关联更新
        let mut tracker = ctx.tracker.lock().await;
        let tracked_objects = tracker.update(global_detections);

        // 3. 几何规则评估与 5 秒告警防刷屏冷却
        let rules = ctx.rules.read().await;
        let alarms =
            ctx.rule_evaluator
                .evaluate(&rules, &tracked_objects, &mut tracker, timestamp_ms, 5000);

        (tracked_objects, alarms)
    }

    async fn mount_pump(&self, camera_id: &str, pump: SubStreamAnalysisPump) {
        let old_pump = {
            let mut pumps = self.pumps.write().await;
            pumps.remove(camera_id)
        };
        if let Some(mut old) = old_pump {
            old.stop().await;
        }
        self.pumps.write().await.insert(camera_id.to_string(), pump);
    }

    /// 启动某路摄像头的子码流分析驱动泵
    pub async fn start_analysis_pump(
        self: &Arc<Self>,
        camera_id: &str,
        session: Arc<media::CameraStreamSession>,
        decoder: Box<dyn VideoDecoder + Send>,
        worker: infer::InferenceWorkerHandle,
        config: SubStreamPumpConfig,
    ) {
        let pump =
            SubStreamAnalysisPump::start(camera_id, session, decoder, worker, self.clone(), config);
        self.mount_pump(camera_id, pump).await;
        tracing::info!(camera_id = %camera_id, "子码流驱动泵已挂载至管线管理器");
    }

    /// 启动某路摄像头的子码流分析驱动泵 (全量托管 InferenceWorker 运行周期)
    pub async fn start_analysis_pump_with_worker(
        self: &Arc<Self>,
        camera_id: &str,
        session: Arc<media::CameraStreamSession>,
        decoder: Box<dyn VideoDecoder + Send>,
        worker: infer::InferenceWorker,
        config: SubStreamPumpConfig,
    ) {
        let pump = SubStreamAnalysisPump::start_with_worker(
            camera_id,
            session,
            decoder,
            worker,
            self.clone(),
            config,
        );
        self.mount_pump(camera_id, pump).await;
        tracing::info!(camera_id = %camera_id, "子码流驱动泵 (含常驻工作线程) 已挂载至管线管理器");
    }

    /// 停止某路摄像头的子码流分析驱动泵
    pub async fn stop_analysis_pump(&self, camera_id: &str) -> bool {
        let old_pump = {
            let mut pumps = self.pumps.write().await;
            pumps.remove(camera_id)
        };
        if let Some(mut pump) = old_pump {
            pump.stop().await;
            tracing::info!(camera_id = %camera_id, "子码流驱动泵已停止并从管理器注销");
            true
        } else {
            false
        }
    }

    /// 停止所有摄像头的分析驱动泵并等待回收
    pub async fn stop_all_pumps(&self) {
        let old_pumps = {
            let mut pumps = self.pumps.write().await;
            pumps.drain().map(|(_, p)| p).collect::<Vec<_>>()
        };
        for mut pump in old_pumps {
            pump.stop().await;
        }
        tracing::info!("已停止所有子码流分析驱动泵并回收资源");
    }

    /// 查询某路摄像头的驱动泵是否正在运行
    pub async fn is_analysis_pump_running(&self, camera_id: &str) -> bool {
        let pumps = self.pumps.read().await;
        pumps
            .get(camera_id)
            .map(|p| p.is_running())
            .unwrap_or(false)
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
        // 配置全局仅允许 1 个并发硬件抓拍通道，借调超时 10ms
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
        let snapshot = manager
            .trigger_snapshot(cam_id, 1000, None)
            .await
            .expect("配额超限自适应降级抓拍应成功");

        assert!(snapshot.is_fallback_sub_stream);
        assert_eq!(snapshot.width, 640);
        assert_eq!(snapshot.height, 360);

        drop(held_permit);
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[tokio::test]
    async fn test_pipeline_manager_pump_lifecycle_and_cascade_stop() {
        use async_trait::async_trait;
        use infer::InferenceBackend;
        use media::decoders::MockDecoder;
        use std::sync::atomic::AtomicBool;
        use tokio::sync::broadcast;
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

        let (broadcast_tx, _) = broadcast::channel(16);
        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        let session = Arc::new(media::CameraStreamSession {
            camera_id: cam_id.to_string(),
            rtsp_url: "rtsp://dummy/sub".to_string(),
            transport_policy: TransportPolicy::Tcp,
            active_viewers: Arc::new(std::sync::atomic::AtomicUsize::new(1)),
            ai_task_enabled: Arc::new(AtomicBool::new(true)),
            keyframe_cache: Arc::new(tokio::sync::RwLock::new(
                media::stream_hub::KeyframeCache::default(),
            )),
            broadcast_tx,
            cancel_signal: Arc::new(AtomicBool::new(false)),
            cancel_tx,
            cancel_rx,
            ingestor_running: Arc::new(AtomicBool::new(true)),
            last_packet_time: Arc::new(std::sync::atomic::AtomicI64::new(1000)),
            cooldown_cancel: Arc::new(tokio::sync::Mutex::new(None)),
            consecutive_probe_failures: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        });

        let decoder = Box::new(MockDecoder::new(cam_id, CodecType::H264, 640, 360));
        let worker = infer::InferenceWorker::new(Arc::new(DummyInferBackend));

        assert!(!manager.is_analysis_pump_running(cam_id).await);

        manager
            .start_analysis_pump(
                cam_id,
                session,
                decoder,
                worker.handle(),
                SubStreamPumpConfig::default(),
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
        use std::sync::atomic::AtomicBool;
        use tokio::sync::broadcast;
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
            let (broadcast_tx, _) = broadcast::channel(16);
            let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
            let session = Arc::new(media::CameraStreamSession {
                camera_id: cam_id.clone(),
                rtsp_url: "rtsp://dummy/sub".to_string(),
                transport_policy: TransportPolicy::Tcp,
                active_viewers: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
                ai_task_enabled: Arc::new(AtomicBool::new(false)),
                keyframe_cache: Arc::new(tokio::sync::RwLock::new(
                    media::stream_hub::KeyframeCache::default(),
                )),
                broadcast_tx,
                cancel_signal: Arc::new(AtomicBool::new(false)),
                cancel_tx,
                cancel_rx,
                ingestor_running: Arc::new(AtomicBool::new(true)),
                last_packet_time: Arc::new(std::sync::atomic::AtomicI64::new(1000)),
                cooldown_cancel: Arc::new(tokio::sync::Mutex::new(None)),
                consecutive_probe_failures: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            });

            let decoder = Box::new(MockDecoder::new(&cam_id, CodecType::H264, 640, 360));
            let worker = infer::InferenceWorker::new(Arc::new(DummyInfer));

            manager
                .start_analysis_pump(
                    &cam_id,
                    session,
                    decoder,
                    worker.handle(),
                    SubStreamPumpConfig::default(),
                )
                .await;

            assert!(manager.is_analysis_pump_running(&cam_id).await);
        }

        // 停止所有驱动泵
        manager.stop_all_pumps().await;

        assert!(!manager.is_analysis_pump_running("cam_all_1").await);
        assert!(!manager.is_analysis_pump_running("cam_all_2").await);
    }
}
