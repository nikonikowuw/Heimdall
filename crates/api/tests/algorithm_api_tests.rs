#![allow(clippy::unwrap_used)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use db::{AlgorithmRepo, UpsertAlgorithmParams, UpsertVersionParams};
use sea_orm::Set;
use tower::ServiceExt;

async fn setup_test_app() -> (axum::Router, api::AppState, String) {
    let db = db::init_test_db().await.unwrap();
    let pipeline = std::sync::Arc::new(pipeline::PipelineManager::new());
    let state = api::AppState::new(db, pipeline);
    state.sync_auth_state().await;

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
async fn test_algorithms_and_instances_api_endpoints() {
    let (app, state, token) = setup_test_app().await;

    // 1. 预先在 DB 插入一个算法和版本
    AlgorithmRepo::upsert_algorithm(
        &state.db,
        UpsertAlgorithmParams {
            algorithm_id: "test_yolo".to_string(),
            name: "Test YOLO".to_string(),
            algorithm_type: "object_detection".to_string(),
            alarm_type_id: "object_detect".to_string(),
            active_version: "1.0.0".to_string(),
            description: "Test description".to_string(),
            is_builtin: true,
        },
    )
    .await
    .unwrap();

    AlgorithmRepo::upsert_version(
        &state.db,
        UpsertVersionParams {
            algorithm_id: "test_yolo".to_string(),
            version: "1.0.0".to_string(),
            platform_id: "macos-arm64".to_string(),
            min_adapter_version: "1.0.0".to_string(),
            package_root: "var/packages/test_yolo/1.0.0".to_string(),
            fps_tiers: r#"[{"fps":15,"units":100}]"#.to_string(),
            config_schema: r#"{"properties":{"threshold":{"type":"number"}}}"#.to_string(),
            manifest_raw: "{}".to_string(),
            package_size_bytes: 1024,
            is_active: true,
            is_builtin: true,
        },
    )
    .await
    .unwrap();

    // 2. GET /api/v1/algorithms
    let req = Request::builder()
        .uri("/api/v1/algorithms")
        .method("GET")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 3. GET /api/v1/algorithms/stats
    let req = Request::builder()
        .uri("/api/v1/algorithms/stats")
        .method("GET")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 4. GET /api/v1/algorithms/test_yolo
    let req = Request::builder()
        .uri("/api/v1/algorithms/test_yolo")
        .method("GET")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 5. DELETE /api/v1/algorithms/test_yolo/versions/1.0.0 -> 因为是内置包，应拦截返回 403
    let req = Request::builder()
        .uri("/api/v1/algorithms/test_yolo/versions/1.0.0")
        .method("DELETE")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // 6. 算法实例 CRUD 测试
    // 创建前置摄像头
    let camera_model = db::entity::camera::ActiveModel {
        id: sea_orm::ActiveValue::NotSet,
        camera_id: Set("CAM-INST-01".to_string()),
        name: Set("测试摄像头".to_string()),
        protocol: Set("rtsp".to_string()),
        rtsp_url: Set("rtsp://127.0.0.1:8554/live".to_string()),
        sub_rtsp_url: Set("".to_string()),
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
    };
    db::CameraRepo::insert(&state.db, camera_model)
        .await
        .unwrap();

    // POST /api/v1/tasks/instances
    let create_body = serde_json::json!({
        "cameraId": "CAM-INST-01",
        "algorithmId": "test_yolo",
        "analysisFps": 15,
        "params": { "threshold": 0.6 },
        "rules": [],
        "motionGate": { "enabled": true },
        "enabled": true
    });
    let req = Request::builder()
        .uri("/api/v1/tasks/instances")
        .method("POST")
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .body(Body::from(create_body.to_string()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let instance_id = val["data"]["instanceId"].as_str().unwrap().to_string();

    // GET /api/v1/tasks/instances?cameraId=CAM-INST-01
    let req = Request::builder()
        .uri("/api/v1/tasks/instances?cameraId=CAM-INST-01")
        .method("GET")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // PUT /api/v1/tasks/instances/{instance_id}/enabled
    let req = Request::builder()
        .uri(format!("/api/v1/tasks/instances/{instance_id}/enabled"))
        .method("PUT")
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .body(Body::from(r#"{"enabled":false}"#))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // DELETE /api/v1/tasks/instances/{instance_id}
    let req = Request::builder()
        .uri(format!("/api/v1/tasks/instances/{instance_id}"))
        .method("DELETE")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}
