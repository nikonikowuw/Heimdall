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
    Router::new()
        .route("/", get(list_tasks))
        .route("/{camera_id}", get(get_task).put(update_task))
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
