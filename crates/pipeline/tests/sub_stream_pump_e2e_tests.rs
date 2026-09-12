//! 子码流驱动泵与常驻推理工作线程端到端闭环集成测试
#![allow(clippy::unwrap_used)]

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
#[cfg(target_os = "macos")]
use infer::package::AlgoPackage;
use infer::{InferenceBackend, InferenceWorker};
#[cfg(target_os = "macos")]
use media::decoder::DecodeDeliveryPolicy;
use media::decoder::VideoDecoder;
use media::decoders::MockDecoder;
use media::stream_hub::CameraStreamSession;
use pipeline::{PipelineManager, SubStreamPumpConfig};
use types::{
    BoundingBox, CodecType, Detection, DetectionLineDirection, DetectionPoint, DetectionRule,
    DetectionRuleRole, EncodedPacket, FrameRef, TransportPolicy,
};

/// 测试专用模拟推理后端
#[derive(Debug)]
struct E2eMockInferBackend {
    current_y: std::sync::Mutex<f32>,
}

#[async_trait]
impl InferenceBackend for E2eMockInferBackend {
    fn name(&self) -> &'static str {
        "E2eMockInferBackend"
    }

    async fn detect(&self, _frame: &FrameRef) -> Result<Vec<Detection>, infer::InferError> {
        let y = *self.current_y.lock().unwrap();
        Ok(vec![Detection {
            class_id: 0,
            label: "person".to_string(),
            confidence: 0.98,
            // 构造宽度 0.2，高度 0.3，底中心为 (0.5, y) 的目标框
            bbox: BoundingBox::new(0.4, y - 0.3, 0.6, y),
        }])
    }
}

#[tokio::test]
async fn test_sub_stream_pump_and_inference_worker_e2e_lifecycle() {
    let temp_dir =
        std::env::temp_dir().join(format!("test_pump_e2e_{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let manager = Arc::new(PipelineManager::with_evidence_dir(&temp_dir));
    let cam_id = "camera_pump_e2e";

    // 1. 设置空间几何绊线布防规则 (Y = 0.5 水平线，A->B 即由上至下单向跨越)
    let rules = vec![DetectionRule {
        role: DetectionRuleRole::Line,
        line_direction: DetectionLineDirection::AToB,
        points: vec![DetectionPoint::new(0.0, 0.5), DetectionPoint::new(1.0, 0.5)],
    }];
    manager.set_camera_rules(cam_id, rules).await;

    // 2. 初始化模拟子码流会话
    let session = CameraStreamSession::mock(cam_id, "rtsp://mock-sub/live", TransportPolicy::Tcp);
    let broadcast_tx = session.broadcast_tx.clone();

    // 3. 构建专用常驻推理线程与解码器
    let infer_backend = Arc::new(E2eMockInferBackend {
        current_y: std::sync::Mutex::new(0.45), // 初始位置底中心在 Y = 0.45 (绊线 Y=0.5 上方)
    });
    let worker = InferenceWorker::new(infer_backend.clone());
    let decoder: Box<dyn VideoDecoder + Send> =
        Box::new(MockDecoder::new(cam_id, CodecType::H264, 640, 360));

    // 4. 挂载并启动子码流驱动泵
    let config = SubStreamPumpConfig {
        target_fps: 25, // 全采样
        motion_gate_enabled: false,
    };
    manager
        .start_analysis_pump(cam_id, session.clone(), decoder, worker.handle(), config)
        .await;

    assert!(manager.is_analysis_pump_running(cam_id).await);
    assert!(
        session.ai_task_enabled.load(Ordering::Relaxed),
        "挂载驱动泵后，StreamHub 会话 AI 活跃标记必须自动置为 true"
    );

    // 5. 模拟发送第 1 帧编码视频包 (目标在 Y = 0.40)
    let pkt1 = Arc::new(EncodedPacket {
        pts_ms: 1000,
        is_keyframe: true,
        codec: CodecType::H264,
        payload: Bytes::from_static(&[0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1f]),
        ..Default::default()
    });
    let _ = broadcast_tx.send(pkt1);

    // 等待异步处理完成
    tokio::time::sleep(Duration::from_millis(50)).await;

    // 验证 sub_stream_fallback 已经更新
    let ctx = manager.get_or_create_context(cam_id).await;
    {
        let fallback = ctx.sub_stream_fallback.read().await;
        assert!(fallback.is_some(), "解码后必须实时刷新子码流保底帧");
        assert_eq!(fallback.as_ref().unwrap().timestamp, 1000);
    }

    // 6. 模拟发送音频包，验证驱动泵解码循环严格忽略音频包，不污染视频解码器
    let audio_pkt = Arc::new(EncodedPacket {
        pts_ms: 1020,
        is_keyframe: false,
        codec: CodecType::Aac,
        payload: Bytes::from_static(&[0xFF, 0xF1, 0x50, 0x80, 0x00, 0x09, 0x00, 0xAA, 0xBB]),
        stream_tag: types::StreamTag::Audio,
    });
    let _ = broadcast_tx.send(audio_pkt);

    // 移动目标至 Y = 0.55 (跨越 Y = 0.5 绊线，且与前一帧维持 IoU=0.5 关联)，发送第 2 帧
    *infer_backend.current_y.lock().unwrap() = 0.55;
    let pkt2 = Arc::new(EncodedPacket {
        pts_ms: 1040,
        is_keyframe: false,
        codec: CodecType::H264,
        payload: Bytes::from_static(&[0x00, 0x00, 0x00, 0x01, 0x41, 0x9a]),
        ..Default::default()
    });
    let _ = broadcast_tx.send(pkt2);

    // 等待跨帧轨迹关联与规则判定及快照落盘完成
    tokio::time::sleep(Duration::from_millis(300)).await;

    // 7. 验证驱动泵运行监控指标
    let metrics = manager
        .get_analysis_pump_metrics(cam_id)
        .await
        .expect("驱动泵指标必须可读取");

    assert_eq!(
        metrics.packets_received.load(Ordering::Relaxed),
        2,
        "必须累计接收 2 个包"
    );
    assert_eq!(
        metrics.frames_decoded.load(Ordering::Relaxed),
        2,
        "必须成功解码 2 帧"
    );
    assert_eq!(
        metrics.frames_inferred.load(Ordering::Relaxed),
        2,
        "必须成功完成 2 帧常驻推理"
    );
    assert_eq!(
        metrics.alarms_triggered.load(Ordering::Relaxed),
        1,
        "跨越绊线必须触发 1 次业务告警"
    );
    assert_eq!(
        metrics.snapshots_saved.load(Ordering::Relaxed),
        1,
        "告警必须自动闭环触发 1 次靶向快照落地"
    );

    // 验证 evidence 目录下确实落地了快照文件
    let cam_evidence_dir = temp_dir.join(cam_id);
    assert!(cam_evidence_dir.exists(), "必须生成摄像头专属快照证据目录");
    let files: Vec<_> = std::fs::read_dir(&cam_evidence_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert!(!files.is_empty(), "证据目录下必须真实落地快照图片");

    // 8. 停止驱动泵并验证资源释放
    let stopped = manager.stop_analysis_pump(cam_id).await;
    assert!(stopped, "必须成功停止驱动泵");
    assert!(!manager.is_analysis_pump_running(cam_id).await);
    assert!(
        !session.ai_task_enabled.load(Ordering::Relaxed),
        "停止驱动泵后，StreamHub 会话 AI 活跃标记必须重置为 false"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_sub_stream_pump_managed_worker_lifecycle() {
    let temp_dir = std::env::temp_dir().join(format!(
        "test_pump_managed_worker_{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let manager = Arc::new(PipelineManager::with_evidence_dir(&temp_dir));
    let cam_id = "cam_managed_worker_test";

    let session = CameraStreamSession::mock(cam_id, "rtsp://mock-sub/live", TransportPolicy::Tcp);

    let infer_backend = Arc::new(E2eMockInferBackend {
        current_y: std::sync::Mutex::new(0.45),
    });
    let worker = InferenceWorker::new(infer_backend);
    let worker_handle = worker.handle();
    let decoder: Box<dyn VideoDecoder + Send> =
        Box::new(MockDecoder::new(cam_id, CodecType::H264, 640, 360));

    // 使用 start_analysis_pump_with_worker 托管整个工作线程
    manager
        .start_analysis_pump_with_worker(
            cam_id,
            session.clone(),
            decoder,
            worker,
            SubStreamPumpConfig::default(),
        )
        .await;

    assert!(manager.is_analysis_pump_running(cam_id).await);
    assert!(worker_handle.is_alive());

    // 停止驱动泵
    let stopped = manager.stop_analysis_pump(cam_id).await;
    assert!(stopped);
    assert!(!manager.is_analysis_pump_running(cam_id).await);

    // 验证常驻推理工作线程已随之优雅停止
    assert!(
        !worker_handle.is_alive(),
        "驱动泵注销后，托管的常驻推理线程必须同步退出"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[cfg(target_os = "macos")]
struct MockPixelBufferDecoder {
    camera_id: String,
    width: u32,
    height: u32,
    pixel_buf: infer::c_abi::cvpixelbuffer::NativePixelBuffer,
    policy: DecodeDeliveryPolicy,
}

#[cfg(target_os = "macos")]
#[async_trait]
impl VideoDecoder for MockPixelBufferDecoder {
    async fn decode_packet(
        &mut self,
        _packet: &[u8],
        pts: i64,
    ) -> Result<Option<FrameRef>, media::error::MediaError> {
        let (ys, _) = self.pixel_buf.strides();
        let handle = self
            .pixel_buf
            .to_frame_handle()
            .expect("pixel_buf 必须有效");
        let frame = FrameRef::new(
            self.camera_id.clone(),
            pts,
            self.width,
            self.height,
            types::StrideInfo::new(ys as u32, self.height),
            types::PixelFormat::Nv12,
            handle,
        );
        Ok(Some(frame))
    }

    async fn flush(&mut self) -> Result<Vec<FrameRef>, media::error::MediaError> {
        Ok(Vec::new())
    }

    fn set_delivery_policy(&mut self, policy: DecodeDeliveryPolicy) {
        self.policy = policy;
    }

    fn delivery_policy(&self) -> DecodeDeliveryPolicy {
        self.policy
    }
}

#[cfg(target_os = "macos")]
#[tokio::test]
async fn test_sub_stream_pump_with_real_macos_algo_package_e2e() {
    let candidates = [
        std::path::Path::new("algo-packages/macos-arm64/general_detection"),
        std::path::Path::new("../../algo-packages/macos-arm64/general_detection"),
    ];
    let Some(pkg_path) = candidates.into_iter().find(|p| p.exists()) else {
        return;
    };

    let temp_dir = std::env::temp_dir().join(format!(
        "test_real_macos_pump_e2e_{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let manager = Arc::new(PipelineManager::with_evidence_dir(&temp_dir));
    let cam_id = "camera_real_macos_e2e";

    // 1. 加载真实算法包并创建实例
    let pkg = Arc::new(AlgoPackage::load_and_verify(pkg_path, false).expect("加载算法包失败"));
    let valid_cfg =
        r#"{"confidence_threshold":0.35,"iou_threshold":0.45,"target_classes":["person","car"]}"#;

    let inst = pkg
        .create_instance(cam_id, Some(valid_cfg))
        .expect("创建算法实例失败");
    let backend: Arc<dyn InferenceBackend> = Arc::new(inst);

    // 2. 准备自测图并构建 NativePixelBuffer
    let testimage_path = pkg_path.join("testimage.jpg");
    let img = image::open(&testimage_path).expect("读取 testimage.jpg 失败");
    let rgb = img.to_rgb8();
    let (width, height) = rgb.dimensions();
    let rgb_raw = rgb.into_raw();
    let pixel_buf =
        infer::c_abi::cvpixelbuffer::NativePixelBuffer::from_rgb_to_nv12(&rgb_raw, width, height)
            .expect("转换 CVPixelBuffer 失败");

    let decoder: Box<dyn VideoDecoder + Send> = Box::new(MockPixelBufferDecoder {
        camera_id: cam_id.to_string(),
        width: width & !1,
        height: height & !1,
        pixel_buf,
        policy: DecodeDeliveryPolicy::LosslessBackpressure,
    });

    // 3. 构造常驻 OS 线程 InferenceWorker
    let worker = InferenceWorker::new(backend);

    // 4. 初始化模拟子码流会话
    let session = CameraStreamSession::mock(cam_id, "rtsp://mock-real/live", TransportPolicy::Tcp);
    let broadcast_tx = session.broadcast_tx.clone();

    // 5. 挂载子码流驱动泵并启动
    manager
        .start_analysis_pump(
            cam_id,
            session.clone(),
            decoder,
            worker.handle(),
            SubStreamPumpConfig::default(),
        )
        .await;

    assert!(manager.is_analysis_pump_running(cam_id).await);

    // 6. 发送一帧数据驱动全链路
    let pkt = Arc::new(EncodedPacket {
        pts_ms: 1000,
        is_keyframe: true,
        codec: CodecType::H264,
        payload: Bytes::from_static(&[0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1f]),
        ..Default::default()
    });
    let _ = broadcast_tx.send(pkt);

    // 等待异步流转完成（CoreML 首次前向推理包含模型预热，等待 500ms）
    tokio::time::sleep(Duration::from_millis(500)).await;

    // 7. 校验监控指标
    let metrics = manager
        .get_analysis_pump_metrics(cam_id)
        .await
        .expect("必须能读取指标");
    assert_eq!(metrics.packets_received.load(Ordering::Relaxed), 1);
    assert_eq!(metrics.frames_decoded.load(Ordering::Relaxed), 1);
    assert_eq!(metrics.frames_inferred.load(Ordering::Relaxed), 1);

    // 8. 优雅停止
    manager.stop_analysis_pump(cam_id).await;
    assert!(!manager.is_analysis_pump_running(cam_id).await);

    let _ = std::fs::remove_dir_all(&temp_dir);
}
