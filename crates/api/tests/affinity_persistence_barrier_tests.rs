#![allow(clippy::unwrap_used)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use db::{AlgorithmRepo, UpsertAlgorithmParams};
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
    let token =
        api::crypto::generate_jwt(&claims, &state.get_jwt_secret().expect("测试环境密钥可用"))
            .unwrap();

    let app = api::create_app(state.clone());
    (app, state, token)
}

async fn insert_camera(state: &api::AppState, camera_id: &str) {
    db::CameraRepo::insert(
        &state.db,
        db::entity::camera::ActiveModel {
            id: sea_orm::ActiveValue::NotSet,
            camera_id: Set(camera_id.to_string()),
            name: Set("测试摄像头".to_string()),
            protocol: Set("rtsp".to_string()),
            rtsp_url: Set("rtsp://127.0.0.1:8554/live".to_string()),
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
            recording_config: Set(String::new()),
            created_at: Set(chrono::Utc::now()),
            updated_at: Set(chrono::Utc::now()),
        },
    )
    .await
    .unwrap();
}

async fn insert_algorithm(state: &api::AppState, algorithm_id: &str) {
    AlgorithmRepo::upsert_algorithm(
        &state.db,
        UpsertAlgorithmParams {
            algorithm_id: algorithm_id.to_string(),
            name: "测试算法".to_string(),
            algorithm_type: "detection".to_string(),
            alarm_type_id: "intrusion".to_string(),
            active_version: "1.0.0".to_string(),
            description: "test".to_string(),
            is_builtin: true,
        },
    )
    .await
    .unwrap();
}

async fn put_task(
    app: &axum::Router,
    token: &str,
    camera_id: &str,
    body: &serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .uri(format!("/api/v1/tasks/{camera_id}"))
        .method("PUT")
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_vec(body).unwrap()))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

async fn get_task(
    app: &axum::Router,
    token: &str,
    camera_id: &str,
) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .uri(format!("/api/v1/tasks/{camera_id}"))
        .method("GET")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

fn task_body(camera_id: &str, name: &str, instances: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "cameraId": camera_id,
        "name": name,
        "desiredEnabled": false,
        "rules": [],
        "algorithmInstances": instances,
    })
}

/// T15 / T16: Affinity validation via HTTP API
#[tokio::test]
async fn test_affinity_http_validation_rules() {
    let (app, state, token) = setup_test_app().await;
    let camera_id = "cam_aff_val";
    insert_camera(&state, camera_id).await;
    insert_algorithm(&state, "algo_det").await;

    // 1. 无效的 manual 亲和：空 deviceId
    let body_empty_device = task_body(
        camera_id,
        "非法亲和任务",
        serde_json::json!([
            {
                "algorithmId": "algo_det",
                "analysisFps": 15,
                "affinity": {
                    "mode": "manual",
                    "deviceId": "  ",
                    "coreIndex": 0
                }
            }
        ]),
    );
    let (status, resp) = put_task(&app, &token, camera_id, &body_empty_device).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        resp["message"]
            .as_str()
            .unwrap_or_default()
            .contains("deviceId"),
        "必须明确拒绝空白 deviceId"
    );

    // 2. 无效的 auto 亲和：未知策略 (除 spread / pack 以外)
    let body_unknown_policy = task_body(
        camera_id,
        "未知策略任务",
        serde_json::json!([
            {
                "algorithmId": "algo_det",
                "analysisFps": 15,
                "affinity": {
                    "mode": "auto",
                    "policy": "invalid_policy_name"
                }
            }
        ]),
    );
    let (status, resp) = put_task(&app, &token, camera_id, &body_unknown_policy).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        resp["message"]
            .as_str()
            .unwrap_or_default()
            .contains("policy"),
        "必须明确拒绝未知的 auto policy"
    );

    // 3. 有效的 auto 亲和 (pack)
    let body_valid = task_body(
        camera_id,
        "合法亲和任务",
        serde_json::json!([
            {
                "algorithmId": "algo_det",
                "analysisFps": 15,
                "affinity": {
                    "mode": "auto",
                    "policy": "pack"
                }
            }
        ]),
    );
    let (status, resp) = put_task(&app, &token, camera_id, &body_valid).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "合法 auto pack 亲和应提交成功: {resp:?}"
    );
}

/// T30 & T40: Revision barrier, camelCase DTO serialization and round-trip
#[tokio::test]
async fn test_affinity_revision_barrier_and_dto_contract() {
    let (app, state, token) = setup_test_app().await;
    let camera_id = "cam_barrier_dto";
    insert_camera(&state, camera_id).await;
    insert_algorithm(&state, "algo_det").await;

    // 1. 首次创建携带 manual 亲和
    let create_body = task_body(
        camera_id,
        "亲和屏障测试",
        serde_json::json!([
            {
                "algorithmId": "algo_det",
                "analysisFps": 15,
                "affinity": {
                    "mode": "manual",
                    "deviceId": "npu-0",
                    "coreIndex": 2
                }
            }
        ]),
    );

    let (status, resp) = put_task(&app, &token, camera_id, &create_body).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(resp["code"], 0);

    let instance = &resp["data"]["algorithmInstances"][0];
    assert_eq!(instance["algorithmId"], "algo_det");
    assert_eq!(instance["desiredRevision"], 0);
    assert_eq!(instance["appliedRevision"], 0);
    // 验证 DTO 返回了驼峰命名
    assert_eq!(instance["affinity"]["mode"], "manual");
    assert_eq!(instance["affinity"]["deviceId"], "npu-0");
    assert_eq!(instance["affinity"]["coreIndex"], 2);

    let config_rev = resp["data"]["configRevision"].as_i64().unwrap();
    assert_eq!(config_rev, 1);

    // 2. GET 查询验证持久化与序列化
    let (status, get_resp) = get_task(&app, &token, camera_id).await;
    assert_eq!(status, StatusCode::OK);
    let get_inst = &get_resp["data"]["algorithmInstances"][0];
    assert_eq!(get_inst["affinity"]["mode"], "manual");
    assert_eq!(get_inst["affinity"]["deviceId"], "npu-0");
    assert_eq!(get_inst["affinity"]["coreIndex"], 2);

    // 3. 过期 configRevision 提交导致 409 Conflict
    let mut stale_body = task_body(
        camera_id,
        "过期提交",
        serde_json::json!([
            {
                "algorithmId": "algo_det",
                "analysisFps": 20
            }
        ]),
    );
    stale_body["configRevision"] = serde_json::json!(config_rev - 1); // 0 不是当前版本 1
    let (status, stale_resp) = put_task(&app, &token, camera_id, &stale_body).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(stale_resp["code"], 40903);

    // 4. 正确 configRevision 提交：修改为 auto 亲和，desiredRevision 递增
    let mut update_body = task_body(
        camera_id,
        "新版本提交",
        serde_json::json!([
            {
                "instanceId": get_inst["instanceId"],
                "algorithmId": "algo_det",
                "analysisFps": 15,
                "affinity": {
                    "mode": "auto",
                    "policy": "spread"
                }
            }
        ]),
    );
    update_body["configRevision"] = serde_json::json!(config_rev);
    let (status, update_resp) = put_task(&app, &token, camera_id, &update_body).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(update_resp["data"]["configRevision"], 2);
    let updated_inst = &update_resp["data"]["algorithmInstances"][0];
    assert_eq!(updated_inst["desiredRevision"], 1);
    assert_eq!(updated_inst["affinity"]["mode"], "auto");
    assert_eq!(updated_inst["affinity"]["policy"], "spread");
}
