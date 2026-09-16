//! 布防开关（任务级状态动词）的 API 契约集成测试
//!
//! `PUT /api/v1/tasks/{cameraId}/enabled` 是「整份配置下发」之外的第二个写入口，
//! 但它只表达一个期望：该通道的 AI 分析应该运行还是停止。
//! 覆盖：
//! - 状态动词不携带也不改写名称、防区、门控与算法参数；
//! - 与整体下发共用配置版本号，基于旧快照的整体下发会被 409 拒绝；
//! - 撤防/布防都把真实运行状态写回数据库，而不是只改一个布尔值；
//! - 任务或摄像头不存在时明确失败，不隐式创建任务。
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
    let token = api::crypto::generate_jwt(&claims, &state.get_jwt_secret()).unwrap();

    let app = api::create_app(state.clone());
    (app, state, token)
}

/// 插入一台摄像头；`rtsp_url` 为空用于稳定复现「媒体源不可用」的布防守卫
async fn insert_camera(state: &api::AppState, camera_id: &str, rtsp_url: &str) {
    db::CameraRepo::insert(
        &state.db,
        db::entity::camera::ActiveModel {
            id: sea_orm::ActiveValue::NotSet,
            camera_id: Set(camera_id.to_string()),
            name: Set("测试摄像头".to_string()),
            protocol: Set("rtsp".to_string()),
            rtsp_url: Set(rtsp_url.to_string()),
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

async fn send(
    app: &axum::Router,
    token: &str,
    method: &str,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .uri(uri)
        .method(method)
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .body(match body {
            Some(v) => Body::from(serde_json::to_vec(&v).unwrap()),
            None => Body::empty(),
        })
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

fn task_body(camera_id: &str, desired_enabled: bool) -> serde_json::Value {
    serde_json::json!({
        "cameraId": camera_id,
        "name": "状态动词任务",
        "desiredEnabled": desired_enabled,
        "rules": [{ "role": "line", "points": [{ "x": 0.2, "y": 0.5 }, { "x": 0.8, "y": 0.5 }] }],
        "motionGate": { "enabled": true, "threshold": 30 },
        "algorithmInstances": [
            { "algorithmId": "general_detection", "analysisFps": 12, "algoParams": { "confidence": 0.42 }, "enabled": true }
        ]
    })
}

/// 布防开关只翻转总闸：配置本体与分闸意图逐项保持，版本号照常推进。
#[tokio::test]
async fn test_enabled_verb_toggles_only_intent_and_reports_truthful_status() {
    let (app, state, token) = setup_test_app().await;
    // 空 RTSP 地址：布防必然走「媒体源不可用」的守卫分支，无需解码器即可稳定验证状态回写
    insert_camera(&state, "CAM-VERB-01", "").await;
    insert_algorithm(&state, "general_detection").await;

    // 1. 先整体下发一份撤防配置（含具体防区、门控与实例参数）
    let (status, json) = send(
        &app,
        &token,
        "PUT",
        "/api/v1/tasks/CAM-VERB-01",
        Some(task_body("CAM-VERB-01", false)),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let revision_after_config = json["data"]["configRevision"].as_i64().unwrap();
    assert_eq!(json["data"]["actualStatus"], 0);

    // 2. 布防：只发一个布尔值。媒体源不可用时必须如实报错，并把实例代际收敛掉，
    //    不能在库里写一个「已布防」而运行时什么都没有。
    let (status, json) = send(
        &app,
        &token,
        "PUT",
        "/api/v1/tasks/CAM-VERB-01/enabled",
        Some(serde_json::json!({ "enabled": true })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data"]["desiredEnabled"], true);
    assert_eq!(
        json["data"]["configRevision"].as_i64().unwrap(),
        revision_after_config + 1,
        "布防意图同样属于配置，必须推进版本号"
    );
    assert_eq!(
        json["data"]["actualStatus"], 5,
        "媒体源不可用时必须是 Error"
    );
    assert!(
        json["data"]["statusMessage"]
            .as_str()
            .is_some_and(|m| m.contains("RTSP 地址为空")),
        "必须给出具体原因：{json}"
    );
    // 配置本体逐项未被改写
    assert_eq!(json["data"]["name"], "状态动词任务");
    assert_eq!(json["data"]["rules"].as_array().unwrap().len(), 1);
    assert_eq!(json["data"]["motionGate"]["threshold"], 30);
    let instance = &json["data"]["algorithmInstances"][0];
    assert_eq!(instance["analysisFps"], 12);
    assert_eq!(instance["algoParams"]["confidence"], 0.42);
    assert_eq!(
        instance["applyState"], "applied",
        "不可运行的期望配置不存在可重试的未生效状态，代际必须收敛"
    );

    // 3. 撤防：状态回到停机，实例运行状态写回停机，分闸意图保留
    let (status, json) = send(
        &app,
        &token,
        "PUT",
        "/api/v1/tasks/CAM-VERB-01/enabled",
        Some(serde_json::json!({ "enabled": false })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data"]["desiredEnabled"], false);
    assert_eq!(json["data"]["actualStatus"], 0);
    assert_eq!(
        json["data"]["configRevision"].as_i64().unwrap(),
        revision_after_config + 2
    );
    let instances = db::AlgorithmInstanceRepo::list_by_camera_id(&state.db, "CAM-VERB-01")
        .await
        .unwrap();
    assert_eq!(
        instances[0].actual_status,
        types::TaskStatus::Stopped.as_i32()
    );
    assert!(
        instances[0].enabled,
        "撤防不得连坐改写分闸意图，否则再次布防时无实例可运行"
    );
    assert_eq!(instances[0].params_json, r#"{"confidence":0.42}"#);

    // 4. 陈旧快照的整体下发必须被拒绝：开关已经改过配置版本
    let mut stale = task_body("CAM-VERB-01", true);
    stale["configRevision"] = serde_json::json!(revision_after_config);
    let (status, json) = send(
        &app,
        &token,
        "PUT",
        "/api/v1/tasks/CAM-VERB-01",
        Some(stale),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(json["code"], 40903);
    let task_after_conflict = db::TaskRepo::find_by_camera_id(&state.db, "CAM-VERB-01")
        .await
        .unwrap()
        .unwrap();
    assert!(
        !task_after_conflict.desired_enabled,
        "被拒绝的整份覆盖写不得把布防状态改回去"
    );
}

/// 状态动词不隐式创建任务，也不为缺失的摄像头制造「布防成功」的假象。
#[tokio::test]
async fn test_enabled_verb_fails_cleanly_without_task_or_camera() {
    let (app, state, token) = setup_test_app().await;
    insert_camera(&state, "CAM-VERB-02", "rtsp://127.0.0.1:8554/live").await;

    // 摄像头存在但没有任务：状态动词不负责创建（创建仍由整体下发承担）
    let (status, json) = send(
        &app,
        &token,
        "PUT",
        "/api/v1/tasks/CAM-VERB-02/enabled",
        Some(serde_json::json!({ "enabled": true })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json["code"], 40401);
    assert!(
        db::TaskRepo::find_by_camera_id(&state.db, "CAM-VERB-02")
            .await
            .unwrap()
            .is_none(),
        "状态动词不得凭空创建任务"
    );

    // 任务与摄像头都不存在：布防与撤防都必须明确失败
    for enabled in [true, false] {
        let (status, _) = send(
            &app,
            &token,
            "PUT",
            "/api/v1/tasks/CAM-VERB-404/enabled",
            Some(serde_json::json!({ "enabled": enabled })),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
