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
use crate::roi::RoiAffineMapper;
use crate::rules::{RuleEvaluator, TriggeredAlarm};
use crate::snapshot::{SnapshotEngine, SnapshotResult};
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

/// 全局多路视频分析与快照管线调度控制器
#[derive(Debug)]
pub struct PipelineManager {
    tasks: Arc<TokioRwLock<HashMap<String, AnalysisTask>>>,
    pipelines: Arc<TokioRwLock<HashMap<String, Arc<CameraPipelineContext>>>>,
    snapshot_engine: Arc<SnapshotEngine>,
}

impl Default for PipelineManager {
    fn default() -> Self {
        Self::new()
    }
}

impl PipelineManager {
    pub fn new() -> Self {
        Self::with_evidence_dir("var/data/evidence")
    }

    pub fn with_evidence_dir(dir: impl Into<PathBuf>) -> Self {
        Self {
            tasks: Arc::new(TokioRwLock::new(HashMap::new())),
            pipelines: Arc::new(TokioRwLock::new(HashMap::new())),
            snapshot_engine: Arc::new(SnapshotEngine::new(dir)),
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

    /// 触发靶向快拍抽帧与证据图片落地 (细粒度锁隔离与后台异步落盘)
    pub async fn trigger_snapshot(
        &self,
        camera_id: &str,
        target_pts_ms: i64,
        bbox: Option<BoundingBox>,
    ) -> Result<SnapshotResult, PipelineError> {
        let ctx = self.get_or_create_context(camera_id).await;
        let fallback_frame = ctx.sub_stream_fallback.read().await.clone();

        // 仅在解码阶段持有解码器互斥锁，解码完成立刻释放
        let (frame_to_process, is_fallback) = {
            let mut decoder_guard = ctx.snapshot_decoder.lock().await;
            if decoder_guard.is_none() && !ctx.ring_buffer.is_empty() {
                let codec = ctx
                    .ring_buffer
                    .latest_codec()
                    .unwrap_or(types::CodecType::H264);
                *decoder_guard = Some(media::create_decoder(camera_id, codec));
            }

            SnapshotEngine::decode_target_frame(
                camera_id,
                target_pts_ms,
                Some(&ctx.ring_buffer),
                fallback_frame.as_ref(),
                decoder_guard.as_deref_mut(),
            )
            .await?
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

    /// 停止某路摄像头的分析任务
    pub async fn stop_task(&self, camera_id: &str) -> Result<(), PipelineError> {
        let mut tasks = self.tasks.write().await;
        if tasks.remove(camera_id).is_some() {
            self.set_ai_active(camera_id, false).await;

            let pipelines = self.pipelines.read().await;
            if let Some(ctx) = pipelines.get(camera_id) {
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
}
