#![allow(clippy::unwrap_used)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use db::entity::recognition::ActiveModel as RecognitionActiveModel;
use db::RecognitionRepo;
use sea_orm::Set;
use tower::ServiceExt;

async fn setup_test_app() -> (axum::Router, api::AppState, String) {
    let db = db::init_test_db().await.unwrap();
    let pipeline = std::sync::Arc::new(pipeline::PipelineManager::new());
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

#[tokio::test]
async fn test_review_recognition_lifecycle_and_broadcast() {
    let (app, state, token) = setup_test_app().await;

    // 1. 插入一条待复核识别对账记录
    let rec = RecognitionActiveModel {
        id: sea_orm::NotSet,
        recognition_id: Set("rec_review_1".to_string()),
        camera_id: Set("CAM-01".to_string()),
        gallery_id: Set("default".to_string()),
        subject_id: Set("sub_alice".to_string()),
        subject_name: Set("Alice".to_string()),
        similarity: Set(0.68),
        field_crop_path: Set("captures/crop_1.jpg".to_string()),
        registered_photo_path: Set("recognitions/rec_review_1_gallery.jpg".to_string()),
        status: Set("pending_review".to_string()),
        candidates_json: Set(Some(
            r#"[{"rank":1,"subjectId":"sub_alice","subjectName":"Alice","faceId":"f1","photoRelPath":"galleries/sub_alice/f1.jpg","similarity":0.68}]"#.to_string(),
        )),
        reviewer_id: Set(None),
        reviewed_at: Set(None),
        recognized_at: Set(chrono::Utc::now()),
        created_at: Set(chrono::Utc::now()),
    };
    RecognitionRepo::insert(&state.db, rec).await.unwrap();

    // 订阅 WebSocket 广播
    let mut rx = state.event_broadcaster.subscribe();

    // 2. 调用审核接口确认放行 (POST /api/v1/evidence/recognitions/rec_review_1/review)
    let body = serde_json::json!({
        "status": "confirmed",
        "subjectId": "sub_alice",
        "subjectName": "Alice Cooper",
        "similarity": 0.85
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/evidence/recognitions/rec_review_1/review")
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let res_bytes = axum::body::to_bytes(res.into_body(), 1024 * 16)
        .await
        .unwrap();
    let json_val: serde_json::Value = serde_json::from_slice(&res_bytes).unwrap();
    assert_eq!(json_val["code"], 0);
    assert_eq!(json_val["data"]["status"], "confirmed");
    assert_eq!(json_val["data"]["subjectName"], "Alice Cooper");
    assert_eq!(json_val["data"]["reviewerId"], "admin");
    assert!(json_val["data"]["reviewedAt"].as_i64().is_some());

    // 3. 验证广播事件收到 TOPIC_RECOGNITION_STATUS_CHANGED
    let broadcast_event = rx.recv().await.unwrap();
    assert_eq!(
        broadcast_event.topic,
        types::TOPIC_RECOGNITION_STATUS_CHANGED
    );
    assert_eq!(broadcast_event.payload["recognitionId"], "rec_review_1");
    assert_eq!(broadcast_event.payload["status"], "confirmed");
    assert_eq!(broadcast_event.payload["reviewerId"], "admin");

    // 4. 测试驳回状态 (rejected)
    let reject_body = serde_json::json!({
        "status": "rejected"
    });
    let req_reject = Request::builder()
        .method("POST")
        .uri("/api/v1/evidence/recognitions/rec_review_1/review")
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_vec(&reject_body).unwrap()))
        .unwrap();
    let res_reject = app.clone().oneshot(req_reject).await.unwrap();
    assert_eq!(res_reject.status(), StatusCode::OK);
    let reject_bytes = axum::body::to_bytes(res_reject.into_body(), 1024 * 16)
        .await
        .unwrap();
    let reject_json: serde_json::Value = serde_json::from_slice(&reject_bytes).unwrap();
    assert_eq!(reject_json["data"]["status"], "rejected");
}

#[tokio::test]
async fn test_review_recognition_validation_errors() {
    let (app, _, token) = setup_test_app().await;

    // 1. 无效状态参数 -> 400 Bad Request
    let bad_body = serde_json::json!({ "status": "unknown_status" });
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/evidence/recognitions/rec_none/review")
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_vec(&bad_body).unwrap()))
        .unwrap();
    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // 2. 试图将审核结果设为 pending_review -> 400 Bad Request
    let pending_body = serde_json::json!({ "status": "pending_review" });
    let req_pending = Request::builder()
        .method("POST")
        .uri("/api/v1/evidence/recognitions/rec_none/review")
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_vec(&pending_body).unwrap()))
        .unwrap();
    let res_pending = app.clone().oneshot(req_pending).await.unwrap();
    assert_eq!(res_pending.status(), StatusCode::BAD_REQUEST);

    // 3. 不存在的记录 -> 404 Not Found
    let valid_body = serde_json::json!({ "status": "confirmed" });
    let req_404 = Request::builder()
        .method("POST")
        .uri("/api/v1/evidence/recognitions/nonexistent_rec/review")
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_vec(&valid_body).unwrap()))
        .unwrap();
    let res_404 = app.clone().oneshot(req_404).await.unwrap();
    assert_eq!(res_404.status(), StatusCode::NOT_FOUND);

    // 4. 未授权调用 -> 401 Unauthorized
    let req_noauth = Request::builder()
        .method("POST")
        .uri("/api/v1/evidence/recognitions/rec_none/review")
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_vec(&valid_body).unwrap()))
        .unwrap();
    let res_noauth = app.clone().oneshot(req_noauth).await.unwrap();
    assert_eq!(res_noauth.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_review_recognition_self_copy_safety_does_not_truncate() {
    let (app, state, token) = setup_test_app().await;

    // 创建测试证据文件目录及样本照片
    let base_dir = state.pipeline.snapshot_engine().base_evidence_dir();
    let rec_dir = base_dir.join("recognitions");
    tokio::fs::create_dir_all(&rec_dir).await.unwrap();
    let photo_file = rec_dir.join("rec_self_copy_gallery.jpg");
    let test_bytes = b"EXACT_IMAGE_DATA_SHOULD_NOT_BE_TRUNCATED_TO_ZERO";
    tokio::fs::write(&photo_file, test_bytes).await.unwrap();

    let rec = RecognitionActiveModel {
        id: sea_orm::NotSet,
        recognition_id: Set("rec_self_copy".to_string()),
        camera_id: Set("CAM-01".to_string()),
        gallery_id: Set("default".to_string()),
        subject_id: Set("sub_bob".to_string()),
        subject_name: Set("Bob".to_string()),
        similarity: Set(0.70),
        field_crop_path: Set("captures/crop_2.jpg".to_string()),
        registered_photo_path: Set("recognitions/rec_self_copy_gallery.jpg".to_string()),
        status: Set("pending_review".to_string()),
        candidates_json: Set(None),
        reviewer_id: Set(None),
        reviewed_at: Set(None),
        recognized_at: Set(chrono::Utc::now()),
        created_at: Set(chrono::Utc::now()),
    };
    RecognitionRepo::insert(&state.db, rec).await.unwrap();

    // 发起审核请求，携带与自身完全同名的 photo_rel_path
    let body = serde_json::json!({
        "status": "confirmed",
        "photoRelPath": "recognitions/rec_self_copy_gallery.jpg"
    });

    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/evidence/recognitions/rec_self_copy/review")
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 关键断言：校验文件未被截断为 0 字节，数据完好无损
    let read_back = tokio::fs::read(&photo_file).await.unwrap();
    assert_eq!(read_back.as_slice(), test_bytes);

    // 清理测试临时文件
    let _ = tokio::fs::remove_dir_all(&base_dir).await;
}
