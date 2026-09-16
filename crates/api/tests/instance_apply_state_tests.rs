//! 任务级配置提交与实例收敛状态的 API 契约集成测试
//!
//! 覆盖设计文档第 7、8 节对控制面的承诺：
//! - 写路径只有任务级保存：新增、改参、启停、删除都是「整份任务配置一次下发」；
//! - 响应逐实例返回 `desiredRevision` / `appliedRevision` / `applyState` / `statusMessage`；
//! - 任务未运行时期望配置仍被接受，并明确标注为「启动时生效」而不是假装已生效。
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

/// 任务级保存：唯一写路径
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

fn instance_of<'a>(json: &'a serde_json::Value, algorithm_id: &str) -> &'a serde_json::Value {
    json["data"]["algorithmInstances"]
        .as_array()
        .expect("algorithmInstances 应为数组")
        .iter()
        .find(|inst| inst["algorithmId"] == algorithm_id)
        .expect("目标算法实例应在响应中")
}

fn task_body(camera_id: &str, instances: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "cameraId": camera_id,
        "name": "两阶段提交任务",
        "desiredEnabled": false,
        "rules": [],
        "algorithmInstances": instances,
    })
}

/// 未布防任务：期望配置被持久化并标注为「下次启动生效」，不能假装已在运行时生效。
#[tokio::test]
async fn test_task_save_apply_state_contract_without_running_task() {
    let (app, state, token) = setup_test_app().await;
    insert_camera(&state, "CAM-APPLY-01").await;
    insert_algorithm(&state, "general_detection").await;

    // 1. 新增启用态实例：未布防任务不启动运行时，但必须返回收敛状态字段
    let mut body = task_body(
        "CAM-APPLY-01",
        serde_json::json!([
            {
                "algorithmId": "general_detection",
                "analysisFps": 10,
                "algoParams": { "confidence": 0.5 },
                "enabled": true
            }
        ]),
    );
    let (status, json) = put_task(&app, &token, "CAM-APPLY-01", &body).await;
    assert_eq!(status, StatusCode::OK);
    let instance = instance_of(&json, "general_detection");
    assert!(
        instance["instanceId"]
            .as_str()
            .is_some_and(|id| !id.is_empty()),
        "任务级保存必须返回运行时寻址使用的 instanceId"
    );
    assert_eq!(instance["applyState"], "applied");
    assert_eq!(instance["desiredRevision"], 0);
    assert_eq!(instance["appliedRevision"], 0);
    assert_eq!(
        instance["enabled"], false,
        "未布防任务下实例保持停用，期望配置仍会随任务启动生效"
    );

    let instance_id = db::AlgorithmInstanceRepo::list_by_camera_id(&state.db, "CAM-APPLY-01")
        .await
        .unwrap()
        .first()
        .map(|inst| inst.instance_id.clone())
        .expect("实例应已持久化");

    // 2. 应用参数：期望代际递增，且任务未运行时立即收敛为已生效（下次启动生效）
    body["algorithmInstances"][0]["analysisFps"] = serde_json::json!(12);
    body["algorithmInstances"][0]["algoParams"] = serde_json::json!({ "confidence": 0.8 });
    let (status, json) = put_task(&app, &token, "CAM-APPLY-01", &body).await;
    assert_eq!(status, StatusCode::OK);
    let instance = instance_of(&json, "general_detection");
    assert_eq!(instance["analysisFps"], 12);
    assert_eq!(instance["algoParams"]["confidence"], 0.8);
    assert_eq!(
        instance["desiredRevision"], 1,
        "每次期望配置提交必须递增期望代际"
    );
    assert_eq!(
        instance["appliedRevision"], 1,
        "无运行时时期望配置就是启动时使用的那一份，代际必须收敛"
    );
    assert_eq!(instance["applyState"], "applied");
    assert_eq!(instance["statusMessage"], "");

    // 3. 任务查询接口必须返回同一份收敛状态
    let (status, json) = get_task(&app, &token, "CAM-APPLY-01").await;
    assert_eq!(status, StatusCode::OK);
    let instance = instance_of(&json, "general_detection");
    assert_eq!(instance["instanceId"], instance_id);
    assert_eq!(instance["desiredRevision"], 1);
    assert_eq!(instance["appliedRevision"], 1);
    assert_eq!(instance["applyState"], "applied");

    // 4. 禁用实例：条目保留在集合中，只是不再期望运行
    body["algorithmInstances"][0]["enabled"] = serde_json::json!(false);
    let (status, json) = put_task(&app, &token, "CAM-APPLY-01", &body).await;
    assert_eq!(status, StatusCode::OK);
    let instance = instance_of(&json, "general_detection");
    assert_eq!(instance["enabled"], false);
    assert_eq!(instance["applyState"], "applied");
    assert!(instance["desiredRevision"].as_i64().unwrap() >= 1);

    // 5. 删除实例：从集合中移除即触发清理，不得残留记录
    let (status, _) = put_task(
        &app,
        &token,
        "CAM-APPLY-01",
        &task_body("CAM-APPLY-01", serde_json::json!([])),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let remaining = db::AlgorithmInstanceRepo::list_by_camera_id(&state.db, "CAM-APPLY-01")
        .await
        .unwrap();
    assert!(remaining.is_empty(), "删除后不得残留实例记录");
}

/// 任务级保存返回的实例 DTO 必须携带收敛状态字段，供前端区分「已应用」与「未保存」。
#[tokio::test]
async fn test_task_level_save_reports_instance_apply_state() {
    let (app, state, token) = setup_test_app().await;
    insert_camera(&state, "CAM-APPLY-02").await;
    insert_algorithm(&state, "general_detection").await;

    let (status, json) = put_task(
        &app,
        &token,
        "CAM-APPLY-02",
        &task_body(
            "CAM-APPLY-02",
            serde_json::json!([
                {
                    "algorithmId": "general_detection",
                    "analysisFps": 10,
                    "algoParams": { "confidence": 0.4 },
                    "enabled": true
                }
            ]),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let instances = json["data"]["algorithmInstances"].as_array().unwrap();
    assert_eq!(instances.len(), 1);
    let instance = &instances[0];
    assert_eq!(instance["applyState"], "applied");
    assert!(instance["desiredRevision"].is_i64());
    assert!(instance["appliedRevision"].is_i64());
    assert_eq!(instance["statusMessage"], "");

    // 任务列表同样必须带上收敛状态，供任务卡片展示未生效实例
    let req = Request::builder()
        .uri("/api/v1/tasks")
        .method("GET")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let listed = json["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|task| task["cameraId"] == "CAM-APPLY-02")
        .expect("任务应在列表中");
    assert_eq!(listed["algorithmInstances"][0]["applyState"], "applied");
}

/// 整体下发的乐观并发保护：携带快照版本的保存与库中版本不匹配时拒绝写入，
/// 避免两个会话各自基于旧快照互相覆盖实例集合与参数。
#[tokio::test]
async fn test_task_save_rejects_stale_config_revision() {
    let (app, state, token) = setup_test_app().await;
    insert_camera(&state, "CAM-CONFLICT-01").await;
    insert_algorithm(&state, "general_detection").await;

    // 1. 兼容路径：不带 configRevision 的保存仍被接受（旧客户端），响应回传当前版本号
    let body = task_body(
        "CAM-CONFLICT-01",
        serde_json::json!([
            { "algorithmId": "general_detection", "analysisFps": 10, "algoParams": {} }
        ]),
    );
    let (status, json) = put_task(&app, &token, "CAM-CONFLICT-01", &body).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data"]["configRevision"], 1);

    // 2. GET 与列表都必须暴露快照版本号，供客户端回传
    let (status, json) = get_task(&app, &token, "CAM-CONFLICT-01").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data"]["configRevision"], 1);

    let list_req = Request::builder()
        .uri("/api/v1/tasks")
        .method("GET")
        .header("Authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(list_req).await.unwrap();
    let list_json: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(list_json["data"][0]["configRevision"], 1);

    // 3. 会话 A 基于版本 1 保存成功，版本号推进
    let mut session_a = body.clone();
    session_a["configRevision"] = serde_json::json!(1);
    session_a["name"] = serde_json::json!("会话A");
    session_a["algorithmInstances"] = serde_json::json!([
        { "algorithmId": "general_detection", "analysisFps": 15, "algoParams": {} }
    ]);
    let (status, json) = put_task(&app, &token, "CAM-CONFLICT-01", &session_a).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data"]["configRevision"], 2);
    assert_eq!(
        instance_of(&json, "general_detection")["analysisFps"],
        15,
        "会话A的修改必须落库"
    );

    // 4. 会话 B 仍持有版本 1：保存被拒绝，返回 409 + 业务码 40903
    let mut session_b = body.clone();
    session_b["configRevision"] = serde_json::json!(1);
    session_b["name"] = serde_json::json!("会话B");
    session_b["algorithmInstances"] = serde_json::json!([
        { "algorithmId": "general_detection", "analysisFps": 30, "algoParams": {} }
    ]);
    let (status, json) = put_task(&app, &token, "CAM-CONFLICT-01", &session_b).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(json["code"], 40903);
    assert!(json["data"].is_null(), "冲突响应不得返回伪造的配置快照");
    assert!(
        json["message"]
            .as_str()
            .is_some_and(|m| m.contains("其他会话")),
        "冲突提示必须可读：{json}"
    );

    // 5. 被拒绝的请求不得留下任何痕迹（配置与实例参数都保持会话 A 的结果）
    let (status, json) = get_task(&app, &token, "CAM-CONFLICT-01").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data"]["name"], "会话A");
    assert_eq!(json["data"]["configRevision"], 2);
    assert_eq!(instance_of(&json, "general_detection")["analysisFps"], 15);

    // 6. 会话 B 载入最新配置后重试成功，版本号继续推进
    session_b["configRevision"] = serde_json::json!(2);
    let (status, json) = put_task(&app, &token, "CAM-CONFLICT-01", &session_b).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["data"]["configRevision"], 3);
    assert_eq!(
        instance_of(&json, "general_detection")["analysisFps"],
        30,
        "载入最新配置后重试必须生效"
    );

    // 7. 快速创建断言「该通道还没有任务」（configRevision: 0）：通道已有任务时按冲突拒绝，
    //    否则并发创建会把对方刚配置好的实例与防区整个覆盖掉
    let mut quick_create = task_body(
        "CAM-CONFLICT-01",
        serde_json::json!([
            { "algorithmId": "general_detection", "analysisFps": 5, "algoParams": {} }
        ]),
    );
    quick_create["name"] = serde_json::json!("并发创建");
    quick_create["configRevision"] = serde_json::json!(0);
    let (status, json) = put_task(&app, &token, "CAM-CONFLICT-01", &quick_create).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(json["code"], 40903);

    let (_, json) = get_task(&app, &token, "CAM-CONFLICT-01").await;
    assert_eq!(json["data"]["name"], "会话B");
    assert_eq!(json["data"]["configRevision"], 3);
    assert_eq!(instance_of(&json, "general_detection")["analysisFps"], 30);
}
