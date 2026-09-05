use axum::extract::{Path, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use db::TaskRepo;
use types::{DetectionRule, MotionGateConfig};

use crate::error::ApiError;
use crate::middleware::AuthUser;
use crate::response::ApiResponse;
use crate::state::AppState;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskConfigDto {
    pub camera_id: String,
    pub name: String,
    pub desired_enabled: bool,
    pub rules: Vec<DetectionRule>,
    #[serde(default)]
    pub motion_gate: Option<MotionGateConfig>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskSummaryDto {
    pub id: i64,
    pub camera_id: String,
    pub name: String,
    pub desired_enabled: bool,
    pub actual_status: i32,
    pub rules_count: usize,
    pub motion_gate_enabled: bool,
    pub rules: Vec<DetectionRule>,
    pub motion_gate: Option<MotionGateConfig>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<db::entity::task::Model> for TaskSummaryDto {
    fn from(m: db::entity::task::Model) -> Self {
        let rules: Vec<DetectionRule> = serde_json::from_str(&m.rules_json).unwrap_or_default();
        let motion_gate: Option<MotionGateConfig> = serde_json::from_str(&m.motion_gate_json).ok();
        let motion_gate_enabled = motion_gate.as_ref().map(|mg| mg.enabled).unwrap_or(false);
        let rules_count = rules.len();

        Self {
            id: m.id,
            camera_id: m.camera_id,
            name: m.name,
            desired_enabled: m.desired_enabled,
            actual_status: m.actual_status,
            rules_count,
            motion_gate_enabled,
            rules,
            motion_gate,
            created_at: m.created_at.timestamp_millis(),
            updated_at: m.updated_at.timestamp_millis(),
        }
    }
}

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(list_tasks)).route(
        "/{camera_id}",
        get(get_task).put(update_task).delete(delete_task),
    )
}

async fn list_tasks(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<ApiResponse<Vec<TaskSummaryDto>>, ApiError> {
    let list = TaskRepo::list_all(&state.db).await?;
    let dtos = list.into_iter().map(TaskSummaryDto::from).collect();
    Ok(ApiResponse::success(dtos))
}

async fn get_task(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(camera_id): Path<String>,
) -> Result<ApiResponse<TaskConfigDto>, ApiError> {
    if let Some(task) = TaskRepo::find_by_camera_id(&state.db, &camera_id).await? {
        let rules: Vec<DetectionRule> = serde_json::from_str(&task.rules_json).unwrap_or_default();
        let motion_gate: Option<MotionGateConfig> =
            serde_json::from_str(&task.motion_gate_json).ok();

        Ok(ApiResponse::success(TaskConfigDto {
            camera_id: task.camera_id,
            name: task.name,
            desired_enabled: task.desired_enabled,
            rules,
            motion_gate,
        }))
    } else {
        // 未配置时返回默认结构
        Ok(ApiResponse::success(TaskConfigDto {
            camera_id: camera_id.clone(),
            name: format!("Task-{camera_id}"),
            desired_enabled: false,
            rules: Vec::new(),
            motion_gate: Some(MotionGateConfig::default()),
        }))
    }
}

async fn update_task(
    State(state): State<AppState>,
    user: AuthUser,
    Path(camera_id): Path<String>,
    Json(dto): Json<TaskConfigDto>,
) -> Result<ApiResponse<TaskConfigDto>, ApiError> {
    let rules_json = serde_json::to_string(&dto.rules).unwrap_or_else(|_| "[]".to_string());
    let mg = dto.motion_gate.clone().unwrap_or_default();
    let mg_json = serde_json::to_string(&mg).unwrap_or_else(|_| "{}".to_string());

    let saved = TaskRepo::save_or_update(
        &state.db,
        &camera_id,
        &dto.name,
        dto.desired_enabled,
        &rules_json,
        &mg_json,
    )
    .await?;

    // 同步更新 PipelineManager 的规则引擎集合与 AI 分析活跃状态
    state
        .pipeline
        .set_camera_rules(&camera_id, dto.rules.clone())
        .await;

    state
        .pipeline
        .set_ai_active(&camera_id, dto.desired_enabled)
        .await;

    // 记录操作审计日志
    let req_json = serde_json::to_string(&dto).unwrap_or_default();
    let _ = db::OplogRepo::record(
        &state.db,
        &user.username,
        "task",
        "update_rules",
        "PUT",
        &format!("/api/v1/tasks/{camera_id}"),
        "",
        &req_json,
        200,
        0,
        "",
        "",
    )
    .await;

    Ok(ApiResponse::success(TaskConfigDto {
        camera_id: saved.camera_id,
        name: saved.name,
        desired_enabled: saved.desired_enabled,
        rules: dto.rules,
        motion_gate: dto.motion_gate,
    }))
}

async fn delete_task(
    State(state): State<AppState>,
    user: AuthUser,
    Path(camera_id): Path<String>,
) -> Result<ApiResponse<()>, ApiError> {
    let rows = TaskRepo::delete_by_camera_id(&state.db, &camera_id).await?;
    if rows == 0 {
        return Err(ApiError::NotFound(format!("任务未找到: {camera_id}")));
    }

    // 清理 pipeline 规则、停止任务并释放解码器
    let _ = state.pipeline.stop_task(&camera_id).await;
    state
        .pipeline
        .set_camera_rules(&camera_id, Vec::new())
        .await;
    state.pipeline.set_ai_active(&camera_id, false).await;

    // 记录审计日志
    let _ = db::OplogRepo::record(
        &state.db,
        &user.username,
        "task",
        "delete",
        "DELETE",
        &format!("/api/v1/tasks/{camera_id}"),
        "",
        "",
        200,
        0,
        "",
        "",
    )
    .await;

    Ok(ApiResponse::success(()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use sea_orm::Set;
    use tower::ServiceExt;

    async fn setup_test_app() -> (axum::Router, AppState, String) {
        let db = db::init_test_db().await.unwrap();
        let pipeline = std::sync::Arc::new(pipeline::PipelineManager::new());
        let state = AppState::new(db, pipeline);
        state.sync_auth_state().await;

        let password_hash =
            crate::crypto::hash_password_async("adminPassword123".to_string()).await;
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
        let token = crate::crypto::generate_jwt(&claims, &state.get_jwt_secret()).unwrap();

        let app = crate::create_app(state.clone());
        (app, state, token)
    }

    #[tokio::test]
    async fn test_task_crud_lifecycle() {
        let (app, state, token) = setup_test_app().await;

        // 1. 创建关联摄像头
        let camera_model = db::entity::camera::ActiveModel {
            id: sea_orm::ActiveValue::NotSet,
            camera_id: Set("CAM-TASK-01".to_string()),
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

        // 2. 初始获取任务（默认空规则）
        let req = Request::builder()
            .uri("/api/v1/tasks/CAM-TASK-01")
            .method("GET")
            .header("Authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // 3. 更新任务规则
        let update_body = serde_json::json!({
            "cameraId": "CAM-TASK-01",
            "name": "周界入侵防护",
            "desiredEnabled": true,
            "rules": [
                {
                    "id": "rule-01",
                    "name": "禁区入侵",
                    "role": "roi",
                    "points": [{"x": 0.1, "y": 0.1}, {"x": 0.9, "y": 0.1}, {"x": 0.9, "y": 0.9}]
                }
            ],
            "motionGate": {
                "enabled": true,
                "threshold": 25
            }
        });
        let req = Request::builder()
            .uri("/api/v1/tasks/CAM-TASK-01")
            .method("PUT")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_vec(&update_body).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // 4. 列出全部任务
        let req = Request::builder()
            .uri("/api/v1/tasks")
            .method("GET")
            .header("Authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let list_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        let items = list_json["data"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["cameraId"], "CAM-TASK-01");
        assert_eq!(items[0]["rulesCount"], 1);

        // 5. 删除任务
        let req = Request::builder()
            .uri("/api/v1/tasks/CAM-TASK-01")
            .method("DELETE")
            .header("Authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // 6. 再次删除返回 404
        let req = Request::builder()
            .uri("/api/v1/tasks/CAM-TASK-01")
            .method("DELETE")
            .header("Authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
