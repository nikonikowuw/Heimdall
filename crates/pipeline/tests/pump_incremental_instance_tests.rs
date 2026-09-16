//! 算法实例增量运行时集成测试
//!
//! 验证设计目标：算法实例的挂载、抽帧频率变更、Worker 替换与移除都不重启解码器
//! 与其他实例；被替换 Worker 的迟到结果由实例代际栅栏丢弃。
#![allow(clippy::unwrap_used)]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use infer::{InferenceBackend, InferenceWorker};
use media::decoder::VideoDecoder;
use media::decoders::MockDecoder;
use media::stream_hub::CameraStreamSession;
use media::PacketDispatcher;
use pipeline::{PipelineManager, WorkerInstanceConfig};
use types::{BoundingBox, CodecType, Detection, EncodedPacket, FrameRef, TransportPolicy};

/// 计数式模拟推理后端：记录自身服务过的帧数，并可选地模拟硬件推理耗时。
#[derive(Debug)]
struct CountingBackend {
    served: Arc<AtomicU64>,
    delay_ms: u64,
}

#[async_trait(?Send)]
impl InferenceBackend for CountingBackend {
    fn name(&self) -> &'static str {
        "CountingBackend"
    }

    async fn detect(&self, _frame: &FrameRef) -> Result<Vec<Detection>, infer::InferError> {
        if self.delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
        }
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

/// 轮询等待异步条件成立，避免固定 sleep 与帧处理时序竞争。
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

async fn instance_metrics(
    manager: &PipelineManager,
    camera_id: &str,
    instance_id: &str,
) -> Option<Arc<pipeline::InstanceMetrics>> {
    manager
        .get_instance_metrics(camera_id)
        .await
        .into_iter()
        .find(|(id, _)| id == instance_id)
        .map(|(_, metrics)| metrics)
}

async fn inferred_frames(manager: &PipelineManager, camera_id: &str, instance_id: &str) -> u64 {
    instance_metrics(manager, camera_id, instance_id)
        .await
        .map(|metrics| metrics.frames_inferred.load(Ordering::Relaxed))
        .unwrap_or(0)
}

async fn sampled_frames(manager: &PipelineManager, camera_id: &str, instance_id: &str) -> u64 {
    instance_metrics(manager, camera_id, instance_id)
        .await
        .map(|metrics| metrics.frames_sampled.load(Ordering::Relaxed))
        .unwrap_or(0)
}

/// 推送一帧并等待目标实例完成对应推理，消除 drop-oldest 带来的计数抖动。
async fn publish_and_settle(
    manager: &PipelineManager,
    dispatcher: &PacketDispatcher,
    camera_id: &str,
    instance_id: &str,
    pts_ms: i64,
) {
    let target = inferred_frames(manager, camera_id, instance_id).await + 1;
    dispatcher.publish(keyframe_packet(pts_ms));
    let reached = wait_until(3000, || async {
        inferred_frames(manager, camera_id, instance_id).await >= target
    })
    .await;
    assert!(
        reached,
        "实例 {instance_id} 未在时限内完成第 {target} 帧推理"
    );
}

/// 增量挂载、抽帧频率变更与移除都不得影响已有实例，也不得重启解码器。
#[tokio::test]
async fn test_incremental_instance_lifecycle_keeps_other_instances_running() {
    let temp_dir =
        std::env::temp_dir().join(format!("test_pump_incr_{}", uuid::Uuid::now_v7().simple()));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let manager = Arc::new(PipelineManager::with_evidence_dir(&temp_dir));
    let cam_id = "camera_pump_incremental";
    let session = CameraStreamSession::mock(cam_id, "rtsp://mock-sub/live", TransportPolicy::Tcp);
    let dispatcher = session.dispatcher.clone();

    let worker_a = InferenceWorker::new(Arc::new(CountingBackend {
        served: Arc::new(AtomicU64::new(0)),
        delay_ms: 0,
    }));
    let handle_a = worker_a.handle();
    let decoder: Box<dyn VideoDecoder + Send> =
        Box::new(MockDecoder::new(cam_id, CodecType::H264, 640, 360));

    manager
        .start_analysis_pump_multi_worker(
            cam_id,
            session.clone(),
            decoder,
            vec![WorkerInstanceConfig {
                instance_id: "inst_a".to_string(),
                algorithm_id: "algo_a".to_string(),
                algorithm_type: "detection".to_string(),
                target_fps: 0,
                config_json: None,
            }],
            vec![("inst_a".to_string(), handle_a.clone(), Some(worker_a))],
            None,
        )
        .await;

    let mut pts = 1000;
    for _ in 0..3 {
        pts += 100;
        publish_and_settle(&manager, &dispatcher, cam_id, "inst_a", pts).await;
    }

    // ── 增量挂载实例 B：不得触碰解码器、实例 A 与其 Worker ────────────────────
    let served_b = Arc::new(AtomicU64::new(0));
    let worker_b = InferenceWorker::new(Arc::new(CountingBackend {
        served: served_b.clone(),
        delay_ms: 0,
    }));
    let handle_b = worker_b.handle();
    let added = manager
        .add_pump_instance(
            cam_id,
            WorkerInstanceConfig {
                instance_id: "inst_b".to_string(),
                algorithm_id: "algo_b".to_string(),
                algorithm_type: "detection".to_string(),
                target_fps: 0,
                config_json: None,
            },
            worker_b,
        )
        .await
        .expect("分析泵必须处于运行状态");
    assert!(added, "增量挂载实例 B 必须成功");
    assert!(handle_a.is_alive(), "增量挂载不得重建实例 A 的 Worker");

    let a_before_add = inferred_frames(&manager, cam_id, "inst_a").await;
    for _ in 0..3 {
        pts += 100;
        publish_and_settle(&manager, &dispatcher, cam_id, "inst_b", pts).await;
    }
    assert!(
        inferred_frames(&manager, cam_id, "inst_a").await > a_before_add,
        "新增实例 B 后实例 A 必须持续产出推理结果"
    );

    // ── 抽帧频率变更：只在帧边界替换 governor，不重建 Worker ─────────────────
    assert!(manager
        .set_pump_instance_fps(cam_id, "inst_b", 1)
        .await
        .expect("分析泵必须处于运行状态"));
    assert!(handle_b.is_alive(), "抽帧频率变更不得重建实例 B 的 Worker");
    let b_sampled_before = sampled_frames(&manager, cam_id, "inst_b").await;

    // 1 FPS 下以 100ms 间隔推送 10 帧，最多命中 2 次采样周期。
    for _ in 0..10 {
        pts += 100;
        dispatcher.publish(keyframe_packet(pts));
    }
    tokio::time::sleep(Duration::from_millis(400)).await;
    let sampled_delta = sampled_frames(&manager, cam_id, "inst_b").await - b_sampled_before;
    assert!(
        sampled_delta <= 2,
        "抽帧频率必须已降至 1 FPS，实际新增采样 {sampled_delta} 帧"
    );
    assert!(
        sampled_delta >= 1,
        "抽帧频率变更后仍必须按新周期采样，实际新增采样 {sampled_delta} 帧"
    );

    // ── 移除实例 B：A 继续产出，B 停止发帧且 Worker 被回收 ─────────────────────
    // 记录移除前的指标句柄：槽位卸载后仍可断言计数已冻结。
    let b_metrics_before_remove = instance_metrics(&manager, cam_id, "inst_b")
        .await
        .expect("实例 B 指标");
    assert!(manager
        .remove_pump_instance(cam_id, "inst_b")
        .await
        .expect("分析泵必须处于运行状态"));
    let descriptors = manager.get_instance_descriptors(cam_id).await;
    assert_eq!(
        descriptors
            .iter()
            .map(|descriptor| descriptor.instance_id.as_str())
            .collect::<Vec<_>>(),
        vec!["inst_a"],
        "移除后仅应保留实例 A"
    );
    assert!(!handle_b.is_alive(), "被移除实例的 Worker 必须已关停回收");
    assert!(handle_a.is_alive(), "移除实例 B 不得影响实例 A 的 Worker");

    let a_before_remove = inferred_frames(&manager, cam_id, "inst_a").await;
    let b_sampled_after_remove = b_metrics_before_remove
        .frames_sampled
        .load(Ordering::Relaxed);

    for _ in 0..3 {
        pts += 100;
        publish_and_settle(&manager, &dispatcher, cam_id, "inst_a", pts).await;
    }
    assert!(
        inferred_frames(&manager, cam_id, "inst_a").await > a_before_remove,
        "移除实例 B 后实例 A 必须持续产出结果"
    );
    assert_eq!(
        b_sampled_after_remove,
        b_sampled_before + sampled_delta,
        "被移除实例不得再产生新的采样帧"
    );
    assert!(
        instance_metrics(&manager, cam_id, "inst_b").await.is_none(),
        "被移除实例不得再出现在运行时指标中"
    );

    manager.stop_all_pumps().await;
    let _ = std::fs::remove_dir_all(&temp_dir);
}

/// 目标实例 Worker 替换后，旧 Worker 的迟到结果必须被实例代际栅栏丢弃。
#[tokio::test]
async fn test_replaced_worker_stale_result_is_discarded() {
    let temp_dir = std::env::temp_dir().join(format!(
        "test_pump_replace_{}",
        uuid::Uuid::now_v7().simple()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let manager = Arc::new(PipelineManager::with_evidence_dir(&temp_dir));
    let cam_id = "camera_pump_replace";
    let session = CameraStreamSession::mock(cam_id, "rtsp://mock-sub/live", TransportPolicy::Tcp);
    let dispatcher = session.dispatcher.clone();

    let old_served = Arc::new(AtomicU64::new(0));
    let old_worker = InferenceWorker::new(Arc::new(CountingBackend {
        served: old_served.clone(),
        delay_ms: 400,
    }));
    let old_handle = old_worker.handle();
    let decoder: Box<dyn VideoDecoder + Send> =
        Box::new(MockDecoder::new(cam_id, CodecType::H264, 640, 360));

    manager
        .start_analysis_pump_multi_worker(
            cam_id,
            session.clone(),
            decoder,
            vec![WorkerInstanceConfig {
                instance_id: "inst_slow".to_string(),
                algorithm_id: "algo_slow".to_string(),
                algorithm_type: "detection".to_string(),
                target_fps: 0,
                config_json: None,
            }],
            vec![(
                "inst_slow".to_string(),
                old_handle.clone(),
                Some(old_worker),
            )],
            None,
        )
        .await;

    dispatcher.publish(keyframe_packet(1000));
    let busy = wait_until(2000, || async { old_handle.is_busy() }).await;
    assert!(busy, "旧 Worker 必须已开始处理首批推理");

    let new_served = Arc::new(AtomicU64::new(0));
    let new_worker = InferenceWorker::new(Arc::new(CountingBackend {
        served: new_served.clone(),
        delay_ms: 0,
    }));
    let new_handle = new_worker.handle();
    let replaced = manager
        .replace_pump_instance_worker(cam_id, "inst_slow", new_worker)
        .await
        .expect("分析泵必须处于运行状态");
    assert!(replaced, "目标实例 Worker 替换必须成功");
    assert!(
        !old_handle.is_alive(),
        "被替换的旧 Worker 必须立即停止接收新帧"
    );

    // 旧 Worker 的推理仍在途，完成后必须因代际失配被丢弃。
    let stale_discarded = wait_until(3000, || async {
        instance_metrics(&manager, cam_id, "inst_slow")
            .await
            .is_some_and(|metrics| metrics.stale_results.load(Ordering::Relaxed) >= 1)
    })
    .await;
    assert!(
        stale_discarded,
        "替换后旧 Worker 的迟到结果必须被代际栅栏丢弃"
    );
    assert_eq!(
        old_served.load(Ordering::SeqCst),
        1,
        "旧 Worker 只应服务过替换前的那一帧"
    );
    assert_eq!(
        inferred_frames(&manager, cam_id, "inst_slow").await,
        0,
        "被丢弃的迟到结果不得计入实例推理帧数"
    );

    // 新 Worker 必须接管后续帧。
    dispatcher.publish(keyframe_packet(2000));
    let new_worker_served =
        wait_until(3000, || async { new_served.load(Ordering::SeqCst) >= 1 }).await;
    assert!(new_worker_served, "替换后的新 Worker 必须接管后续推理");
    assert!(new_handle.is_alive(), "新 Worker 必须在替换后保持存活");

    let counted = wait_until(3000, || async {
        inferred_frames(&manager, cam_id, "inst_slow").await == 1
    })
    .await;
    assert!(
        counted,
        "只有新 Worker 的结果可以计入该实例的推理帧数，实际: {}",
        inferred_frames(&manager, cam_id, "inst_slow").await
    );

    manager.stop_all_pumps().await;
    let _ = std::fs::remove_dir_all(&temp_dir);
}
