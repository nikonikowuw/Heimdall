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
    BoundingBox, DetectionRuleRole, EvidenceImageSource, EvidenceImageStream, FaceDetail,
    TrackedObject, TOPIC_ALARM_STATUS_CHANGED, TOPIC_ALARM_TRIGGERED,
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
            body_crop_image_id: String::new(),
            body_crop_image_rel_path: String::new(),
            file_size_bytes: 10240,
            width: 1920,
            height: 1080,
            // 证据图与事件同帧（INV-3 审计字段）
            frame_pts_ms: timestamp,
            image_source: EvidenceImageSource::Targeted,
            image_stream: EvidenceImageStream::Main,
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
                quality_score: None,
                embedding: None,
                bbox: BoundingBox::new(0.2, 0.3, 0.4, 0.6),
                face: None,
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
    let event_id = uuid::Uuid::now_v7().to_string();
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

    // 6. 断言 SQLite capture_records 表不应重复写入检测类告警记录（三支柱严格隔离：检测类只落 alarm_records）
    let captures = CaptureRepo::list_recent(&state.db, Some("CAM-01"), 10, 0)
        .await
        .unwrap();
    assert_eq!(captures.len(), 0, "检测类安全告警不得重复写入通行抓拍表");
}

#[tokio::test]
async fn test_alarm_persistence_with_failed_evidence() {
    let (_app, state, _token) = setup_test_app().await;

    let alarm_svc = Arc::new(AlarmDispatchService::from_state(&state));
    let _worker_handle = alarm_svc.clone().start_worker();

    let mut ws_rx = state.event_broadcaster.subscribe();

    // 模拟快照生成失败场景（with_snapshot = false）
    let event_id = uuid::Uuid::now_v7().to_string();
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

    // 构建一个识别类客观通行抓拍事件（M2 结算峰值候选帧：内存候选字节落盘 + 融合模板特征）
    let capture_id = uuid::Uuid::now_v7().to_string();
    let mock_capture = PipelineCaptureEvent {
        capture_id: capture_id.clone(),
        camera_id: "CAM-01".to_string(),
        algorithm_id: "face_recognition".to_string(),
        tracked_object: TrackedObject {
            track_id: 301,
            class_id: 0,
            label: "face".to_string(),
            confidence: 0.98,
            quality_score: Some(0.86),
            embedding: None,
            bbox: BoundingBox::new(0.3, 0.3, 0.5, 0.5),
            face: Some(FaceDetail {
                bbox: BoundingBox::new(0.32, 0.32, 0.48, 0.48),
                confidence: 0.98,
                quality_score: Some(0.86),
                fused_count: Some(4),
                template_quality: Some(0.72),
                template_mature: Some(true),
                pseudo_body: None,
                embedding: None,
            }),
            trajectory: vec![(0.4, 0.4)],
        },
        snapshot: Some(SnapshotResult {
            image_id: "snap_full_301".to_string(),
            image_rel_path: "2026/03/04/CAM-01/full_301.jpg".to_string(),
            crop_image_id: "snap_crop_301".to_string(),
            crop_image_rel_path: "2026/03/04/CAM-01/crop_301.jpg".to_string(),
            body_crop_image_id: "snap_body_301".to_string(),
            body_crop_image_rel_path: "2026/03/04/CAM-01/body_301.jpg".to_string(),
            file_size_bytes: 10240,
            width: 1920,
            height: 1080,
            // 峰值帧与事件同一刻（结算前最后一帧即峰值）
            frame_pts_ms: 1741100060000,
            image_source: EvidenceImageSource::PeakCandidate,
            image_stream: EvidenceImageStream::Sub,
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
    assert_eq!(
        cap.body_crop_image_rel_path, "2026/03/04/CAM-01/body_301.jpg",
        "人体特写（人工复查看衣着的主体证据）必须与全景、人脸特写一同落库"
    );
    // 目标 3 可追溯性：来源路径/码流/帧 PTS 与所用融合模板元数据必须全部落库
    assert_eq!(cap.image_source, "peak_candidate");
    assert_eq!(cap.image_stream, "sub");
    assert_eq!(cap.image_pts_ms, 1741100060000);
    assert_eq!(cap.fused_count, Some(4));
    assert_eq!(cap.template_quality, Some(0.72));

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
async fn test_face_recognition_with_pseudo_body_purges_body_and_sets_face_label() {
    let (_app, state, _token) = setup_test_app().await;

    let capture_svc = Arc::new(api::CaptureDispatchService::from_state(&state));
    let _capture_worker = capture_svc.clone().start_worker();

    // 构造一个带有伪造人体框 (pseudo_body: true) 的人脸抓拍事件，原本 label 为 "person"
    let capture_id = uuid::Uuid::now_v7().to_string();
    let mock_capture = PipelineCaptureEvent {
        capture_id: capture_id.clone(),
        camera_id: "CAM-PSEUDO-01".to_string(),
        algorithm_id: "face_recognition".to_string(),
        tracked_object: TrackedObject {
            track_id: 501,
            class_id: 0,
            label: "person".to_string(),
            confidence: 0.96,
            quality_score: Some(0.88),
            embedding: None,
            // 伪造的人体躯干框（贴到画面下沿）
            bbox: BoundingBox::new(0.2, 0.2, 0.8, 1.0),
            face: Some(FaceDetail {
                bbox: BoundingBox::new(0.35, 0.22, 0.55, 0.42),
                confidence: 0.98,
                quality_score: Some(0.88),
                fused_count: Some(2),
                template_quality: Some(0.85),
                template_mature: Some(true),
                pseudo_body: Some(true),
                embedding: None,
            }),
            trajectory: vec![(0.4, 0.4)],
        },
        snapshot: Some(SnapshotResult {
            image_id: "snap_full_501".to_string(),
            image_rel_path: "2026/03/04/CAM-01/full_501.jpg".to_string(),
            crop_image_id: "snap_crop_501".to_string(),
            crop_image_rel_path: "2026/03/04/CAM-01/crop_501.jpg".to_string(),
            body_crop_image_id: "snap_body_501".to_string(),
            body_crop_image_rel_path: "2026/03/04/CAM-01/body_501.jpg".to_string(),
            file_size_bytes: 10240,
            width: 1920,
            height: 1080,
            frame_pts_ms: 1741100070000,
            image_source: EvidenceImageSource::PeakCandidate,
            image_stream: EvidenceImageStream::Sub,
        }),
        timestamp: 1741100070000,
    };

    state
        .pipeline
        .publish_analysis_event(PipelineAnalysisEvent::Capture(Box::new(mock_capture)));

    tokio::time::sleep(Duration::from_millis(200)).await;

    let captures = CaptureRepo::list_recent(&state.db, Some("CAM-PSEUDO-01"), 10, 0)
        .await
        .unwrap();
    assert_eq!(captures.len(), 1);
    let cap = &captures[0];

    // 核心断言 1: target_label 必须自动修正为 "face"
    assert_eq!(cap.target_label, "face");

    // 核心断言 2: 伪造人体特写必须被清空剔除
    assert_eq!(cap.body_crop_image_rel_path, "");
    assert_eq!(cap.body_crop_image_id, "");
    assert_eq!(cap.crop_image_rel_path, "2026/03/04/CAM-01/crop_501.jpg");

    // 核心断言 3: bbox_json 中不得包含伪造的 "body"
    assert!(!cap.bbox_json.contains("\"body\""));
    assert!(cap.bbox_json.contains("\"face\""));
}

/// 背身/低头的人（有人体检测、无人脸）必须落抓拍记录，且带人体特写。
///
/// 这是抓拍语义从「人脸通行证据」收敛为「人体通行证据」的核心回归：曾经的准入判定
/// 直接以人脸存在为前提，导致刻意回避镜头的目标彻底不留证据。此处同时锁定质量分口径
/// ——无脸且算法未上报质量分时取归一化人体框面积，而不是置信度（置信度与"图清不清楚"无关）。
#[tokio::test]
async fn test_faceless_person_capture_persists_body_crop_and_area_quality() {
    let (_app, state, _token) = setup_test_app().await;

    let capture_svc = Arc::new(api::CaptureDispatchService::with_options(
        state.db.clone(),
        state.pipeline.clone(),
        state.shutdown_tx.clone(),
        1,
        50,
    ));
    let _capture_worker = capture_svc.clone().start_worker();

    let capture_id = uuid::Uuid::now_v7().to_string();
    let bbox = BoundingBox::new(0.30, 0.30, 0.50, 0.60);
    let mock_capture = PipelineCaptureEvent {
        capture_id: capture_id.clone(),
        camera_id: "CAM-09".to_string(),
        algorithm_id: "face_recognition".to_string(),
        tracked_object: TrackedObject {
            track_id: 909,
            class_id: 0,
            label: "person".to_string(),
            confidence: 0.93,
            // 算法包未上报目标级质量分：必须回退到归一化面积，而不是置信度
            quality_score: None,
            embedding: None,
            bbox,
            face: None,
            trajectory: vec![],
        },
        snapshot: Some(SnapshotResult {
            image_id: "snap_full_909".to_string(),
            image_rel_path: "2026/03/04/CAM-09/full_909.jpg".to_string(),
            crop_image_id: String::new(),
            crop_image_rel_path: String::new(),
            body_crop_image_id: "snap_body_909".to_string(),
            body_crop_image_rel_path: "2026/03/04/CAM-09/body_909.jpg".to_string(),
            file_size_bytes: 8192,
            width: 640,
            height: 360,
            frame_pts_ms: 1741100061000,
            image_source: EvidenceImageSource::PeakCandidate,
            image_stream: EvidenceImageStream::Sub,
        }),
        timestamp: 1741100061000,
    };

    state
        .pipeline
        .publish_analysis_event(PipelineAnalysisEvent::Capture(Box::new(mock_capture)));

    tokio::time::sleep(Duration::from_millis(150)).await;

    let captures = CaptureRepo::list_recent(&state.db, Some("CAM-09"), 10, 0)
        .await
        .unwrap();
    assert_eq!(captures.len(), 1, "背身/低头的通行抓拍必须落库");
    let cap = &captures[0];
    assert_eq!(cap.capture_id, capture_id);
    assert_eq!(cap.target_label, "person");
    assert!(cap.crop_image_rel_path.is_empty(), "无人脸 ⇒ 无人脸特写");
    assert_eq!(
        cap.body_crop_image_rel_path, "2026/03/04/CAM-09/body_909.jpg",
        "无人脸时必须有人体特写，否则人工复核无图可看"
    );
    let expected_area = bbox.area();
    assert!(
        (cap.quality_score - expected_area).abs() < 1e-6,
        "无脸且无目标质量分时质量分必须取归一化人体框面积 {expected_area}，实际 {}",
        cap.quality_score
    );

    // 无脸事件不得产生虚假违规告警
    let alarms = AlarmRepo::list_recent(&state.db, Some("CAM-09"), 10, 0)
        .await
        .unwrap();
    assert_eq!(alarms.len(), 0);
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
                quality_score: Some(0.85),
                embedding: None,
                bbox: BoundingBox::new(0.2, 0.2, 0.4, 0.4),
                face: None,
                trajectory: vec![],
            },
            snapshot: Some(SnapshotResult {
                image_id: format!("img_{i}"),
                image_rel_path: format!("2026/03/04/CAM-01/face_{i}.jpg"),
                crop_image_id: format!("crop_{i}"),
                crop_image_rel_path: format!("2026/03/04/CAM-01/crop_{i}.jpg"),
                body_crop_image_id: String::new(),
                body_crop_image_rel_path: String::new(),
                file_size_bytes: 1024,
                width: 1920,
                height: 1080,
                frame_pts_ms: 1741100200000 + (i as i64) * 1000,
                image_source: EvidenceImageSource::Targeted,
                image_stream: EvidenceImageStream::Sub,
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
    // 冷启动补偿队列与实时通道共用同一落库映射：来源标识同样不能丢
    assert!(
        captures
            .iter()
            .all(|c| c.image_source == "targeted" && c.image_stream == "sub"),
        "补偿落库必须保留证据来源标识"
    );
}

#[tokio::test]
async fn test_count_and_batch_update_alarm_status_api() {
    let (app, state, token) = setup_test_app().await;

    // 1. 插入 3 条测试告警，2 条未处理，1 条已处理
    let a1 = db::entity::alarm::ActiveModel {
        id: sea_orm::NotSet,
        event_id: Set("batch-evt-1".to_string()),
        camera_id: Set("CAM-01".to_string()),
        alarm_type_id: Set("rule_1".to_string()),
        occurred_at: Set(chrono::Utc::now()),
        target_label: Set("person".to_string()),
        confidence: Set(0.95),
        track_id: Set(101),
        bbox_json: Set("{}".to_string()),
        image_id: Set("img1".to_string()),
        image_rel_path: Set("path1".to_string()),
        crop_image_id: Set("crop1".to_string()),
        crop_image_rel_path: Set("crop_path1".to_string()),
        rule_type: Set("roi".to_string()),
        severity: Set("critical".to_string()),
        status: Set("unprocessed".to_string()),
        handled_at: Set(None),
        created_at: Set(chrono::Utc::now()),
    };
    let a2 = db::entity::alarm::ActiveModel {
        id: sea_orm::NotSet,
        event_id: Set("batch-evt-2".to_string()),
        camera_id: Set("CAM-01".to_string()),
        alarm_type_id: Set("rule_1".to_string()),
        occurred_at: Set(chrono::Utc::now()),
        target_label: Set("car".to_string()),
        confidence: Set(0.90),
        track_id: Set(102),
        bbox_json: Set("{}".to_string()),
        image_id: Set("img2".to_string()),
        image_rel_path: Set("path2".to_string()),
        crop_image_id: Set("crop2".to_string()),
        crop_image_rel_path: Set("crop_path2".to_string()),
        rule_type: Set("line".to_string()),
        severity: Set("warning".to_string()),
        status: Set("unprocessed".to_string()),
        handled_at: Set(None),
        created_at: Set(chrono::Utc::now()),
    };
    let a3 = db::entity::alarm::ActiveModel {
        id: sea_orm::NotSet,
        event_id: Set("batch-evt-3".to_string()),
        camera_id: Set("CAM-02".to_string()),
        alarm_type_id: Set("rule_2".to_string()),
        occurred_at: Set(chrono::Utc::now()),
        target_label: Set("person".to_string()),
        confidence: Set(0.92),
        track_id: Set(103),
        bbox_json: Set("{}".to_string()),
        image_id: Set("img3".to_string()),
        image_rel_path: Set("path3".to_string()),
        crop_image_id: Set("crop3".to_string()),
        crop_image_rel_path: Set("crop_path3".to_string()),
        rule_type: Set("roi".to_string()),
        severity: Set("critical".to_string()),
        status: Set("processed".to_string()),
        handled_at: Set(Some(chrono::Utc::now())),
        created_at: Set(chrono::Utc::now()),
    };

    let m1 = AlarmRepo::insert(&state.db, a1).await.unwrap();
    let m2 = AlarmRepo::insert(&state.db, a2).await.unwrap();
    let _m3 = AlarmRepo::insert(&state.db, a3).await.unwrap();

    // 2. 测试 GET /api/v1/alarms/count
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/alarms/count?status=unprocessed")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["data"]["total"], 2);

    // 带 target_label 过滤测试 count
    let req = Request::builder()
        .method("GET")
        .uri("/api/v1/alarms/count?target_label=person")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["data"]["total"], 2);

    // 3. 测试 POST /api/v1/alarms/batch-status 批量将 m1 和 m2 标记为 processed
    let mut ws_rx = state.event_broadcaster.subscribe();
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/alarms/batch-status")
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "ids": [m1.id, m2.id],
                "status": "processed"
            })
            .to_string(),
        ))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["code"], 0);
    assert_eq!(json["data"].as_array().unwrap().len(), 2);

    // 检查 DB
    let rec1 = AlarmRepo::find_by_event_id(&state.db, "batch-evt-1")
        .await
        .unwrap()
        .unwrap();
    let rec2 = AlarmRepo::find_by_event_id(&state.db, "batch-evt-2")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rec1.status, "processed");
    assert_eq!(rec2.status, "processed");

    // 检查 WS 广播接收
    let evt1 = ws_rx.recv().await.unwrap();
    assert_eq!(evt1.topic, TOPIC_ALARM_STATUS_CHANGED);
    let evt2 = ws_rx.recv().await.unwrap();
    assert_eq!(evt2.topic, TOPIC_ALARM_STATUS_CHANGED);
}

#[tokio::test]
async fn test_face_recognition_below_threshold_does_not_persist_invalid_recognition() {
    let (_app, state, _token) = setup_test_app().await;

    // 1. 底库注册 5 名人员，每个人的向量与待测人脸向量保持较低相似度 (0.20 ~ 0.45，均低于默认确认阈值 0.75)
    let mut faces = Vec::new();
    for i in 1..=5 {
        let mut vec = [0.0f32; 512];
        // 设第 0 维和第 i 维，产生可控的点积分数
        vec[0] = 0.50 - (i as f32) * 0.05; // 0.45, 0.40, 0.35, 0.30, 0.25
        vec[i] = (1.0 - vec[0] * vec[0]).sqrt(); // 保持单位模长
        faces.push(api::RegisteredFace::from_512(
            format!("sub_mock_{i}"),
            format!("Mock Person {i}"),
            format!("face_mock_{i}"),
            format!("galleries/sub_mock_{i}/photo.jpg"),
            vec,
        ));
    }
    state.gallery_index.upsert_faces(faces).await;
    assert_eq!(state.gallery_index.count().await, 5);

    // 2. 构造查询向量: 单位向量 [1.0, 0.0, 0.0, ...]
    // 与候选人的余弦相似度正好等于候选人 vec[0] (0.45, 0.40, 0.35, 0.30, 0.25)，全部低于确认阈值 0.75
    let mut query_vec = [0.0f32; 512];
    query_vec[0] = 1.0;
    let query_embedding = std::sync::Arc::from(api::embedding_to_le_bytes(&query_vec));

    let capture_svc = Arc::new(api::CaptureDispatchService::from_state(&state));
    let _capture_worker = capture_svc.clone().start_worker();

    let mut ws_rx = state.event_broadcaster.subscribe();

    // 3. 发布携带该 embedding 的人脸通行抓拍事件
    let capture_id = uuid::Uuid::now_v7().to_string();
    let mock_capture = PipelineCaptureEvent {
        capture_id: capture_id.clone(),
        camera_id: "CAM-REC-01".to_string(),
        algorithm_id: "face_recognition".to_string(),
        tracked_object: TrackedObject {
            track_id: 888,
            class_id: 0,
            label: "face".to_string(),
            confidence: 0.95,
            quality_score: Some(0.50), // 动态微调量为 0
            embedding: Some(query_embedding),
            bbox: BoundingBox::new(0.2, 0.2, 0.4, 0.4),
            face: None,
            trajectory: vec![(0.3, 0.3)],
        },
        snapshot: Some(SnapshotResult {
            image_id: "snap_full_888".to_string(),
            image_rel_path: "2026/03/04/CAM-REC-01/full_888.jpg".to_string(),
            crop_image_id: "snap_crop_888".to_string(),
            crop_image_rel_path: "2026/03/04/CAM-REC-01/crop_888.jpg".to_string(),
            body_crop_image_id: String::new(),
            body_crop_image_rel_path: String::new(),
            file_size_bytes: 10240,
            width: 1920,
            height: 1080,
            frame_pts_ms: 1741100088000,
            image_source: EvidenceImageSource::PeakCandidate,
            image_stream: EvidenceImageStream::Sub,
        }),
        timestamp: 1741100088000,
    };

    state
        .pipeline
        .publish_analysis_event(PipelineAnalysisEvent::Capture(Box::new(mock_capture)));

    // 等待异步识别队列与落库完成
    tokio::time::sleep(Duration::from_millis(250)).await;

    // 4. 验证行迹抓拍已正常落库
    let captures = CaptureRepo::list_recent(&state.db, Some("CAM-REC-01"), 10, 0)
        .await
        .unwrap();
    assert_eq!(captures.len(), 1, "通行抓拍必须正常落库");

    // 5. 核心断言：未达确认阈值的人脸绝对不落库识别对账记录 (不把无效记录写入识别对账)
    let recognitions = db::RecognitionRepo::list_recent(&state.db, Some("CAM-REC-01"), 10, 0)
        .await
        .unwrap();
    assert_eq!(
        recognitions.len(),
        0,
        "未达确认阈值的人脸判定为无效比对，不得进入识别对账"
    );

    // 6. 验证 WebSocket 不广播 TOPIC_RECOGNITION_MATCHED 事件
    let mut found_ws_match = false;
    while let Ok(msg) = ws_rx.try_recv() {
        if msg.topic == types::TOPIC_RECOGNITION_MATCHED {
            found_ws_match = true;
            break;
        }
    }
    assert!(
        !found_ws_match,
        "未达阈值的无效人脸不得通过 WebSocket 广播 TOPIC_RECOGNITION_MATCHED 事件"
    );
}

#[tokio::test]
async fn test_face_recognition_above_threshold_persists_unconditional_top5_confirmed() {
    let (_app, state, _token) = setup_test_app().await;

    // 1. 底库注册 5 名人员，首位候选人相似度 0.85 (高于阈值 0.75)，其余候选人分数较低 (0.40 ~ 0.25)
    let mut faces = Vec::new();
    // sub_mock_1: 相似度 0.85
    let mut vec1 = [0.0f32; 512];
    vec1[0] = 0.85;
    vec1[1] = (1.0 - 0.85 * 0.85f32).sqrt();
    faces.push(api::RegisteredFace::from_512(
        "sub_mock_1".to_string(),
        "Mock Person 1".to_string(),
        "face_mock_1".to_string(),
        "galleries/sub_mock_1/photo.jpg".to_string(),
        vec1,
    ));

    for i in 2..=5 {
        let mut vec = [0.0f32; 512];
        vec[0] = 0.50 - (i as f32) * 0.05; // 0.40, 0.35, 0.30, 0.25
        vec[i] = (1.0 - vec[0] * vec[0]).sqrt();
        faces.push(api::RegisteredFace::from_512(
            format!("sub_mock_{i}"),
            format!("Mock Person {i}"),
            format!("face_mock_{i}"),
            format!("galleries/sub_mock_{i}/photo.jpg"),
            vec,
        ));
    }
    state.gallery_index.upsert_faces(faces).await;
    assert_eq!(state.gallery_index.count().await, 5);

    // 2. 构造查询向量: 单位向量 [1.0, 0.0, 0.0, ...]，与 sub_mock_1 相似度为 0.85
    let mut query_vec = [0.0f32; 512];
    query_vec[0] = 1.0;
    let query_embedding = std::sync::Arc::from(api::embedding_to_le_bytes(&query_vec));

    let capture_svc = Arc::new(api::CaptureDispatchService::from_state(&state));
    let _capture_worker = capture_svc.clone().start_worker();

    let mut ws_rx = state.event_broadcaster.subscribe();

    // 3. 发布携带该 embedding 的人脸通行抓拍事件
    let capture_id = uuid::Uuid::now_v7().to_string();
    let mock_capture = PipelineCaptureEvent {
        capture_id: capture_id.clone(),
        camera_id: "CAM-REC-02".to_string(),
        algorithm_id: "face_recognition".to_string(),
        tracked_object: TrackedObject {
            track_id: 999,
            class_id: 0,
            label: "face".to_string(),
            confidence: 0.98,
            quality_score: Some(0.50), // 动态微调量为 0
            embedding: Some(query_embedding),
            bbox: BoundingBox::new(0.2, 0.2, 0.4, 0.4),
            face: None,
            trajectory: vec![(0.3, 0.3)],
        },
        snapshot: Some(SnapshotResult {
            image_id: "snap_full_999".to_string(),
            image_rel_path: "2026/03/04/CAM-REC-02/full_999.jpg".to_string(),
            crop_image_id: "snap_crop_999".to_string(),
            crop_image_rel_path: "2026/03/04/CAM-REC-02/crop_999.jpg".to_string(),
            body_crop_image_id: String::new(),
            body_crop_image_rel_path: String::new(),
            file_size_bytes: 10240,
            width: 1920,
            height: 1080,
            frame_pts_ms: 1741100099000,
            image_source: EvidenceImageSource::PeakCandidate,
            image_stream: EvidenceImageStream::Sub,
        }),
        timestamp: 1741100099000,
    };

    state
        .pipeline
        .publish_analysis_event(PipelineAnalysisEvent::Capture(Box::new(mock_capture)));

    tokio::time::sleep(Duration::from_millis(250)).await;

    // 4. 验证行迹抓拍已落库
    let captures = CaptureRepo::list_recent(&state.db, Some("CAM-REC-02"), 10, 0)
        .await
        .unwrap();
    assert_eq!(captures.len(), 1);

    // 5. 核心断言：Top-1 相似度 (0.85) 达到阈值 (0.75)，必须入识别对账且状态为 confirmed
    let recognitions = db::RecognitionRepo::list_recent(&state.db, Some("CAM-REC-02"), 10, 0)
        .await
        .unwrap();
    assert_eq!(recognitions.len(), 1, "达标人脸必须落库识别对账记录");
    let rec = &recognitions[0];
    assert_eq!(rec.status, "confirmed", "达标人脸应直接判定为 confirmed");
    assert_eq!(rec.subject_id, "sub_mock_1");
    let expected_calibrated = api::gallery_index::megvii_calibrate_cosine(0.85);
    assert!((rec.similarity - expected_calibrated).abs() < 1e-4);

    // 6. 核心断言：必须保存无条件的完整 Top-5 候选人 (哪怕第 2~5 位分数低)
    let candidates_json = rec
        .candidates_json
        .as_deref()
        .expect("candidates_json must exist");
    let cands: Vec<types::FaceCandidateItem> = serde_json::from_str(candidates_json).unwrap();
    assert_eq!(cands.len(), 5, "topk5 必须是无条件的完整 top5");
    assert_eq!(cands[0].rank, 1);
    assert_eq!(cands[0].subject_id, "sub_mock_1");
    let exp_c0 = api::gallery_index::megvii_calibrate_cosine(0.85);
    assert!((cands[0].similarity - exp_c0).abs() < 1e-4);
    assert_eq!(cands[4].rank, 5);
    assert_eq!(cands[4].subject_id, "sub_mock_5");
    let exp_c4 = api::gallery_index::megvii_calibrate_cosine(0.25);
    assert!((cands[4].similarity - exp_c4).abs() < 1e-4);

    // 7. 验证 WebSocket 成功广播 TOPIC_RECOGNITION_MATCHED 事件且包含完整的 5 个候选人
    let mut found_ws_match = false;
    while let Ok(msg) = ws_rx.try_recv() {
        if msg.topic == types::TOPIC_RECOGNITION_MATCHED {
            found_ws_match = true;
            assert_eq!(msg.payload["status"], "confirmed");
            let ws_cands = msg.payload["candidates"]
                .as_array()
                .expect("ws candidates array");
            assert_eq!(ws_cands.len(), 5, "WS payload 候选人列表也必须为完整 5 人");
            break;
        }
    }
    assert!(
        found_ws_match,
        "达标记录必须通过 WebSocket 广播 TOPIC_RECOGNITION_MATCHED 事件"
    );
}

#[tokio::test]
async fn test_face_recognition_tight_margin_persists_pending_review() {
    let (_app, state, _token) = setup_test_app().await;

    // 1. 底库注册 2 名相似人员：sub_mock_1 相似度 0.85，sub_mock_2 相似度 0.83 (差值 0.02 < MIN_CONFIRM_MARGIN 0.05)
    let mut vec1 = [0.0f32; 512];
    vec1[0] = 0.85;
    vec1[1] = (1.0 - 0.85 * 0.85f32).sqrt();

    let mut vec2 = [0.0f32; 512];
    vec2[0] = 0.83;
    vec2[1] = (1.0 - 0.83 * 0.83f32).sqrt();

    state
        .gallery_index
        .upsert_faces(vec![
            api::RegisteredFace::from_512(
                "sub_mock_1".to_string(),
                "Mock Person 1".to_string(),
                "face_mock_1".to_string(),
                "galleries/sub_mock_1/photo.jpg".to_string(),
                vec1,
            ),
            api::RegisteredFace::from_512(
                "sub_mock_2".to_string(),
                "Mock Person 2".to_string(),
                "face_mock_2".to_string(),
                "galleries/sub_mock_2/photo.jpg".to_string(),
                vec2,
            ),
        ])
        .await;

    // 2. 构造查询向量
    let mut query_vec = [0.0f32; 512];
    query_vec[0] = 1.0;
    let query_embedding = std::sync::Arc::from(api::embedding_to_le_bytes(&query_vec));

    let capture_svc = Arc::new(api::CaptureDispatchService::from_state(&state));
    let _capture_worker = capture_svc.clone().start_worker();

    let mut ws_rx = state.event_broadcaster.subscribe();

    // 3. 发布携带该 embedding 的人脸抓拍事件
    let capture_id = uuid::Uuid::now_v7().to_string();
    let mock_capture = PipelineCaptureEvent {
        capture_id: capture_id.clone(),
        camera_id: "CAM-REC-MARGIN".to_string(),
        algorithm_id: "face_recognition".to_string(),
        tracked_object: TrackedObject {
            track_id: 888,
            class_id: 0,
            label: "face".to_string(),
            confidence: 0.98,
            quality_score: Some(0.50),
            embedding: Some(query_embedding),
            bbox: BoundingBox::new(0.2, 0.2, 0.4, 0.4),
            face: None,
            trajectory: vec![(0.3, 0.3)],
        },
        snapshot: Some(SnapshotResult {
            image_id: "snap_full_888".to_string(),
            image_rel_path: "2026/03/04/CAM-REC-MARGIN/full_888.jpg".to_string(),
            crop_image_id: "snap_crop_888".to_string(),
            crop_image_rel_path: "2026/03/04/CAM-REC-MARGIN/crop_888.jpg".to_string(),
            body_crop_image_id: String::new(),
            body_crop_image_rel_path: String::new(),
            file_size_bytes: 10240,
            width: 1920,
            height: 1080,
            frame_pts_ms: 1741100088000,
            image_source: EvidenceImageSource::PeakCandidate,
            image_stream: EvidenceImageStream::Sub,
        }),
        timestamp: 1741100088000,
    };

    state
        .pipeline
        .publish_analysis_event(PipelineAnalysisEvent::Capture(Box::new(mock_capture)));

    tokio::time::sleep(Duration::from_millis(250)).await;

    // 4. 验证识别记录已落库且状态因 Margin 混淆防控被降级为 pending_review
    let recognitions = db::RecognitionRepo::list_recent(&state.db, Some("CAM-REC-MARGIN"), 10, 0)
        .await
        .unwrap();
    assert_eq!(recognitions.len(), 1, "混淆匹配记录必须入库供复核");
    let rec = &recognitions[0];
    assert_eq!(
        rec.status, "pending_review",
        "Top-1 与 Top-2 差值不足 0.05 必须降级为 pending_review"
    );
    assert_eq!(rec.subject_id, "sub_mock_1");

    // 5. 验证广播了 status: "pending_review"
    let mut found_ws_match = false;
    while let Ok(msg) = ws_rx.try_recv() {
        if msg.topic == types::TOPIC_RECOGNITION_MATCHED {
            found_ws_match = true;
            assert_eq!(msg.payload["status"], "pending_review");
            break;
        }
    }
    assert!(found_ws_match);
}
