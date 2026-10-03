#![allow(clippy::unwrap_used)]

use infer::package::{AlgoPackage, AlgoRegistry};
use media::StreamHub;
use pipeline::{
    InstanceLaunchConfig, PipelineManager, StartCameraPipelineParams, TaskRuntimeCoordinator,
};
use std::path::PathBuf;
use std::sync::Arc;
use types::{CodecType, TransportPolicy};

#[tokio::test]
async fn test_coordinator_all_instances_fail_rolls_back() {
    let temp_dir = std::env::temp_dir().join(format!(
        "test_coord_all_fail_{}",
        uuid::Uuid::now_v7().simple()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let pipeline_mgr = Arc::new(PipelineManager::with_evidence_dir(&temp_dir));
    let stream_hub = Arc::new(StreamHub::new());
    let algo_registry = Arc::new(AlgoRegistry::new());
    let coordinator =
        TaskRuntimeCoordinator::new(pipeline_mgr.clone(), stream_hub.clone(), algo_registry);

    let cam_id = "cam_all_fail";
    let params = StartCameraPipelineParams {
        camera_id: cam_id.to_string(),
        main_rtsp_url: "rtsp://mock-main/live".to_string(),
        main_codec: CodecType::H264,
        sub_rtsp_url: "rtsp://mock-sub/live".to_string(),
        sub_codec: CodecType::H264,
        transport_policy: TransportPolicy::Tcp,
        instances: vec![
            InstanceLaunchConfig {
                instance_id: "inst_1".to_string(),
                algorithm_id: "unregistered_algo_1".to_string(),
                algo_params: serde_json::json!({}),
                target_fps: 15,
                desired_revision: 0,
            },
            InstanceLaunchConfig {
                instance_id: "inst_2".to_string(),
                algorithm_id: "unregistered_algo_2".to_string(),
                algo_params: serde_json::json!({}),
                target_fps: 10,
                desired_revision: 0,
            },
        ],
        motion_gate: None,
    };

    let result = coordinator.start_camera_pipeline(params).await;
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err(),
        pipeline::CoordinatorError::AlgorithmNotFound { .. }
    ));

    assert!(!coordinator.is_pipeline_running(cam_id).await);
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_coordinator_partial_instance_failure_isolation() {
    let pkg_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("algo-packages/macos/arm64/general_detection");

    let Ok(pkg) = AlgoPackage::load_and_verify(&pkg_path, false) else {
        // 非 macOS aarch64 或未就绪环境跳过平台专属真实包测试
        eprintln!("跳过 macOS arm64 真实算法包测试: {pkg_path:?}");
        return;
    };

    let temp_dir = std::env::temp_dir().join(format!(
        "test_coord_partial_fail_{}",
        uuid::Uuid::now_v7().simple()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let pipeline_mgr = Arc::new(PipelineManager::with_evidence_dir(&temp_dir));
    let stream_hub = Arc::new(StreamHub::new());
    let algo_registry = Arc::new(AlgoRegistry::new());

    // 仅注册 general_detection
    let algo_id = pkg.manifest().algorithm_id.clone();
    algo_registry.register(Arc::new(pkg)).await;

    let coordinator =
        TaskRuntimeCoordinator::new(pipeline_mgr.clone(), stream_hub.clone(), algo_registry);

    let cam_id = "cam_partial_isolation";
    let main_url = "rtsp://mock-main/live";
    let sub_url = "rtsp://mock-sub/live";

    let _ = stream_hub
        .get_or_create_session(&format!("{cam_id}:main"), main_url, TransportPolicy::Tcp)
        .await;
    let _ = stream_hub
        .get_or_create_session(&format!("{cam_id}:sub"), sub_url, TransportPolicy::Tcp)
        .await;

    let params = StartCameraPipelineParams {
        camera_id: cam_id.to_string(),
        main_rtsp_url: main_url.to_string(),
        main_codec: CodecType::H264,
        sub_rtsp_url: sub_url.to_string(),
        sub_codec: CodecType::H264,
        transport_policy: TransportPolicy::Tcp,
        instances: vec![
            // 实例 1: 正常注册并就绪的算法
            InstanceLaunchConfig {
                instance_id: "inst_healthy".to_string(),
                algorithm_id: algo_id.clone(),
                algo_params: serde_json::json!({ "confidence_threshold": 0.5 }),
                target_fps: 15,
                desired_revision: 1,
            },
            // 实例 2: 未注册算法（租约获取必失败）
            InstanceLaunchConfig {
                instance_id: "inst_broken".to_string(),
                algorithm_id: "unregistered_broken_algo".to_string(),
                algo_params: serde_json::json!({}),
                target_fps: 10,
                desired_revision: 1,
            },
        ],
        motion_gate: None,
    };

    // 冷启动：尽管 inst_broken 失败，健康实例 inst_healthy 仍应成功启动，管线整体正常运行
    let gen = coordinator
        .start_camera_pipeline(params)
        .await
        .expect("健康实例应成功启动管线，单实例故障不连坐");

    assert_eq!(gen, 1);
    assert!(coordinator.is_pipeline_running(cam_id).await);

    // 查询运行时信息：只有健康的实例被挂载到分析泵
    let info = coordinator
        .get_runtime_info(cam_id)
        .await
        .expect("运行时信息存在");
    assert_eq!(info.instances.len(), 1);
    assert_eq!(info.instances[0].instance_id, "inst_healthy");
    assert_eq!(info.instances[0].algorithm_id, algo_id);

    // 平稳停止
    coordinator.stop_camera_pipeline(cam_id).await;
    assert!(!coordinator.is_pipeline_running(cam_id).await);

    let _ = std::fs::remove_dir_all(&temp_dir);
}
