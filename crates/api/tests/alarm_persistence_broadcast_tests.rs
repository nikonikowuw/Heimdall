#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use db::{AlarmRepo, CameraRepo, CaptureRepo};
use pipeline::snapshot::SnapshotResult;
use pipeline::{
    EvidenceStatus, PipelineAlarmEvent, PipelineAnalysisEvent, PipelineCaptureEvent,
    PipelineManager, TriggeredAlarm,
};
use sea_orm::Set;
use tower::ServiceExt;
use types::{
    BoundingBox, DetectionRuleRole, TrackedObject, TOPIC_ALARM_STATUS_CHANGED,
    TOPIC_ALARM_TRIGGERED,
};

use api::AlarmDispatchService;

async fn setup_test_app() -> (axum::Router, api::AppState, String) {
    let db = db::init_test_db().await.unwrap();
    let pipeline = Arc::new(PipelineManager::new());
    let state = api::AppState::new(db, pipeline);
    api::sync_auth_state(&state).await;

    let password_hash = api::crypto::hash_password_async("adminPassword123".to_string()).await;
    db::AdminUserRepo::create_admin(&state.db, "admin", &password_hash)
        .await
        .unwrap();
    state
        .is_initialized
        .store(true, std::sync::atomic::Ordering::Relaxed);

    let claims = types::AuthClaims {
        sub: "admin".to_string(),
        iat: chrono::Utc::now().timestamp_millis(),
        exp: chrono::Utc::now().timestamp_millis() + 86400000,
    };
    let token = api::crypto::generate_jwt(&claims, &state.get_jwt_secret()).unwrap();

    let app = api::create_app(state.clone());
    (app, state, token)
}

fn create_mock_alarm_event(
    event_id: &str,
    camera_id: &str,
    with_snapshot: bool,
    timestamp: i64,
) -> PipelineAlarmEvent {
    let snapshot = if with_snapshot {
        Some(SnapshotResult {
            image_id: format!("img-{event_id}"),
            crop_image_id: format!("crop-{event_id}"),
            image_rel_path: format!("{camera_id}/full_{event_id}.jpg"),
            crop_image_rel_path: format!("{camera_id}/crop_{event_id}.jpg"),
            file_size_bytes: 10240,
            width: 1920,
            height: 1080,
            is_fallback_sub_stream: false,
        })
    } else {
        None
    };

    let evidence_status = if with_snapshot {
        EvidenceStatus::Ready
    } else {
        EvidenceStatus::Failed
    };

    let evidence_error = if with_snapshot {
        None
    } else {
        Some("Burst decode timeout".to_string())
    };

    PipelineAlarmEvent {
        event_id: event_id.to_string(),
        camera_id: camera_id.to_string(),
        algorithm_id: "mock_algo".to_string(),
        alarm: TriggeredAlarm {
            rule_index: 0,
            role: DetectionRuleRole::Roi,
            tracked_object: TrackedObject {
                track_id: 101,
                label: "person".to_string(),
                confidence: 0.95,
                bbox: BoundingBox::new(0.2, 0.3, 0.4, 0.6),
                class_id: 0,
                trajectory: vec![(0.3, 0.6)],
            },
            occurred_at_ms: timestamp,
        },
        snapshot,
        evidence_status,
        evidence_error,
        timestamp,
    }
}

#[tokio::test]
async fn test_alarm_persistence_and_ws_broadcast_flow() {
    let (_app, state, _token) = setup_test_app().await;

    // 1. 注册测试摄像头以验证 cameraName 解析
    CameraRepo::insert(
        &state.db,
        db::entity::camera::ActiveModel {
            id: sea_orm::NotSet,
            camera_id: Set("CAM-01".to_string()),
            name: Set("东大门通道".to_string()),
            protocol: Set("rtsp".to_string()),
            rtsp_url: Set("rtsp://127.0.0.1/live/main".to_string()),
            sub_rtsp_url: Set("".to_string()),
            stream_mode: Set("auto".to_string()),
            remark: Set("".to_string()),
            last_probe_status: Set("healthy".to_string()),
            last_probe_at: Set(None),
            last_probe_error_code: Set("".to_string()),
            last_success_at: Set(None),
            last_codec: Set("h264".to_string()),
            last_width: Set(1920),
            last_height: Set(1080),
            last_fps: Set(25.0),
            gb28181_device_id: Set(None),
            gb28181_channel_id: Set(None),
            created_at: Set(chrono::Utc::now()),
            updated_at: Set(chrono::Utc::now()),
        },
    )
    .await
    .unwrap();

    // 注册测试算法以验证 alarm_type_id 从算法库解析
    db::AlgorithmRepo::upsert_algorithm(
        &state.db,
        db::UpsertAlgorithmParams {
            algorithm_id: "mock_algo".to_string(),
            name: "入侵检测算法".to_string(),
            algorithm_type: "detection".to_string(),
            alarm_type_id: "INTRUSION".to_string(),
            active_version: "1.0.0".to_string(),
            description: "测试算法".to_string(),
            is_builtin: true,
        },
    )
    .await
    .unwrap();

    // 2. 启动后台告警持久化 Worker 并订阅 WebSocket 广播
    let alarm_svc = Arc::new(AlarmDispatchService::from_state(&state));
    let _worker_handle = alarm_svc.clone().start_worker();

    let mut ws_rx = state.event_broadcaster.subscribe();

    // 3. 模拟管线触发一条告警并发布
    let event_id = uuid::Uuid::new_v4().to_string();
    let mock_event = create_mock_alarm_event(&event_id, "CAM-01", true, 1741100000000);
    state
        .pipeline
        .publish_analysis_event(PipelineAnalysisEvent::Alarm(Box::new(mock_event)));

    // 4. 断言 WebSocket 广播通道接收到 alarm.triggered 消息
    let ws_event = tokio::time::timeout(Duration::from_secs(2), ws_rx.recv())
        .await
        .expect("等待 WS 广播超时")
        .expect("接收 WS 广播失败");

    assert_eq!(ws_event.topic, TOPIC_ALARM_TRIGGERED);
    let payload = ws_event.payload;
    assert_eq!(payload["eventId"], event_id);
    assert_eq!(payload["cameraId"], "CAM-01");
    assert_eq!(payload["cameraName"], "东大门通道");
    assert_eq!(payload["algorithmId"], "mock_algo");
    assert_eq!(payload["alarmTypeId"], "INTRUSION");
    assert_eq!(payload["targetLabel"], "person");
    assert_eq!(payload["ruleType"], "roi");
    assert_eq!(payload["severity"], "warning");
    assert_eq!(payload["occurredAt"], 1741100000000i64);
    assert!(payload["cropImageRelPath"]
        .as_str()
        .unwrap()
        .contains("crop_"));

    // 5. 断言 SQLite alarm_records 表成功持久化
    let alarm_record = AlarmRepo::find_by_event_id(&state.db, &event_id)
        .await
        .unwrap()
        .expect("数据库中未找到告警记录");

    assert_eq!(alarm_record.camera_id, "CAM-01");
    assert_eq!(alarm_record.alarm_type_id, "INTRUSION");
    assert_eq!(alarm_record.status, "unprocessed");
    assert_eq!(alarm_record.target_label, "person");
    assert_eq!(alarm_record.confidence, 0.95);
    assert_eq!(alarm_record.track_id, 101);
    assert_eq!(alarm_record.rule_type, "roi");
    assert!(alarm_record.image_rel_path.contains("full_"));
    assert!(alarm_record.crop_image_rel_path.contains("crop_"));

    // 6. 断言 SQLite capture_records 表同步写入了行迹抓拍凭证
    let captures = CaptureRepo::list_recent(&state.db, Some("CAM-01"), 10, 0)
        .await
        .unwrap();
    assert_eq!(captures.len(), 1);
    let cap = &captures[0];
    assert_eq!(cap.camera_id, "CAM-01");
    assert_eq!(cap.target_label, "person");
    assert_eq!(cap.track_id, 101);
    assert!(cap.crop_image_rel_path.contains("crop_"));
}

#[tokio::test]
async fn test_alarm_persistence_with_failed_evidence() {
    let (_app, state, _token) = setup_test_app().await;

    let alarm_svc = Arc::new(AlarmDispatchService::from_state(&state));
    let _worker_handle = alarm_svc.clone().start_worker();

    let mut ws_rx = state.event_broadcaster.subscribe();

    // 模拟快照生成失败场景（with_snapshot = false）
    let event_id = uuid::Uuid::new_v4().to_string();
    let mock_event = create_mock_alarm_event(&event_id, "CAM-02", false, 1741100050000);
    state
        .pipeline
        .publish_analysis_event(PipelineAnalysisEvent::Alarm(Box::new(mock_event)));

    let ws_event = tokio::time::timeout(Duration::from_secs(2), ws_rx.recv())
        .await
        .expect("等待 WS 广播超时")
        .expect("接收 WS 广播失败");

    assert_eq!(ws_event.topic, TOPIC_ALARM_TRIGGERED);
    assert_eq!(ws_event.payload["cropImageRelPath"], "");
    assert_eq!(ws_event.payload["imageRelPath"], "");

    // 断言数据库仍保留了告警事实（图片路径为空字符串）
    let alarm_record = AlarmRepo::find_by_event_id(&state.db, &event_id)
        .await
        .unwrap()
        .expect("快照失败时告警事实绝不能丢弃");

    assert_eq!(alarm_record.event_id, event_id);
    assert_eq!(alarm_record.image_rel_path, "");
    assert_eq!(alarm_record.crop_image_rel_path, "");
    assert_eq!(alarm_record.status, "unprocessed");

    // 快照失败时，行迹抓拍表不应产生空路径的幽灵抓拍记录（图像资产库保持纯净）
    let captures = CaptureRepo::list_recent(&state.db, Some("CAM-02"), 10, 0)
        .await
        .unwrap();
    assert_eq!(captures.len(), 0);
}

#[tokio::test]
async fn test_cold_start_pending_alarm_drain() {
    let (_app, state, _token) = setup_test_app().await;

    // 1. 在 Worker 启动之前，管线已经触发并积压了 3 个告警事件
    let mut event_ids = Vec::new();
    for i in 0..3 {
        let event_id = format!("cold-start-evt-{i}");
        event_ids.push(event_id.clone());
        let evt = create_mock_alarm_event(&event_id, "CAM-03", true, 1741100100000 + i * 1000);
        state
            .pipeline
            .publish_analysis_event(PipelineAnalysisEvent::Alarm(Box::new(evt)));
    }

    assert_eq!(state.pipeline.pending_alarm_event_count(), 3);

    // 2. 启动 Worker，应当自动触发 drain_and_persist_pending 补偿入库
    let alarm_svc = Arc::new(AlarmDispatchService::from_state(&state));
    let _worker_handle = alarm_svc.clone().start_worker();

    // 等待 Worker 处理完成
    tokio::time::sleep(Duration::from_millis(150)).await;

    // 3. 验证内存补偿队列已被完全清空
    assert_eq!(state.pipeline.pending_alarm_event_count(), 0);

    // 4. 验证积压的 3 条告警均成功落库
    for event_id in event_ids {
        let record = AlarmRepo::find_by_event_id(&state.db, &event_id)
            .await
            .unwrap();
        assert!(record.is_some(), "冷启动积压告警 {event_id} 必须成功落库");
    }
}

#[tokio::test]
async fn test_update_alarm_status_and_ws_broadcast() {
    let (app, state, token) = setup_test_app().await;

    // 1. 在 DB 插入一条待处理告警
    let active = db::entity::alarm::ActiveModel {
        id: sea_orm::NotSet,
        event_id: Set("evt-status-test-1".to_string()),
        camera_id: Set("CAM-01".to_string()),
        alarm_type_id: Set("rule_0".to_string()),
        occurred_at: Set(chrono::Utc::now()),
        target_label: Set("car".to_string()),
        confidence: Set(0.88),
        track_id: Set(202),
        bbox_json: Set("{}".to_string()),
        image_id: Set("img".to_string()),
        image_rel_path: Set("path".to_string()),
        crop_image_id: Set("crop".to_string()),
        crop_image_rel_path: Set("crop_path".to_string()),
        rule_type: Set("line".to_string()),
        severity: Set("warning".to_string()),
        status: Set("unprocessed".to_string()),
        handled_at: Set(None),
        created_at: Set(chrono::Utc::now()),
    };
    let inserted = AlarmRepo::insert(&state.db, active).await.unwrap();

    // 2. 订阅 WebSocket 广播
    let mut ws_rx = state.event_broadcaster.subscribe();

    // 3. 调用 PUT /api/v1/alarms/{id}/status 标记为 processed
    let req = Request::builder()
        .method("PUT")
        .uri(format!("/api/v1/alarms/{}/status", inserted.id))
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"status":"processed"}"#))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["code"], 0);
    assert_eq!(json["data"]["status"], "processed");
    assert!(json["data"]["handledAt"].is_number());

    // 4. 验证 WebSocket 收到 alarm.status_changed 广播
    let ws_event = tokio::time::timeout(Duration::from_secs(2), ws_rx.recv())
        .await
        .expect("等待 WS 广播超时")
        .expect("接收 WS 广播失败");

    assert_eq!(ws_event.topic, TOPIC_ALARM_STATUS_CHANGED);
    assert_eq!(ws_event.payload["id"], inserted.id);
    assert_eq!(ws_event.payload["eventId"], "evt-status-test-1");
    assert_eq!(ws_event.payload["status"], "processed");
    assert!(ws_event.payload["handledAt"].is_number());
}

#[tokio::test]
async fn test_recognition_capture_event_persistence_without_alarm() {
    let (_app, state, _token) = setup_test_app().await;

    let alarm_svc = Arc::new(AlarmDispatchService::from_state(&state));
    let _alarm_worker = alarm_svc.clone().start_worker();

    let capture_svc = Arc::new(api::CaptureDispatchService::with_options(
        state.db.clone(),
        state.pipeline.clone(),
        state.shutdown_tx.clone(),
        1,  // 批量大小设为 1，即刻落库便于测试断言
        50, // 50ms 刷新周期
    ));
    let _capture_worker = capture_svc.clone().start_worker();

    let mut ws_rx = state.event_broadcaster.subscribe();

    // 构建一个识别类客观通行抓拍事件
    let capture_id = uuid::Uuid::new_v4().to_string();
    let mock_capture = PipelineCaptureEvent {
        capture_id: capture_id.clone(),
        camera_id: "CAM-01".to_string(),
        algorithm_id: "face_recognition".to_string(),
        tracked_object: TrackedObject {
            track_id: 301,
            class_id: 0,
            label: "face".to_string(),
            confidence: 0.98,
            bbox: BoundingBox::new(0.3, 0.3, 0.5, 0.5),
            trajectory: vec![(0.4, 0.4)],
        },
        snapshot: Some(SnapshotResult {
            image_id: "snap_full_301".to_string(),
            image_rel_path: "2026/03/04/CAM-01/full_301.jpg".to_string(),
            crop_image_id: "snap_crop_301".to_string(),
            crop_image_rel_path: "2026/03/04/CAM-01/crop_301.jpg".to_string(),
            file_size_bytes: 10240,
            width: 1920,
            height: 1080,
            is_fallback_sub_stream: false,
        }),
        timestamp: 1741100060000,
    };

    // 发布客观通行抓拍事件
    state
        .pipeline
        .publish_analysis_event(PipelineAnalysisEvent::Capture(Box::new(mock_capture)));

    // 等待 Capture Worker 异步落库
    tokio::time::sleep(Duration::from_millis(150)).await;

    // 1. 断言 SQLite capture_records 表写入了行迹抓拍凭证
    let captures = CaptureRepo::list_recent(&state.db, Some("CAM-01"), 10, 0)
        .await
        .unwrap();
    assert_eq!(captures.len(), 1, "通行抓拍凭证必须成功落库");
    let cap = &captures[0];
    assert_eq!(cap.capture_id, capture_id);
    assert_eq!(cap.camera_id, "CAM-01");
    assert_eq!(cap.target_label, "face");
    assert_eq!(cap.track_id, 301);
    assert_eq!(cap.image_rel_path, "2026/03/04/CAM-01/full_301.jpg");
    assert_eq!(cap.crop_image_rel_path, "2026/03/04/CAM-01/crop_301.jpg");

    // 2. 关键核心断言：alarm_records 表绝对不能产生虚假违规告警！
    let alarms = AlarmRepo::list_recent(&state.db, Some("CAM-01"), 10, 0)
        .await
        .unwrap();
    assert_eq!(alarms.len(), 0, "识别类通行抓拍绝对不能产生违规告警记录");

    // 3. 断言 WebSocket 未广播 alarm.triggered 报警
    let timeout_res = tokio::time::timeout(Duration::from_millis(100), ws_rx.recv()).await;
    assert!(timeout_res.is_err(), "通行抓拍绝对不能向客户端广播报警弹窗");
}

#[tokio::test]
async fn test_cold_start_pending_capture_drain_and_batch_persistence() {
    let (_app, state, _token) = setup_test_app().await;

    // 1. 在 Capture Worker 启动之前，管线已经触发并积压了 3 个通行抓拍事件
    let mut capture_ids = Vec::new();
    for i in 0..3 {
        let capture_id = format!("cold-cap-{i}");
        capture_ids.push(capture_id.clone());
        let mock_capture = PipelineCaptureEvent {
            capture_id: capture_id.clone(),
            camera_id: "CAM-01".to_string(),
            algorithm_id: "face_recognition".to_string(),
            tracked_object: TrackedObject {
                track_id: 400 + i as u64,
                class_id: 0,
                label: "face".to_string(),
                confidence: 0.95,
                bbox: BoundingBox::new(0.2, 0.2, 0.4, 0.4),
                trajectory: vec![],
            },
            snapshot: Some(SnapshotResult {
                image_id: format!("img_{i}"),
                image_rel_path: format!("2026/03/04/CAM-01/face_{i}.jpg"),
                crop_image_id: format!("crop_{i}"),
                crop_image_rel_path: format!("2026/03/04/CAM-01/crop_{i}.jpg"),
                file_size_bytes: 1024,
                width: 1920,
                height: 1080,
                is_fallback_sub_stream: false,
            }),
            timestamp: 1741100200000 + (i as i64) * 1000,
        };
        state
            .pipeline
            .publish_analysis_event(PipelineAnalysisEvent::Capture(Box::new(mock_capture)));
    }

    assert_eq!(state.pipeline.pending_capture_event_count(), 3);

    // 2. 启动 Capture Worker，应当自动触发 drain_and_persist_pending 批量补偿入库
    let capture_svc = Arc::new(api::CaptureDispatchService::with_options(
        state.db.clone(),
        state.pipeline.clone(),
        state.shutdown_tx.clone(),
        10,
        100,
    ));
    let _capture_worker = capture_svc.clone().start_worker();

    // 等待 Worker 补偿处理完成
    tokio::time::sleep(Duration::from_millis(150)).await;

    // 3. 验证内存补偿队列已被完全排空
    assert_eq!(state.pipeline.pending_capture_event_count(), 0);

    // 4. 验证积压的 3 条抓拍均已成功批量写入 capture_records
    let captures = CaptureRepo::list_recent(&state.db, Some("CAM-01"), 10, 0)
        .await
        .unwrap();
    assert_eq!(captures.len(), 3, "所有冷启动积压通行抓拍必须全部落库");
}
