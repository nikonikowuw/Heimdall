#![allow(clippy::unwrap_used)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::json;
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
async fn test_gb28181_config_and_device_api_endpoints() {
    let (app, state, token) = setup_test_app().await;

    // 1. GET /api/v1/system/gb28181/config
    let req = Request::builder()
        .uri("/api/v1/system/gb28181/config")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["code"], 0);
    assert_eq!(json["data"]["config"]["sipId"], "34020000002000000001");
    assert_eq!(json["data"]["health"]["running"], true);

    // 2. PUT /api/v1/system/gb28181/config
    let update_payload = json!({
        "sipId": "34020000002000000099",
        "sipPassword": "newPassword999",
        "sipPort": 5062,
        "rtpPortRangeStart": 31000,
        "rtpPortRangeEnd": 31500,
        "autoCatalogSync": false,
        "heartbeatTimeoutSec": 240
    });
    let req = Request::builder()
        .method("PUT")
        .uri("/api/v1/system/gb28181/config")
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_vec(&update_payload).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["code"], 0);
    assert_eq!(json["data"]["sipId"], "34020000002000000099");
    assert_eq!(json["data"]["sipPassword"], "newPassword999");
    assert_eq!(json["data"]["sipPort"], 5062);

    // 3. 在 DB 预置一个 GB28181 设备和通道
    db::Gb28181DeviceRepo::upsert_device(
        &state.db,
        "34020000001180000001",
        "Hikvision 16-ch NVR",
        "192.168.1.120",
        5060,
        "udp",
        "online",
    )
    .await
    .unwrap();

    let channels = vec![types::Gb28181ChannelDto {
        device_id: "34020000001180000001".to_string(),
        channel_id: "34020000001310000001".to_string(),
        name: "Gate Camera".to_string(),
        manufacturer: "Hikvision".to_string(),
        model: "IPC-200".to_string(),
        status: "ON".to_string(),
        parent_id: "34020000001180000001".to_string(),
        sub_stream_supported: true,
        last_seen_ms: 1000,
        is_imported: false,
        camera_id: None,
    }];
    db::Gb28181DeviceRepo::batch_upsert_channels(&state.db, "34020000001180000001", &channels)
        .await
        .unwrap();

    // 4. GET /api/v1/system/gb28181/devices
    let req = Request::builder()
        .uri("/api/v1/system/gb28181/devices")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["code"], 0);
    let dev_list = json["data"].as_array().unwrap();
    assert_eq!(dev_list.len(), 1);
    assert_eq!(dev_list[0]["deviceId"], "34020000001180000001");
    assert_eq!(dev_list[0]["channels"].as_array().unwrap().len(), 1);
    assert_eq!(
        dev_list[0]["channels"][0]["isImported"], false,
        "未导入前 isImported 必须为 false"
    );

    // 5. POST /api/v1/system/gb28181/channels/import
    let import_payload = json!({
        "channels": [
            {
                "deviceId": "34020000001180000001",
                "channelId": "34020000001310000001",
                "name": "Gate Camera Imported",
                "streamMode": "auto"
            }
        ]
    });
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/system/gb28181/channels/import")
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_vec(&import_payload).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["code"], 0);
    assert_eq!(json["data"]["importedCount"], 1);

    // 6. 验证通道已被纳管至 cameras 表中
    let imported_cam = db::CameraRepo::find_by_camera_id(
        &state.db,
        "gb_34020000001180000001_34020000001310000001",
    )
    .await
    .unwrap()
    .expect("imported camera must exist in cameras table");
    assert_eq!(imported_cam.protocol, "gb28181");
    assert_eq!(
        imported_cam.rtsp_url,
        "gb28181://34020000001180000001/34020000001310000001"
    );
    assert_eq!(
        imported_cam.gb28181_device_id,
        Some("34020000001180000001".to_string())
    );
    assert_eq!(
        imported_cam.gb28181_channel_id,
        Some("34020000001310000001".to_string())
    );

    // 7. 再次 GET /api/v1/system/gb28181/devices，断言 isImported 变为 true
    let req = Request::builder()
        .uri("/api/v1/system/gb28181/devices")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["data"][0]["channels"][0]["isImported"], true);

    // 8. POST /api/v1/system/gb28181/devices/{deviceId}/sync
    // 不存在的设备返回 404
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/system/gb28181/devices/nonexistent_device/sync")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // 会话在线设备同步返回 200 OK
    state
        .gb28181_sip_server
        .register_test_session(media::gb28181::RegisteredDeviceSession {
            device_id: "34020000001180000001".to_string(),
            name: "Hikvision 16-ch NVR".to_string(),
            remote_addr: "127.0.0.1:5060".parse().unwrap(),
            transport: "udp".to_string(),
            last_keepalive_mono_ms: 1000,
            cseq: 1,
        });
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/system/gb28181/devices/34020000001180000001/sync")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}
