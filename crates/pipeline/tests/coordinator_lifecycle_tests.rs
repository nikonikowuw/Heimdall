//! 分析任务运行时协调器生命周期集成测试
#![allow(clippy::unwrap_used)]

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use infer::package::AlgoRegistry;
use infer::{InferenceBackend, InferenceWorker};
use media::decoder::VideoDecoder;
use media::decoders::MockDecoder;
use media::stream_hub::StreamHub;
use pipeline::{
    InstanceLaunchConfig, PipelineAnalysisEvent, PipelineManager, StartCameraPipelineParams,
    TaskRuntimeCoordinator,
};
use types::{
    BoundingBox, CodecType, Detection, DetectionLineDirection, DetectionPoint, DetectionRule,
    DetectionRuleRole, EncodedPacket, FrameRef, TransportPolicy,
};

/// 具备位置移动能力的测试模拟推理后端
#[derive(Debug)]
struct MockInferBackend {
    current_y: std::sync::Mutex<f32>,
}

impl MockInferBackend {
    fn new(initial_y: f32) -> Self {
        Self {
            current_y: std::sync::Mutex::new(initial_y),
        }
    }

    fn set_y(&self, y: f32) {
        let mut guard = self.current_y.lock().unwrap();
        *guard = y;
    }
}

#[async_trait]
impl InferenceBackend for MockInferBackend {
    fn name(&self) -> &'static str {
        "MockInferBackend"
    }

    async fn detect(&self, _frame: &FrameRef) -> Result<Vec<Detection>, infer::InferError> {
        let y = *self.current_y.lock().unwrap();
        // 目标高度为 0.3，底中心为 (0.5, y)
        Ok(vec![Detection {
            class_id: 0,
            label: "person".to_string(),
            confidence: 0.95,
            bbox: BoundingBox::new(0.4, y - 0.3, 0.6, y),
        }])
    }
}

/// 辅助生成压缩视频包
fn create_packet(pts_ms: i64, is_keyframe: bool) -> Arc<EncodedPacket> {
    let mut payload = vec![0x00, 0x00, 0x00, 0x01];
    if is_keyframe {
        payload.extend_from_slice(&[0x67, 0x42, 0x00, 0x1f]); // 模拟 SPS/IDR
    } else {
        payload.extend_from_slice(&[0x41, 0x9a]); // 模拟 P 帧
    }
    Arc::new(EncodedPacket {
        pts_ms,
        is_keyframe,
        codec: CodecType::H264,
        payload: Bytes::from(payload),
        ..Default::default()
    })
}

#[tokio::test]
async fn test_coordinator_full_lifecycle_and_events() {
    let temp_dir = std::env::temp_dir().join(format!(
        "test_coord_lifecycle_{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let pipeline_mgr = Arc::new(PipelineManager::with_evidence_dir(&temp_dir));
    let stream_hub = Arc::new(StreamHub::new());
    let algo_registry = Arc::new(AlgoRegistry::new());

    let coordinator =
        TaskRuntimeCoordinator::new(pipeline_mgr.clone(), stream_hub.clone(), algo_registry);

    let cam_id = "camera_coord_001";
    let main_url = "rtsp://mock-main/live";
    let sub_url = "rtsp://mock-sub/live";

    // 预先向 StreamHub 注册会话以供模拟数据流推送
    let main_session = stream_hub
        .get_or_create_session(&format!("{}:main", cam_id), main_url, TransportPolicy::Tcp)
        .await;
    let sub_session = stream_hub
        .get_or_create_session(&format!("{}:sub", cam_id), sub_url, TransportPolicy::Tcp)
        .await;

    // 订阅分析事件通道
    let mut event_rx = pipeline_mgr.subscribe_analysis_events();

    // 准备模拟 Worker 与 Mock 解码器
    let infer_backend = Arc::new(MockInferBackend::new(0.45));
    let worker = InferenceWorker::new(infer_backend);
    let decoder: Box<dyn VideoDecoder + Send> =
        Box::new(MockDecoder::new(cam_id, CodecType::H264, 640, 360));

    let params = StartCameraPipelineParams {
        camera_id: cam_id.to_string(),
        main_rtsp_url: main_url.to_string(),
        main_codec: CodecType::H264,
        sub_rtsp_url: sub_url.to_string(),
        sub_codec: CodecType::H264,
        transport_policy: TransportPolicy::Tcp,
        instances: vec![InstanceLaunchConfig {
            algorithm_id: "test_algo_01".to_string(),
            algo_params: serde_json::json!({ "threshold": 0.5 }),
            target_fps: 25,
        }],
        motion_gate_enabled: false,
    };

    // 1. 启动分析管线
    let gen = coordinator
        .start_camera_pipeline_with_decoder_and_worker(params.clone(), decoder, worker)
        .await
        .expect("启动管线应成功");

    assert_eq!(gen, 1);
    assert!(coordinator.is_pipeline_running(cam_id).await);
    assert!(sub_session.ai_task_enabled.load(Ordering::SeqCst));
    assert_eq!(main_session.active_viewers.load(Ordering::SeqCst), 1);

    // 2. 检查 RuntimeInfo
    let info = coordinator
        .get_runtime_info(cam_id)
        .await
        .expect("应该查到运行时信息");
    assert_eq!(info.camera_id, cam_id);
    assert_eq!(info.generation, 1);
    assert!(info.is_pump_running);
    assert_eq!(info.target_fps, 25);

    // 3. 模拟推送主码流包与子码流包
    let _ = main_session.broadcast_tx.send(create_packet(1000, true));
    let _ = sub_session.broadcast_tx.send(create_packet(1000, true));

    // 等待子流解码与推理
    tokio::time::sleep(Duration::from_millis(150)).await;

    // 4. 验证事件通道能收到 Tracks 事件
    let mut received_track = false;
    while let Ok(evt) = event_rx.try_recv() {
        if let PipelineAnalysisEvent::Tracks(track_evt) = evt {
            if track_evt.camera_id == cam_id {
                received_track = true;
                assert!(!track_evt.tracks.is_empty());
                break;
            }
        }
    }
    assert!(received_track, "应通过广播事件通道收到航迹跟踪事件");

    // 5. 优雅停止管线
    let stopped = coordinator.stop_camera_pipeline(cam_id).await;
    assert!(stopped);
    assert!(!coordinator.is_pipeline_running(cam_id).await);
    assert!(!sub_session.ai_task_enabled.load(Ordering::SeqCst));
    assert_eq!(
        main_session.active_viewers.load(Ordering::SeqCst),
        0,
        "主流订阅计数必须准确归零"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_coordinator_idempotency_and_reconfiguration() {
    let temp_dir = std::env::temp_dir().join(format!(
        "test_coord_idempotency_{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let pipeline_mgr = Arc::new(PipelineManager::with_evidence_dir(&temp_dir));
    let stream_hub = Arc::new(StreamHub::new());
    let algo_registry = Arc::new(AlgoRegistry::new());

    let coordinator =
        TaskRuntimeCoordinator::new(pipeline_mgr.clone(), stream_hub.clone(), algo_registry);

    let cam_id = "camera_idempotency";
    let main_url = "rtsp://mock-main/idempotency";
    let sub_url = "rtsp://mock-sub/idempotency";

    let main_session = stream_hub
        .get_or_create_session(&format!("{}:main", cam_id), main_url, TransportPolicy::Tcp)
        .await;

    let params = StartCameraPipelineParams {
        camera_id: cam_id.to_string(),
        main_rtsp_url: main_url.to_string(),
        main_codec: CodecType::H264,
        sub_rtsp_url: sub_url.to_string(),
        sub_codec: CodecType::H264,
        transport_policy: TransportPolicy::Tcp,
        instances: vec![InstanceLaunchConfig {
            algorithm_id: "general_detection".to_string(),
            algo_params: serde_json::json!({}),
            target_fps: 15,
        }],
        motion_gate_enabled: false,
    };

    let backend = Arc::new(MockInferBackend::new(0.5));
    let worker1 = InferenceWorker::new(backend.clone());
    let decoder1: Box<dyn VideoDecoder + Send> =
        Box::new(MockDecoder::new(cam_id, CodecType::H264, 640, 360));

    // 首次启动
    let gen1 = coordinator
        .start_camera_pipeline_with_decoder_and_worker(params.clone(), decoder1, worker1)
        .await
        .expect("首次启动应成功");
    assert_eq!(gen1, 1);
    assert_eq!(main_session.active_viewers.load(Ordering::SeqCst), 1);

    // 重复相同参数启动：幂等成功，不增加观众计数
    let worker_duplicate = InferenceWorker::new(backend.clone());
    let decoder_dup: Box<dyn VideoDecoder + Send> =
        Box::new(MockDecoder::new(cam_id, CodecType::H264, 640, 360));
    let gen1_again = coordinator
        .start_camera_pipeline_with_decoder_and_worker(
            params.clone(),
            decoder_dup,
            worker_duplicate,
        )
        .await
        .expect("重复启动应幂等返回");
    assert_eq!(gen1_again, gen1, "幂等启动应保持相同世代号");
    assert_eq!(
        main_session.active_viewers.load(Ordering::SeqCst),
        1,
        "幂等启动不得重复增加主流订阅计数"
    );

    // 修改参数（重配 target_fps 从 15 改为 25）
    let mut reconfig_params = params.clone();
    reconfig_params.instances[0].target_fps = 25;

    let worker2 = InferenceWorker::new(backend);
    let decoder2: Box<dyn VideoDecoder + Send> =
        Box::new(MockDecoder::new(cam_id, CodecType::H264, 640, 360));
    let gen2 = coordinator
        .start_camera_pipeline_with_decoder_and_worker(reconfig_params, decoder2, worker2)
        .await
        .expect("重配启动应成功");
    assert!(gen2 > gen1, "重配重启后世代号应递增");
    assert_eq!(
        main_session.active_viewers.load(Ordering::SeqCst),
        1,
        "优雅重启后主流订阅计数依然维持 1"
    );

    // 停止并验证计数
    coordinator.stop_camera_pipeline(cam_id).await;
    assert_eq!(main_session.active_viewers.load(Ordering::SeqCst), 0);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_coordinator_validation_and_rollback() {
    let temp_dir = std::env::temp_dir().join(format!(
        "test_coord_validation_{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let pipeline_mgr = Arc::new(PipelineManager::with_evidence_dir(&temp_dir));
    let stream_hub = Arc::new(StreamHub::new());
    let algo_registry = Arc::new(AlgoRegistry::new());

    let coordinator =
        TaskRuntimeCoordinator::new(pipeline_mgr.clone(), stream_hub.clone(), algo_registry);

    // 1. 测试参数校验：空 camera_id
    let invalid_params = StartCameraPipelineParams {
        camera_id: "".to_string(),
        main_rtsp_url: "rtsp://localhost/main".to_string(),
        main_codec: CodecType::H264,
        sub_rtsp_url: "rtsp://localhost/sub".to_string(),
        sub_codec: CodecType::H264,
        transport_policy: TransportPolicy::Tcp,
        instances: vec![InstanceLaunchConfig {
            algorithm_id: "algo_1".to_string(),
            algo_params: serde_json::json!({}),
            target_fps: 20,
        }],
        motion_gate_enabled: false,
    };
    let err = coordinator
        .start_camera_pipeline(invalid_params)
        .await
        .unwrap_err();
    assert!(format!("{err}").contains("camera_id 不能为空"));

    // 2. 测试算法包未注册回滚
    let non_existent_algo_params = StartCameraPipelineParams {
        camera_id: "cam_rollback_test".to_string(),
        main_rtsp_url: "rtsp://localhost/main".to_string(),
        main_codec: CodecType::H264,
        sub_rtsp_url: "rtsp://localhost/sub".to_string(),
        sub_codec: CodecType::H264,
        transport_policy: TransportPolicy::Tcp,
        instances: vec![InstanceLaunchConfig {
            algorithm_id: "non_existent_algo".to_string(),
            algo_params: serde_json::json!({}),
            target_fps: 20,
        }],
        motion_gate_enabled: false,
    };
    let err = coordinator
        .start_camera_pipeline(non_existent_algo_params)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        pipeline::CoordinatorError::AlgorithmNotFound { .. }
    ));

    // 验证无孤儿运行时存在
    assert!(!coordinator.is_pipeline_running("cam_rollback_test").await);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_coordinator_alarm_trigger_and_event_broadcast() {
    let temp_dir = std::env::temp_dir().join(format!(
        "test_coord_alarm_{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let pipeline_mgr = Arc::new(PipelineManager::with_evidence_dir(&temp_dir));
    let stream_hub = Arc::new(StreamHub::new());
    let algo_registry = Arc::new(AlgoRegistry::new());

    let coordinator =
        TaskRuntimeCoordinator::new(pipeline_mgr.clone(), stream_hub.clone(), algo_registry);

    let cam_id = "camera_alarm_test";
    let main_url = "rtsp://mock-main/alarm";
    let sub_url = "rtsp://mock-sub/alarm";

    // 1. 设置空间几何绊线布防规则 (水平线 Y = 0.5，A->B 即从上到下单向穿过)
    let rules = vec![DetectionRule {
        role: DetectionRuleRole::Line,
        line_direction: DetectionLineDirection::AToB,
        points: vec![DetectionPoint::new(0.0, 0.5), DetectionPoint::new(1.0, 0.5)],
    }];
    pipeline_mgr.set_camera_rules(cam_id, rules).await;

    let _main_session = stream_hub
        .get_or_create_session(&format!("{}:main", cam_id), main_url, TransportPolicy::Tcp)
        .await;
    let sub_session = stream_hub
        .get_or_create_session(&format!("{}:sub", cam_id), sub_url, TransportPolicy::Tcp)
        .await;

    let mut event_rx = pipeline_mgr.subscribe_analysis_events();

    // 初始位置 Y = 0.45 (位于绊线上方)
    let backend = Arc::new(MockInferBackend::new(0.45));
    let worker = InferenceWorker::new(backend.clone());
    let decoder: Box<dyn VideoDecoder + Send> =
        Box::new(MockDecoder::new(cam_id, CodecType::H264, 640, 360));

    let params = StartCameraPipelineParams {
        camera_id: cam_id.to_string(),
        main_rtsp_url: main_url.to_string(),
        main_codec: CodecType::H264,
        sub_rtsp_url: sub_url.to_string(),
        sub_codec: CodecType::H264,
        transport_policy: TransportPolicy::Tcp,
        instances: vec![InstanceLaunchConfig {
            algorithm_id: "tripwire_algo".to_string(),
            algo_params: serde_json::json!({}),
            target_fps: 25,
        }],
        motion_gate_enabled: false,
    };

    coordinator
        .start_camera_pipeline_with_decoder_and_worker(params, decoder, worker)
        .await
        .expect("启动管线成功");

    // 推送第 1 帧 (Y=0.45)
    let _ = sub_session.broadcast_tx.send(create_packet(1000, true));
    tokio::time::sleep(Duration::from_millis(80)).await;

    // 移动目标至 Y = 0.55 (穿过绊线，维持 IoU=0.5 航迹关联)，推送第 2 帧
    backend.set_y(0.55);
    let _ = sub_session.broadcast_tx.send(create_packet(1040, false));

    // 验证是否收到告警事件 (异步落盘证据可能需要一定时间，采用有界轮询等待)
    let mut received_alarm = false;
    let deadline = tokio::time::Instant::now() + Duration::from_millis(1500);
    while tokio::time::Instant::now() < deadline {
        if let Ok(PipelineAnalysisEvent::Alarm(alarm_evt)) = event_rx.try_recv() {
            if alarm_evt.camera_id == cam_id {
                received_alarm = true;
                assert!(!alarm_evt.event_id.is_empty());
                assert_eq!(alarm_evt.camera_id, cam_id);
                assert_eq!(
                    alarm_evt.evidence_status,
                    pipeline::events::EvidenceStatus::Ready
                );
                assert!(!alarm_evt
                    .snapshot
                    .as_ref()
                    .expect("成功告警必须携带证据")
                    .image_rel_path
                    .is_empty());
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    assert!(
        received_alarm,
        "越界绊线规则触发后应成功向分析事件通道广播 Alarm 事件"
    );

    coordinator.stop_camera_pipeline(cam_id).await;
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_coordinator_validation_detailed() {
    let temp_dir = std::env::temp_dir().join(format!(
        "test_coord_validation_det_{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let pipeline_mgr = Arc::new(PipelineManager::with_evidence_dir(&temp_dir));
    let stream_hub = Arc::new(StreamHub::new());
    let algo_registry = Arc::new(AlgoRegistry::new());
    let coordinator = TaskRuntimeCoordinator::new(pipeline_mgr, stream_hub, algo_registry);

    let base_params = StartCameraPipelineParams {
        camera_id: "cam_valid".to_string(),
        main_rtsp_url: "rtsp://127.0.0.1:554/main".to_string(),
        main_codec: CodecType::H264,
        sub_rtsp_url: "rtsp://127.0.0.1:554/sub".to_string(),
        sub_codec: CodecType::H264,
        transport_policy: TransportPolicy::Tcp,
        instances: vec![InstanceLaunchConfig {
            algorithm_id: "algo_test".to_string(),
            algo_params: serde_json::json!({}),
            target_fps: 25,
        }],
        motion_gate_enabled: false,
    };

    // 1. target_fps > 60
    let mut p = base_params.clone();
    p.instances[0].target_fps = 61;
    let err = p.validate().unwrap_err();
    assert!(format!("{err}").contains("target_fps 必须在 0..=60 之间"));

    // 2. 空 instances 校验
    let mut p = base_params.clone();
    p.instances.clear();
    let err = p.validate().unwrap_err();
    assert!(format!("{err}").contains("至少需要一个算法实例"));

    // 2b. 重复 algorithm_id
    let mut p = base_params.clone();
    p.instances.push(InstanceLaunchConfig {
        algorithm_id: "algo_test".to_string(),
        algo_params: serde_json::json!({}),
        target_fps: 10,
    });
    let err = p.validate().unwrap_err();
    assert!(format!("{err}").contains("存在重复的 algorithm_id"));

    // 3. 非 RTSP 协议
    let mut p = base_params.clone();
    p.main_rtsp_url = "http://127.0.0.1/main".to_string();
    let err = p.validate().unwrap_err();
    assert!(format!("{err}").contains("不支持的协议类型"));

    // 4. 空主机 RTSP
    let mut p = base_params.clone();
    p.sub_rtsp_url = "rtsp:///live/sub".to_string();
    let err = p.validate().unwrap_err();
    assert!(format!("{err}").contains("缺少主机地址"));

    // 5. camera_id 含 NUL 字符
    let mut p = base_params.clone();
    p.camera_id = "cam\0evil".to_string();
    let err = p.validate().unwrap_err();
    assert!(format!("{err}").contains("非法"));

    // 6. algo_params 超大 (超 64KB)
    let mut p = base_params.clone();
    p.instances[0].algo_params = serde_json::json!({ "big_blob": "x".repeat(70_000) });
    let err = p.validate().unwrap_err();
    assert!(format!("{err}").contains("algo_params 序列化后必须小于"));

    // 验证这些非法参数不能通过 coordinator 启动
    let err = coordinator.start_camera_pipeline(p).await.unwrap_err();
    assert!(matches!(err, pipeline::CoordinatorError::Validation { .. }));

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_coordinator_concurrent_starts_serialized() {
    let temp_dir = std::env::temp_dir().join(format!(
        "test_coord_concurrent_{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let pipeline_mgr = Arc::new(PipelineManager::with_evidence_dir(&temp_dir));
    let stream_hub = Arc::new(StreamHub::new());
    let algo_registry = Arc::new(AlgoRegistry::new());
    let coordinator =
        TaskRuntimeCoordinator::new(pipeline_mgr.clone(), stream_hub.clone(), algo_registry);

    let cam_id = "cam_concurrent_test";
    let main_url = "rtsp://mock-main/concurrent";
    let sub_url = "rtsp://mock-sub/concurrent";

    let main_session = stream_hub
        .get_or_create_session(&format!("{cam_id}:main"), main_url, TransportPolicy::Tcp)
        .await;

    let params = StartCameraPipelineParams {
        camera_id: cam_id.to_string(),
        main_rtsp_url: main_url.to_string(),
        main_codec: CodecType::H264,
        sub_rtsp_url: sub_url.to_string(),
        sub_codec: CodecType::H264,
        transport_policy: TransportPolicy::Tcp,
        instances: vec![InstanceLaunchConfig {
            algorithm_id: "algo_concurrent".to_string(),
            algo_params: serde_json::json!({}),
            target_fps: 20,
        }],
        motion_gate_enabled: false,
    };

    let backend = Arc::new(MockInferBackend::new(0.5));

    // 并发启动 6 个任务尝试启动同一个 camera_id
    let mut handles = Vec::new();
    for _ in 0..6 {
        let coord = coordinator.clone();
        let p = params.clone();
        let w = InferenceWorker::new(backend.clone());
        let d: Box<dyn VideoDecoder + Send> =
            Box::new(MockDecoder::new(cam_id, CodecType::H264, 640, 360));
        handles.push(tokio::spawn(async move {
            coord
                .start_camera_pipeline_with_decoder_and_worker(p, d, w)
                .await
        }));
    }

    let mut gens = Vec::new();
    for h in handles {
        let res = h.await.unwrap();
        gens.push(res.expect("所有并发启动均应成功（首次或幂等返回）"));
    }

    // 所有调用返回的 generation 应一致（或者单调递增至最终同一世代）
    let last_gen = gens[0];
    for g in &gens {
        assert_eq!(*g, last_gen);
    }

    // 主码流观众计数严格为 1，无重复订阅泄漏
    assert_eq!(
        main_session.active_viewers.load(Ordering::SeqCst),
        1,
        "并发启动后主流观众计数必须严格为 1"
    );

    // 并发停止测试
    let mut stop_handles = Vec::new();
    for _ in 0..4 {
        let coord = coordinator.clone();
        stop_handles.push(tokio::spawn(async move {
            coord.stop_camera_pipeline(cam_id).await
        }));
    }

    for h in stop_handles {
        let _ = h.await.unwrap();
    }

    assert!(!coordinator.is_pipeline_running(cam_id).await);
    assert_eq!(
        main_session.active_viewers.load(Ordering::SeqCst),
        0,
        "停止后主流观众计数必须归零"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_coordinator_start_cancellation_safety() {
    let temp_dir = std::env::temp_dir().join(format!(
        "test_coord_cancel_{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let pipeline_mgr = Arc::new(PipelineManager::with_evidence_dir(&temp_dir));
    let stream_hub = Arc::new(StreamHub::new());
    let algo_registry = Arc::new(AlgoRegistry::new());
    let coordinator =
        TaskRuntimeCoordinator::new(pipeline_mgr.clone(), stream_hub.clone(), algo_registry);

    let cam_id = "cam_cancel_test";
    let main_url = "rtsp://mock-main/cancel";
    let sub_url = "rtsp://mock-sub/cancel";

    let params = StartCameraPipelineParams {
        camera_id: cam_id.to_string(),
        main_rtsp_url: main_url.to_string(),
        main_codec: CodecType::H264,
        sub_rtsp_url: sub_url.to_string(),
        sub_codec: CodecType::H264,
        transport_policy: TransportPolicy::Tcp,
        instances: vec![InstanceLaunchConfig {
            algorithm_id: "algo_cancel".to_string(),
            algo_params: serde_json::json!({}),
            target_fps: 20,
        }],
        motion_gate_enabled: false,
    };

    let backend = Arc::new(MockInferBackend::new(0.5));
    let worker = InferenceWorker::new(backend);
    let decoder: Box<dyn VideoDecoder + Send> =
        Box::new(MockDecoder::new(cam_id, CodecType::H264, 640, 360));

    // 在外部协程中调用启动并立即中途 abort，测试取消安全性（内部后台编排不受外层 abort 影响）
    let coord = coordinator.clone();
    let handle = tokio::spawn(async move {
        coord
            .start_camera_pipeline_with_decoder_and_worker(params, decoder, worker)
            .await
    });

    // 立即 abort 外层 JoinHandle
    handle.abort();
    let _ = handle.await;

    // 给后台完成事务 100ms
    tokio::time::sleep(Duration::from_millis(100)).await;

    // 此时后台任务或者完整启动或者完整回滚，状态绝不会处于撕裂/悬挂态
    // 调用 stop 能幂等平稳回收
    coordinator.stop_camera_pipeline(cam_id).await;
    assert!(!coordinator.is_pipeline_running(cam_id).await);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_coordinator_alarm_evidence_failure_preserves_alarm() {
    let temp_base = std::env::temp_dir().join(format!(
        "test_coord_fail_ev_{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&temp_base).unwrap();

    // 制造一个无法创建子目录的路径：在 base 路径下建一个普通文件，证据目录设置为以该文件为父级的路径
    let file_blocker = temp_base.join("blocker_file");
    std::fs::write(&file_blocker, b"im_a_file_not_dir").unwrap();
    let invalid_evidence_dir = file_blocker.join("nested_evidence");

    let pipeline_mgr = Arc::new(PipelineManager::with_evidence_dir(&invalid_evidence_dir));
    let stream_hub = Arc::new(StreamHub::new());
    let algo_registry = Arc::new(AlgoRegistry::new());

    let coordinator =
        TaskRuntimeCoordinator::new(pipeline_mgr.clone(), stream_hub.clone(), algo_registry);

    let cam_id = "camera_fail_evidence";
    let main_url = "rtsp://mock-main/fail_ev";
    let sub_url = "rtsp://mock-sub/fail_ev";

    let rules = vec![DetectionRule {
        role: DetectionRuleRole::Line,
        line_direction: DetectionLineDirection::AToB,
        points: vec![DetectionPoint::new(0.0, 0.5), DetectionPoint::new(1.0, 0.5)],
    }];
    pipeline_mgr.set_camera_rules(cam_id, rules).await;

    let sub_session = stream_hub
        .get_or_create_session(&format!("{}:sub", cam_id), sub_url, TransportPolicy::Tcp)
        .await;

    let mut event_rx = pipeline_mgr.subscribe_analysis_events();

    let backend = Arc::new(MockInferBackend::new(0.45));
    let worker = InferenceWorker::new(backend.clone());
    let decoder: Box<dyn VideoDecoder + Send> =
        Box::new(MockDecoder::new(cam_id, CodecType::H264, 640, 360));

    let params = StartCameraPipelineParams {
        camera_id: cam_id.to_string(),
        main_rtsp_url: main_url.to_string(),
        main_codec: CodecType::H264,
        sub_rtsp_url: sub_url.to_string(),
        sub_codec: CodecType::H264,
        transport_policy: TransportPolicy::Tcp,
        instances: vec![InstanceLaunchConfig {
            algorithm_id: "fail_ev_algo".to_string(),
            algo_params: serde_json::json!({}),
            target_fps: 25,
        }],
        motion_gate_enabled: false,
    };

    coordinator
        .start_camera_pipeline_with_decoder_and_worker(params, decoder, worker)
        .await
        .expect("启动管线成功");

    // 推送第 1 帧 (Y=0.45)
    let _ = sub_session.broadcast_tx.send(create_packet(1000, true));
    tokio::time::sleep(Duration::from_millis(80)).await;

    // 移动目标至 Y = 0.55 (穿过绊线触发告警)，推送第 2 帧
    backend.set_y(0.55);
    let _ = sub_session.broadcast_tx.send(create_packet(1040, false));
    tokio::time::sleep(Duration::from_millis(150)).await;

    // 验证是否收到告警事件（即使证据写盘失败，告警事实也必须广播并保留）
    let mut received_alarm = false;
    while let Ok(evt) = event_rx.try_recv() {
        if let PipelineAnalysisEvent::Alarm(alarm_evt) = evt {
            if alarm_evt.camera_id == cam_id {
                received_alarm = true;
                assert!(!alarm_evt.event_id.is_empty());
                assert_eq!(alarm_evt.camera_id, cam_id);
                assert_eq!(
                    alarm_evt.evidence_status,
                    pipeline::EvidenceStatus::Failed,
                    "证据落盘失败时状态必须为 Failed"
                );
                assert!(alarm_evt.snapshot.is_none(), "失败告警中快照字段应为 None");
                assert!(
                    alarm_evt.evidence_error.is_some(),
                    "必须记录证据失败的错误描述"
                );
                break;
            }
        }
    }

    assert!(
        received_alarm,
        "即使抓拍失败，规则引擎触发的告警事实绝不可被丢弃"
    );

    // 验证待持久化告警队列也成功保留了该失败事件
    assert!(pipeline_mgr.pending_alarm_event_count() >= 1);
    let pending = pipeline_mgr.drain_pending_alarm_events(10);
    assert!(pending
        .iter()
        .any(|a| a.camera_id == cam_id && a.evidence_status == pipeline::EvidenceStatus::Failed));

    coordinator.stop_camera_pipeline(cam_id).await;
    let _ = std::fs::remove_dir_all(&temp_base);
}

#[tokio::test]
async fn test_coordinator_empty_tracks_broadcast() {
    let temp_dir = std::env::temp_dir().join(format!(
        "test_coord_empty_tracks_{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let pipeline_mgr = Arc::new(PipelineManager::with_evidence_dir(&temp_dir));
    let stream_hub = Arc::new(StreamHub::new());
    let algo_registry = Arc::new(AlgoRegistry::new());

    let coordinator =
        TaskRuntimeCoordinator::new(pipeline_mgr.clone(), stream_hub.clone(), algo_registry);

    let cam_id = "camera_empty_tracks";
    let main_url = "rtsp://mock-main/empty_tracks";
    let sub_url = "rtsp://mock-sub/empty_tracks";

    let sub_session = stream_hub
        .get_or_create_session(&format!("{}:sub", cam_id), sub_url, TransportPolicy::Tcp)
        .await;

    let mut event_rx = pipeline_mgr.subscribe_analysis_events();

    // 空检测后端：detect 始终返回空 Vec
    #[derive(Debug)]
    struct EmptyBackend;
    #[async_trait]
    impl InferenceBackend for EmptyBackend {
        fn name(&self) -> &'static str {
            "EmptyBackend"
        }
        async fn detect(&self, _frame: &FrameRef) -> Result<Vec<Detection>, infer::InferError> {
            Ok(vec![])
        }
    }

    let worker = InferenceWorker::new(Arc::new(EmptyBackend));
    let decoder: Box<dyn VideoDecoder + Send> =
        Box::new(MockDecoder::new(cam_id, CodecType::H264, 640, 360));

    let params = StartCameraPipelineParams {
        camera_id: cam_id.to_string(),
        main_rtsp_url: main_url.to_string(),
        main_codec: CodecType::H264,
        sub_rtsp_url: sub_url.to_string(),
        sub_codec: CodecType::H264,
        transport_policy: TransportPolicy::Tcp,
        instances: vec![InstanceLaunchConfig {
            algorithm_id: "empty_algo".to_string(),
            algo_params: serde_json::json!({}),
            target_fps: 20,
        }],
        motion_gate_enabled: false,
    };

    coordinator
        .start_camera_pipeline_with_decoder_and_worker(params, decoder, worker)
        .await
        .expect("启动管线成功");

    // 推送 1 帧
    let _ = sub_session.broadcast_tx.send(create_packet(1000, true));
    tokio::time::sleep(Duration::from_millis(120)).await;

    // 验证空检测时依然广播空 Tracks 事件，以便前端 Canvas 清除残留的旧框
    let mut received_empty_tracks = false;
    while let Ok(evt) = event_rx.try_recv() {
        if let PipelineAnalysisEvent::Tracks(track_evt) = evt {
            if track_evt.camera_id == cam_id {
                assert!(track_evt.tracks.is_empty());
                received_empty_tracks = true;
                break;
            }
        }
    }

    assert!(
        received_empty_tracks,
        "空检测时必须广播空 tracks 以便客户端清理旧检测框"
    );

    coordinator.stop_camera_pipeline(cam_id).await;
    let _ = std::fs::remove_dir_all(&temp_dir);
}

/// 带有析构追踪的包装解码器，用于断言回滚时的资源释放
struct DropTrackingDecoder {
    inner: MockDecoder,
    dropped: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait]
impl VideoDecoder for DropTrackingDecoder {
    async fn decode_packet(
        &mut self,
        packet: &[u8],
        pts: i64,
    ) -> Result<Option<FrameRef>, media::error::MediaError> {
        self.inner.decode_packet(packet, pts).await
    }

    async fn flush(&mut self) -> Result<Vec<FrameRef>, media::error::MediaError> {
        self.inner.flush().await
    }

    fn delivery_policy(&self) -> media::decoder::DecodeDeliveryPolicy {
        self.inner.delivery_policy()
    }
}

impl Drop for DropTrackingDecoder {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn test_coordinator_startup_failure_disposes_decoder_on_rollback() {
    let temp_dir = std::env::temp_dir().join(format!(
        "test_coord_decoder_drop_{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let pipeline_mgr = Arc::new(PipelineManager::with_evidence_dir(&temp_dir));
    let stream_hub = Arc::new(StreamHub::new());
    let algo_registry = Arc::new(AlgoRegistry::new());
    let coordinator =
        TaskRuntimeCoordinator::new(pipeline_mgr.clone(), stream_hub.clone(), algo_registry);

    let cam_id = "camera_decoder_drop_test";
    let main_url = "rtsp://mock-main/drop_test";
    let sub_url = "rtsp://mock-sub/drop_test";

    let params = StartCameraPipelineParams {
        camera_id: cam_id.to_string(),
        main_rtsp_url: main_url.to_string(),
        main_codec: CodecType::H264,
        sub_rtsp_url: sub_url.to_string(),
        sub_codec: CodecType::H264,
        transport_policy: TransportPolicy::Tcp,
        instances: vec![InstanceLaunchConfig {
            algorithm_id: "test_algo".to_string(),
            algo_params: serde_json::json!({}),
            target_fps: 25,
        }],
        motion_gate_enabled: false,
    };

    // 1. 正常启动管线
    let initial_decoder: Box<dyn VideoDecoder + Send> =
        Box::new(MockDecoder::new(cam_id, CodecType::H264, 640, 360));
    let initial_worker = InferenceWorker::new(Arc::new(MockInferBackend::new(0.5)));
    coordinator
        .start_camera_pipeline_with_decoder_and_worker(
            params.clone(),
            initial_decoder,
            initial_worker,
        )
        .await
        .expect("首次启动成功");

    // 2. 再次以相同参数启动（命中 AlreadyRunning 幂等退出分支）
    let dropped_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let duplicate_decoder: Box<dyn VideoDecoder + Send> = Box::new(DropTrackingDecoder {
        inner: MockDecoder::new(cam_id, CodecType::H264, 640, 360),
        dropped: dropped_flag.clone(),
    });
    let duplicate_worker = InferenceWorker::new(Arc::new(MockInferBackend::new(0.5)));

    let gen = coordinator
        .start_camera_pipeline_with_decoder_and_worker(
            params.clone(),
            duplicate_decoder,
            duplicate_worker,
        )
        .await
        .expect("幂等直接返回原有代数");

    assert_eq!(gen, 1);
    // 断言传入的多余 decoder 已被 dispose_decoder 异步安全释放
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        dropped_flag.load(Ordering::SeqCst),
        "幂等提前返回时必须在 spawn_blocking 中安全销毁 decoder"
    );

    coordinator.stop_camera_pipeline(cam_id).await;
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_coordinator_stop_all_parallel_and_worker_handle() {
    let temp_dir = std::env::temp_dir().join(format!(
        "test_coord_stop_all_{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let pipeline_mgr = Arc::new(PipelineManager::with_evidence_dir(&temp_dir));
    let stream_hub = Arc::new(StreamHub::new());
    let algo_registry = Arc::new(AlgoRegistry::new());
    let coordinator =
        TaskRuntimeCoordinator::new(pipeline_mgr.clone(), stream_hub.clone(), algo_registry);

    for i in 1..=3 {
        let cam_id = format!("cam_multi_{i}");
        let main_url = format!("rtsp://mock-main/live_{i}");
        let sub_url = format!("rtsp://mock-sub/live_{i}");

        let _ = stream_hub
            .get_or_create_session(&format!("{}:main", cam_id), &main_url, TransportPolicy::Tcp)
            .await;
        let _ = stream_hub
            .get_or_create_session(&format!("{}:sub", cam_id), &sub_url, TransportPolicy::Tcp)
            .await;

        let decoder: Box<dyn VideoDecoder + Send> =
            Box::new(MockDecoder::new(&cam_id, CodecType::H264, 640, 360));
        let worker = InferenceWorker::new(Arc::new(MockInferBackend::new(0.5)));

        let params = StartCameraPipelineParams {
            camera_id: cam_id.clone(),
            main_rtsp_url: main_url,
            main_codec: CodecType::H264,
            sub_rtsp_url: sub_url,
            sub_codec: CodecType::H264,
            transport_policy: TransportPolicy::Tcp,
            instances: vec![InstanceLaunchConfig {
                algorithm_id: "test_algo".to_string(),
                algo_params: serde_json::json!({}),
                target_fps: 20,
            }],
            motion_gate_enabled: false,
        };

        coordinator
            .start_camera_pipeline_with_decoder_and_worker(params, decoder, worker)
            .await
            .expect("启动成功");

        assert!(coordinator.is_pipeline_running(&cam_id).await);
    }

    // 并行停止所有管线
    coordinator.stop_all().await;

    for i in 1..=3 {
        let cam_id = format!("cam_multi_{i}");
        assert!(!coordinator.is_pipeline_running(&cam_id).await);
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_ai_task_lease_raii_protection() {
    let session =
        media::CameraStreamSession::mock("cam_raii", "rtsp://mock/sub", TransportPolicy::Tcp);
    assert_eq!(session.ai_task_ref_count(), 0);
    assert!(!session.ai_task_enabled.load(Ordering::SeqCst));

    {
        let lease1 = session.acquire_ai_task_lease();
        assert_eq!(session.ai_task_ref_count(), 1);
        assert!(session.ai_task_enabled.load(Ordering::SeqCst));

        {
            let _lease2 = session.acquire_ai_task_lease();
            assert_eq!(session.ai_task_ref_count(), 2);
        }
        // lease2 超出作用域自动 Drop
        assert_eq!(session.ai_task_ref_count(), 1);
        assert!(session.ai_task_enabled.load(Ordering::SeqCst));

        drop(lease1);
    }
    // lease1 显式 Drop 后引用归零
    assert_eq!(session.ai_task_ref_count(), 0);
    assert!(!session.ai_task_enabled.load(Ordering::SeqCst));
}

#[tokio::test]
async fn test_coordinator_multi_algorithm_instances() {
    let temp_dir = std::env::temp_dir().join(format!(
        "test_coord_multi_algo_{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let pipeline_mgr = Arc::new(PipelineManager::with_evidence_dir(&temp_dir));
    let stream_hub = Arc::new(StreamHub::new());
    let algo_registry = Arc::new(AlgoRegistry::new());
    let coordinator =
        TaskRuntimeCoordinator::new(pipeline_mgr.clone(), stream_hub.clone(), algo_registry);

    let cam_id = "camera_multi_algo_01";
    let main_url = "rtsp://mock-main/live";
    let sub_url = "rtsp://mock-sub/live";

    let _ = stream_hub
        .get_or_create_session(&format!("{cam_id}:main"), main_url, TransportPolicy::Tcp)
        .await;
    let sub_session = stream_hub
        .get_or_create_session(&format!("{cam_id}:sub"), sub_url, TransportPolicy::Tcp)
        .await;

    let decoder: Box<dyn VideoDecoder + Send> =
        Box::new(MockDecoder::new(cam_id, CodecType::H264, 640, 360));
    let worker = InferenceWorker::new(Arc::new(MockInferBackend::new(0.5)));

    let params = StartCameraPipelineParams {
        camera_id: cam_id.to_string(),
        main_rtsp_url: main_url.to_string(),
        main_codec: CodecType::H264,
        sub_rtsp_url: sub_url.to_string(),
        sub_codec: CodecType::H264,
        transport_policy: TransportPolicy::Tcp,
        instances: vec![
            InstanceLaunchConfig {
                algorithm_id: "algo_face".to_string(),
                algo_params: serde_json::json!({ "model": "face_v1" }),
                target_fps: 15,
            },
            InstanceLaunchConfig {
                algorithm_id: "algo_helmet".to_string(),
                algo_params: serde_json::json!({ "model": "helmet_v2" }),
                target_fps: 10,
            },
        ],
        motion_gate_enabled: false,
    };

    // 启动多算法实例管线
    let gen = coordinator
        .start_camera_pipeline_with_decoder_and_worker(params.clone(), decoder, worker)
        .await
        .expect("多算法实例启动应成功");
    assert_eq!(gen, 1);
    assert!(coordinator.is_pipeline_running(cam_id).await);

    // 查询运行时信息，验证 1:N 实例数组正确呈现
    let info = coordinator
        .get_runtime_info(cam_id)
        .await
        .expect("必须能查到多算法运行时");
    assert_eq!(info.instances.len(), 2);
    assert_eq!(info.instances[0].algorithm_id, "algo_face");
    assert_eq!(info.instances[0].target_fps, 15);
    assert_eq!(info.instances[1].algorithm_id, "algo_helmet");
    assert_eq!(info.instances[1].target_fps, 10);
    assert_eq!(info.algorithm_id, "algo_face");
    assert_eq!(info.target_fps, 15);

    // 推送子流数据包并验证稳定处理
    let _ = sub_session.broadcast_tx.send(create_packet(1000, true));
    tokio::time::sleep(Duration::from_millis(50)).await;

    // 停止管线
    assert!(coordinator.stop_camera_pipeline(cam_id).await);
    assert!(!coordinator.is_pipeline_running(cam_id).await);

    let _ = std::fs::remove_dir_all(&temp_dir);
}
