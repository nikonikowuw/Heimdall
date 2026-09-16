//! 算法实例增量收敛集成测试（协调器层）
//!
//! 覆盖设计文档的核心承诺：改抽帧频率、禁用实例、媒体契约未变时重挂载实例集合，
//! 都不得整路重建解码器；无法就地生效的变更必须显式返回 `pending` / `failed`，
//! 不允许用「数据库写成功」冒充「运行时已生效」。
#![allow(clippy::unwrap_used)]

use std::sync::atomic::{AtomicU64, Ordering};
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
    InstanceApplyMechanism, InstanceDesiredConfig, InstanceLaunchConfig, PipelineManager,
    StartCameraPipelineParams, TaskRuntimeCoordinator,
};
use types::{
    BoundingBox, CodecType, Detection, EncodedPacket, FrameRef, InstanceApplyState, TransportPolicy,
};

/// 计数式模拟推理后端：验证增量收敛期间所属实例是否持续服务。
#[derive(Debug)]
struct CountingBackend {
    served: Arc<AtomicU64>,
}

#[async_trait(?Send)]
impl InferenceBackend for CountingBackend {
    fn name(&self) -> &'static str {
        "CountingBackend"
    }

    async fn detect(&self, _frame: &FrameRef) -> Result<Vec<Detection>, infer::InferError> {
        self.served.fetch_add(1, Ordering::SeqCst);
        Ok(vec![Detection {
            class_id: 0,
            label: "person".to_string(),
            confidence: 0.95,
            quality_score: None,
            bbox: BoundingBox::new(0.4, 0.2, 0.6, 0.5),
            face: None,
        }])
    }
}

fn keyframe_packet(pts_ms: i64) -> Arc<EncodedPacket> {
    let mut payload = vec![0x00, 0x00, 0x00, 0x01];
    payload.extend_from_slice(&[0x67, 0x42, 0x00, 0x1f]);
    payload.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
    payload.extend_from_slice(&[0x68, 0xce, 0x3c, 0x80]);
    Arc::new(EncodedPacket {
        pts_ms,
        is_keyframe: true,
        codec: CodecType::H264,
        payload: Bytes::from(payload),
        ..Default::default()
    })
}

async fn wait_until<F, Fut>(timeout_ms: u64, mut condition: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        if condition().await {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

struct TestFixture {
    _temp_dir: std::path::PathBuf,
    coordinator: TaskRuntimeCoordinator,
    pipeline_mgr: Arc<PipelineManager>,
    _stream_hub: Arc<StreamHub>,
    camera_id: String,
    sub_session: Arc<media::CameraStreamSession>,
    served: Arc<AtomicU64>,
}

impl TestFixture {
    async fn new(camera_id: &str) -> Self {
        let temp_dir =
            std::env::temp_dir().join(format!("test_coord_incr_{}", uuid::Uuid::now_v7().simple()));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let pipeline_mgr = Arc::new(PipelineManager::with_evidence_dir(&temp_dir));
        let stream_hub = Arc::new(StreamHub::new());
        let coordinator = TaskRuntimeCoordinator::new(
            pipeline_mgr.clone(),
            stream_hub.clone(),
            Arc::new(AlgoRegistry::new()),
        );

        let sub_url = "rtsp://mock-sub/live";
        let sub_session = stream_hub
            .get_or_create_session(&format!("{camera_id}:sub"), sub_url, TransportPolicy::Tcp)
            .await;

        Self {
            _temp_dir: temp_dir,
            coordinator,
            pipeline_mgr,
            _stream_hub: stream_hub,
            camera_id: camera_id.to_string(),
            sub_session,
            served: Arc::new(AtomicU64::new(0)),
        }
    }

    fn params(&self, instances: Vec<InstanceLaunchConfig>) -> StartCameraPipelineParams {
        self.params_with_url(instances, "rtsp://mock-main/live", "rtsp://mock-sub/live")
    }

    fn params_with_url(
        &self,
        instances: Vec<InstanceLaunchConfig>,
        main_url: &str,
        sub_url: &str,
    ) -> StartCameraPipelineParams {
        StartCameraPipelineParams {
            camera_id: self.camera_id.clone(),
            main_rtsp_url: main_url.to_string(),
            main_codec: CodecType::H264,
            sub_rtsp_url: sub_url.to_string(),
            sub_codec: CodecType::H264,
            transport_policy: TransportPolicy::Tcp,
            instances,
            motion_gate: None,
        }
    }

    async fn start(&self, instances: Vec<InstanceLaunchConfig>) -> u64 {
        let worker = InferenceWorker::new(Arc::new(CountingBackend {
            served: self.served.clone(),
        }));
        let decoder: Box<dyn VideoDecoder + Send> =
            Box::new(MockDecoder::new(&self.camera_id, CodecType::H264, 640, 360));
        self.coordinator
            .start_camera_pipeline_with_decoder_and_worker(self.params(instances), decoder, worker)
            .await
            .expect("启动管线应成功")
    }

    fn desired(
        &self,
        instance_id: &str,
        algorithm_id: &str,
        params_json: &str,
    ) -> InstanceDesiredConfig {
        InstanceDesiredConfig {
            camera_id: self.camera_id.clone(),
            instance_id: instance_id.to_string(),
            algorithm_id: algorithm_id.to_string(),
            analysis_fps: 10,
            params_json: params_json.to_string(),
            enabled: true,
            desired_revision: 7,
        }
    }
}

/// 注入式启动路径下 Worker 不携带创建配置，因此期望配置用空对象表达「无配置变更」。
fn launch(instance_id: &str, algorithm_id: &str, target_fps: u32) -> InstanceLaunchConfig {
    InstanceLaunchConfig {
        instance_id: instance_id.to_string(),
        algorithm_id: algorithm_id.to_string(),
        algo_params: serde_json::json!({}),
        target_fps,
    }
}

/// 抽帧频率变更只在帧边界替换 governor：Worker 不重建、解码器不重启、其他实例不受影响。
#[tokio::test]
async fn test_apply_instance_config_fps_change_keeps_pipeline_and_workers() {
    let fixture = TestFixture::new("camera_coord_fps").await;
    let gen = fixture
        .start(vec![
            launch("inst_fps_a", "algo_a", 25),
            launch("inst_fps_b", "algo_b", 25),
        ])
        .await;
    assert_eq!(gen, 1);

    // 让两个实例先进入稳定运行
    fixture
        .sub_session
        .dispatcher
        .publish(keyframe_packet(1000));
    assert!(
        wait_until(3000, || async {
            fixture.served.load(Ordering::SeqCst) >= 1
        })
        .await,
        "实例必须已进入推理"
    );

    let descriptors_before = fixture
        .pipeline_mgr
        .get_instance_descriptors(&fixture.camera_id)
        .await;

    // 仅变更目标实例的抽帧频率
    let mut desired = fixture.desired("inst_fps_a", "algo_a", "{}");
    desired.analysis_fps = 5;
    let outcome = fixture
        .coordinator
        .apply_instance_config(desired)
        .await
        .expect("收敛实例配置应成功");

    assert_eq!(outcome.apply_state, InstanceApplyState::Applied);
    assert_eq!(outcome.mechanism, InstanceApplyMechanism::FrameRate);
    assert_eq!(
        outcome.applied_revision,
        Some(7),
        "生效版本号必须回传期望版本号"
    );
    assert!(outcome.status_message.is_empty());

    // 解码器与整路运行时代际不变 = 没有整路重建
    assert_eq!(
        fixture
            .coordinator
            .get_runtime_info(&fixture.camera_id)
            .await
            .expect("运行时信息")
            .generation,
        gen,
        "抽帧频率变更不得重建整路管线"
    );

    let descriptors_after = fixture
        .pipeline_mgr
        .get_instance_descriptors(&fixture.camera_id)
        .await;
    assert_eq!(
        descriptors_after.len(),
        descriptors_before.len(),
        "不得增删实例"
    );
    let target = descriptors_after
        .iter()
        .find(|descriptor| descriptor.instance_id == "inst_fps_a")
        .expect("目标实例仍在挂载");
    assert_eq!(target.target_fps, 5, "目标实例抽帧频率必须已更新");
    let untouched = descriptors_after
        .iter()
        .find(|descriptor| descriptor.instance_id == "inst_fps_b")
        .expect("另一实例仍在挂载");
    assert_eq!(untouched.target_fps, 25, "其他实例抽帧频率不得被改动");

    // 变更后仍持续产出
    let served_before = fixture.served.load(Ordering::SeqCst);
    let mut pts = 2000;
    for _ in 0..6 {
        pts += 500;
        fixture.sub_session.dispatcher.publish(keyframe_packet(pts));
    }
    assert!(
        wait_until(3000, || async {
            fixture.served.load(Ordering::SeqCst) > served_before
        })
        .await,
        "抽帧频率变更后实例必须继续产出推理结果"
    );

    let _ = fixture
        .coordinator
        .stop_camera_pipeline(&fixture.camera_id)
        .await;
}

/// 禁用实例走卸载路径：只移除目标实例，其他实例与解码器继续运行。
#[tokio::test]
async fn test_disable_instance_unmounts_only_target_instance() {
    let fixture = TestFixture::new("camera_coord_disable").await;
    let gen = fixture
        .start(vec![
            launch("inst_keep", "algo_keep", 25),
            launch("inst_drop", "algo_drop", 25),
        ])
        .await;
    assert_eq!(gen, 1);

    let mut desired = fixture.desired("inst_drop", "algo_drop", r#"{"threshold":0.5}"#);
    desired.enabled = false;
    let outcome = fixture
        .coordinator
        .apply_instance_config(desired)
        .await
        .expect("禁用实例应成功");

    assert_eq!(outcome.apply_state, InstanceApplyState::Applied);
    assert_eq!(outcome.mechanism, InstanceApplyMechanism::Unmount);

    let descriptors = fixture
        .pipeline_mgr
        .get_instance_descriptors(&fixture.camera_id)
        .await;
    assert_eq!(
        descriptors
            .iter()
            .map(|descriptor| descriptor.instance_id.as_str())
            .collect::<Vec<_>>(),
        vec!["inst_keep"],
        "禁用后仅应保留启用中的实例"
    );
    assert!(
        fixture
            .coordinator
            .is_pipeline_running(&fixture.camera_id)
            .await
    );
    assert_eq!(
        fixture
            .coordinator
            .get_runtime_info(&fixture.camera_id)
            .await
            .expect("运行时信息")
            .generation,
        gen,
        "禁用单个实例不得重建整路管线"
    );

    // 卸载后重复应用同一禁用期望必须是幂等 noop
    let mut again = fixture.desired("inst_drop", "algo_drop", r#"{"threshold":0.5}"#);
    again.enabled = false;
    let repeat = fixture
        .coordinator
        .apply_instance_config(again)
        .await
        .expect("重复禁用应成功");
    assert_eq!(repeat.apply_state, InstanceApplyState::Applied);
    assert_eq!(repeat.mechanism, InstanceApplyMechanism::Noop);

    // 保留实例仍在服务
    let served_before = fixture.served.load(Ordering::SeqCst);
    let mut pts = 3000;
    for _ in 0..4 {
        pts += 500;
        fixture.sub_session.dispatcher.publish(keyframe_packet(pts));
    }
    assert!(
        wait_until(3000, || async {
            fixture.served.load(Ordering::SeqCst) > served_before
        })
        .await,
        "禁用其他实例后保留实例必须持续产出"
    );

    let _ = fixture
        .coordinator
        .stop_camera_pipeline(&fixture.camera_id)
        .await;
}

/// 算法包未就绪时新增实例必须显式返回 `pending`，不得静默失败或冒充成功。
#[tokio::test]
async fn test_mount_without_ready_package_returns_pending() {
    let fixture = TestFixture::new("camera_coord_pending").await;
    fixture
        .start(vec![launch("inst_ready", "algo_ready", 25)])
        .await;

    let outcome = fixture
        .coordinator
        .apply_instance_config(fixture.desired(
            "inst_new",
            "algo_not_installed",
            r#"{"threshold":0.5}"#,
        ))
        .await
        .expect("收敛调用本身应成功返回结果");

    assert_eq!(
        outcome.apply_state,
        InstanceApplyState::Pending,
        "算法包未就绪必须返回 pending 而不是 applied"
    );
    assert_eq!(outcome.mechanism, InstanceApplyMechanism::Mount);
    assert!(
        outcome.status_message.contains("未在注册中心就绪"),
        "pending 必须带上可读原因，实际: {}",
        outcome.status_message
    );
    assert_eq!(outcome.applied_revision, None);

    // 未能挂载不得影响既有实例
    let descriptors = fixture
        .pipeline_mgr
        .get_instance_descriptors(&fixture.camera_id)
        .await;
    assert_eq!(descriptors.len(), 1);
    assert_eq!(descriptors[0].instance_id, "inst_ready");

    let _ = fixture
        .coordinator
        .stop_camera_pipeline(&fixture.camera_id)
        .await;
}

/// 任务未运行时，期望配置只停留在持久层，运行时状态返回 applied + noRuntime（无编造生效）。
#[tokio::test]
async fn test_apply_without_running_pipeline_reports_no_runtime() {
    let fixture = TestFixture::new("camera_coord_stopped").await;

    let outcome = fixture
        .coordinator
        .apply_instance_config(fixture.desired("inst_idle", "algo_idle", r#"{"threshold":0.5}"#))
        .await
        .expect("未运行时的收敛调用也应正常返回");

    assert_eq!(outcome.apply_state, InstanceApplyState::Applied);
    assert_eq!(outcome.mechanism, InstanceApplyMechanism::NoRuntime);
    assert!(
        outcome.status_message.contains("将在下次启动时收敛"),
        "未运行时必须说明期望配置的收敛时机，实际: {}",
        outcome.status_message
    );
    assert!(
        !fixture
            .coordinator
            .is_pipeline_running(&fixture.camera_id)
            .await
    );
}

/// 媒体输入契约未变化时，实例集合变更走增量路径；地址变化才整路重建。
#[tokio::test]
async fn test_sync_camera_instances_is_incremental_until_media_contract_changes() {
    let fixture = TestFixture::new("camera_coord_sync").await;
    let gen = fixture
        .start(vec![
            launch("inst_sync_a", "algo_sync_a", 25),
            launch("inst_sync_b", "algo_sync_b", 25),
        ])
        .await;
    assert_eq!(gen, 1);

    // 撤掉一个实例、保留一个实例；媒体契约不变
    let synced = fixture
        .coordinator
        .sync_camera_instances(fixture.params(vec![launch("inst_sync_a", "algo_sync_a", 25)]))
        .await
        .expect("增量收敛应成功");

    assert!(!synced.restarted, "媒体契约未变化时不得整路重建");
    assert_eq!(
        synced.outcomes.len(),
        2,
        "收敛结果必须逐实例上报（卸载 + 保留实例的 noop）"
    );
    // 卸载必须排在挂载/更新之前，尽早归还算力与内存配额
    assert_eq!(synced.outcomes[0].instance_id, "inst_sync_b");
    assert_eq!(
        synced.outcomes[0].mechanism,
        InstanceApplyMechanism::Unmount
    );
    assert_eq!(synced.outcomes[0].apply_state, InstanceApplyState::Applied);
    assert_eq!(synced.outcomes[1].instance_id, "inst_sync_a");
    assert_eq!(synced.outcomes[1].mechanism, InstanceApplyMechanism::Noop);
    assert_eq!(synced.outcomes[1].apply_state, InstanceApplyState::Applied);
    assert_eq!(
        fixture
            .coordinator
            .get_runtime_info(&fixture.camera_id)
            .await
            .expect("运行时信息")
            .generation,
        gen,
        "增量收敛不得改变整路运行时代际"
    );

    // 仅实例集合变化不得要求整路重建
    assert!(
        !fixture
            .coordinator
            .requires_media_restart(&fixture.params(vec![launch("inst_sync_a", "algo_sync_a", 5)]))
            .await,
        "仅抽帧频率/参数变化不得要求整路重建"
    );

    // 分析流地址变化必须整路重建
    let changed_media = fixture.params_with_url(
        vec![launch("inst_sync_a", "algo_sync_a", 25)],
        "rtsp://mock-main/live",
        "rtsp://mock-sub/v2",
    );
    assert!(
        fixture
            .coordinator
            .requires_media_restart(&changed_media)
            .await,
        "分析流地址变化必须要求整路重建"
    );

    // 重建路径自身要求真实算法包（测试环境没有可加载的制品），此处只验证失败时不会
    // 留下半收敛的实例状态：期望配置要么整路生效，要么明确失败。
    let err = fixture
        .coordinator
        .sync_camera_instances(changed_media)
        .await
        .expect_err("无算法包时整路重建必须失败");
    assert!(
        matches!(err, pipeline::CoordinatorError::AlgorithmNotFound { .. }),
        "重建失败原因应为算法包未就绪，实际: {err}"
    );
    assert!(
        fixture
            .pipeline_mgr
            .get_instance_descriptors(&fixture.camera_id)
            .await
            .is_empty(),
        "整路重建失败时不得残留部分实例的收敛状态"
    );
}
