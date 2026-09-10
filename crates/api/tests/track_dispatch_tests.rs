#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use pipeline::{PipelineAnalysisEvent, PipelineManager, PipelineTrackEvent};
use tokio::sync::broadcast;
use types::{BoundingBox, CameraTracksPayload, TrackedObject, TOPIC_CAMERA_TRACKS};

use api::TrackDispatchService;

fn create_mock_track_event(camera_id: &str, count: usize, timestamp: i64) -> PipelineTrackEvent {
    create_mock_track_event_with_algo(camera_id, "algo_default", count, timestamp)
}

fn create_mock_track_event_with_algo(
    camera_id: &str,
    algo_id: &str,
    count: usize,
    timestamp: i64,
) -> PipelineTrackEvent {
    let mut tracks = Vec::new();
    for i in 0..count {
        tracks.push(TrackedObject {
            track_id: (i + 1) as u64,
            class_id: 0,
            label: "person".to_string(),
            confidence: 0.92,
            bbox: BoundingBox::new(0.1, 0.2, 0.3, 0.6),
            trajectory: vec![(0.2, 0.6)],
        });
    }

    PipelineTrackEvent {
        camera_id: camera_id.to_string(),
        algorithm_id: algo_id.to_string(),
        timestamp,
        tracks,
    }
}

#[tokio::test]
async fn test_track_dispatch_zero_load_when_no_previewers() {
    let pipeline = Arc::new(PipelineManager::new());
    let (tx, mut rx) = broadcast::channel(16);
    let (shutdown_tx, _) = broadcast::channel(16);

    let service = TrackDispatchService::new(pipeline.clone(), tx, shutdown_tx);
    let event = create_mock_track_event("CAM-01", 1, 1741100000000);

    // 默认无预览客户端 (preview_count == 0)，必须静默丢弃
    let sent = service.handle_track_event(&event).await;
    assert!(!sent);

    // 验证无任何 WebSocket 消息下发
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn test_track_dispatch_broadcast_when_previewing() {
    let pipeline = Arc::new(PipelineManager::new());
    let (tx, mut rx) = broadcast::channel(16);
    let (shutdown_tx, _) = broadcast::channel(16);

    let service = TrackDispatchService::new(pipeline.clone(), tx, shutdown_tx);

    // 激活实时预览计数
    pipeline.increment_preview("CAM-01").await;

    let event = create_mock_track_event("CAM-01", 2, 1741100000000);
    let sent = service.handle_track_event(&event).await;
    assert!(sent);

    let ws_event = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("等待 WS 广播超时")
        .expect("接收 WS 广播失败");

    assert_eq!(ws_event.topic, TOPIC_CAMERA_TRACKS);
    let payload: CameraTracksPayload =
        serde_json::from_value(ws_event.payload).expect("反序列化 payload 失败");

    assert_eq!(payload.camera_id, "CAM-01");
    assert_eq!(payload.timestamp, 1741100000000);
    assert_eq!(payload.tracks.len(), 2);
    assert_eq!(payload.tracks[0].track_id, 1);
    assert_eq!(payload.tracks[0].label, "person");
    assert_eq!(payload.tracks[0].bbox, [0.1, 0.2, 0.3, 0.6]);
    assert_eq!(payload.tracks[0].trajectory.len(), 1);
}

#[tokio::test]
async fn test_track_dispatch_throttling() {
    let pipeline = Arc::new(PipelineManager::new());
    let (tx, _rx) = broadcast::channel(16);
    let (shutdown_tx, _) = broadcast::channel(16);

    // 配置最小间隔为 100ms (~10 FPS)
    let service = TrackDispatchService::with_interval(pipeline.clone(), tx, shutdown_tx, 100);
    pipeline.increment_preview("CAM-02").await;

    let event1 = create_mock_track_event("CAM-02", 1, 1741100000000);
    assert!(service.handle_track_event(&event1).await);

    // 紧接着下发第二帧，应被节流丢弃
    let event2 = create_mock_track_event("CAM-02", 1, 1741100000030);
    assert!(!service.handle_track_event(&event2).await);

    // 等待超过 100ms 阈值
    tokio::time::sleep(Duration::from_millis(110)).await;

    // 第三帧下发，应成功广播
    let event3 = create_mock_track_event("CAM-02", 1, 1741100000150);
    assert!(service.handle_track_event(&event3).await);
}

#[tokio::test]
async fn test_track_dispatch_edge_triggered_clear() {
    let pipeline = Arc::new(PipelineManager::new());
    let (tx, mut rx) = broadcast::channel(16);
    let (shutdown_tx, _) = broadcast::channel(16);

    let service = TrackDispatchService::new(pipeline.clone(), tx, shutdown_tx);
    pipeline.increment_preview("CAM-03").await;

    // 1. 发送带目标的帧
    let event_with_tracks = create_mock_track_event("CAM-03", 1, 1741100000000);
    assert!(service.handle_track_event(&event_with_tracks).await);
    let _ = rx.recv().await.unwrap();

    // 2. 目标离开画面，下发空帧：必须立即触发单次清空广播
    let event_empty_1 = create_mock_track_event("CAM-03", 0, 1741100000050);
    assert!(service.handle_track_event(&event_empty_1).await);

    let clear_msg = rx.recv().await.unwrap();
    let payload: CameraTracksPayload = serde_json::from_value(clear_msg.payload).unwrap();
    assert_eq!(payload.camera_id, "CAM-03");
    assert!(payload.tracks.is_empty());

    // 3. 持续无目标帧：应被抑制静默，避免空帧刷屏
    let event_empty_2 = create_mock_track_event("CAM-03", 0, 1741100000100);
    assert!(!service.handle_track_event(&event_empty_2).await);
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn test_track_dispatch_worker_integration() {
    let pipeline = Arc::new(PipelineManager::new());
    let (tx, mut rx) = broadcast::channel(16);
    let (shutdown_tx, _) = broadcast::channel(16);

    let service = Arc::new(TrackDispatchService::new(pipeline.clone(), tx, shutdown_tx));
    let _handle = service.start_worker();

    pipeline.increment_preview("CAM-04").await;

    let event = create_mock_track_event("CAM-04", 1, 1741100000000);
    pipeline.publish_analysis_event(PipelineAnalysisEvent::Tracks(event));

    let ws_event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("等待 WS 广播超时")
        .expect("接收 WS 广播失败");

    assert_eq!(ws_event.topic, TOPIC_CAMERA_TRACKS);
    let payload: CameraTracksPayload = serde_json::from_value(ws_event.payload).unwrap();
    assert_eq!(payload.camera_id, "CAM-04");
    assert_eq!(payload.tracks.len(), 1);
}

#[tokio::test]
async fn test_track_dispatch_multi_instance_aggregation() {
    let pipeline = Arc::new(PipelineManager::new());
    let (tx, mut rx) = broadcast::channel(16);
    let (shutdown_tx, _) = broadcast::channel(16);

    // 最小间隔 50ms
    let service = TrackDispatchService::with_interval(pipeline.clone(), tx, shutdown_tx, 50);
    pipeline.increment_preview("CAM-MULTI").await;

    // 1. 算法 A 产出 1 个目标
    let evt_a = create_mock_track_event_with_algo("CAM-MULTI", "algo_yolo", 1, 1741100000000);
    assert!(service.handle_track_event(&evt_a).await);
    let msg1 = rx.recv().await.unwrap();
    let payload1: CameraTracksPayload = serde_json::from_value(msg1.payload).unwrap();
    assert_eq!(payload1.tracks.len(), 1);

    tokio::time::sleep(Duration::from_millis(60)).await;

    // 2. 算法 B 产出 2 个目标，此时应聚合 A + B 共 3 个目标
    let evt_b = create_mock_track_event_with_algo("CAM-MULTI", "algo_face", 2, 1741100000070);
    assert!(service.handle_track_event(&evt_b).await);
    let msg2 = rx.recv().await.unwrap();
    let payload2: CameraTracksPayload = serde_json::from_value(msg2.payload).unwrap();
    assert_eq!(payload2.tracks.len(), 3);

    tokio::time::sleep(Duration::from_millis(60)).await;

    // 3. 算法 B 目标消失 (0 个目标)，但算法 A 仍然有 1 个目标 -> 不应触发清空，仍应保留算法 A
    let evt_b_empty = create_mock_track_event_with_algo("CAM-MULTI", "algo_face", 0, 1741100000140);
    assert!(service.handle_track_event(&evt_b_empty).await);
    let msg3 = rx.recv().await.unwrap();
    let payload3: CameraTracksPayload = serde_json::from_value(msg3.payload).unwrap();
    assert_eq!(payload3.tracks.len(), 1);

    tokio::time::sleep(Duration::from_millis(60)).await;

    // 4. 算法 A 也消失 (0 个目标) -> 此时全量清空，触发单次清空广播
    let evt_a_empty = create_mock_track_event_with_algo("CAM-MULTI", "algo_yolo", 0, 1741100000210);
    assert!(service.handle_track_event(&evt_a_empty).await);
    let msg4 = rx.recv().await.unwrap();
    let payload4: CameraTracksPayload = serde_json::from_value(msg4.payload).unwrap();
    assert!(payload4.tracks.is_empty());
}
