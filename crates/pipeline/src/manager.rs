use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;
use tokio::sync::RwLock as TokioRwLock;

use media::decoder::VideoDecoder;
use media::ring_buffer::{MainStreamRingBuffer, RingBufferConfig};
use types::{AnalysisTask, BoundingBox, Camera, EncodedPacket, FrameRef};

use crate::error::PipelineError;
use crate::snapshot::{SnapshotEngine, SnapshotResult};

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
        let mut dec_guard = ctx.snapshot_decoder.lock().await;
        if dec_guard.is_none() {
            *dec_guard = Some(media::create_decoder(&camera.camera_id, codec));
            tracing::info!(camera_id = %camera.camera_id, ?codec, "已按需初始化硬件解码器实例");
        }

        tracing::info!(camera_id = %camera.camera_id, "分析管线任务已启动/更新");
        Ok(())
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
}
