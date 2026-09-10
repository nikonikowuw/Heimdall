use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use db::{AlgorithmInstanceRepo, CreateInstanceParams, TaskRepo, UpdateInstanceParams};
use types::{DetectionRule, MotionGateConfig};

use crate::error::ApiError;
use crate::response::ApiResponse;
use crate::state::AppState;
use crate::task_service;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListInstancesQuery {
    pub camera_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlgorithmInstanceDto {
    pub id: i64,
    pub instance_id: String,
    pub camera_id: String,
    pub algorithm_id: String,
    pub analysis_fps: i32,
    pub params: serde_json::Value,
    pub rules: serde_json::Value,
    pub motion_gate: serde_json::Value,
    pub enabled: bool,
    pub actual_status: i32,
    pub status_message: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<db::entity::algorithm_instance::Model> for AlgorithmInstanceDto {
    fn from(m: db::entity::algorithm_instance::Model) -> Self {
        let params = serde_json::from_str(&m.params_json).unwrap_or_else(|_| serde_json::json!({}));
        let rules = serde_json::from_str(&m.rules_json).unwrap_or_else(|_| serde_json::json!([]));
        let motion_gate =
            serde_json::from_str(&m.motion_gate_json).unwrap_or_else(|_| serde_json::json!({}));

        Self {
            id: m.id,
            instance_id: m.instance_id,
            camera_id: m.camera_id,
            algorithm_id: m.algorithm_id,
            analysis_fps: m.analysis_fps,
            params,
            rules,
            motion_gate,
            enabled: m.enabled,
            actual_status: m.actual_status,
            status_message: m.status_message,
            created_at: m.created_at.timestamp_millis(),
            updated_at: m.updated_at.timestamp_millis(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateInstanceRequest {
    pub camera_id: String,
    pub algorithm_id: String,
    pub analysis_fps: Option<i32>,
    pub params: Option<serde_json::Value>,
    pub rules: Option<serde_json::Value>,
    pub motion_gate: Option<serde_json::Value>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInstanceRequest {
    pub analysis_fps: Option<i32>,
    pub params: Option<serde_json::Value>,
    pub rules: Option<serde_json::Value>,
    pub motion_gate: Option<serde_json::Value>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetInstanceEnabledRequest {
    pub enabled: bool,
}

fn default_algo_params() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskConfigDto {
    pub camera_id: String,
    pub name: String,
    pub desired_enabled: bool,
    #[serde(default)]
    pub algorithm_id: String,
    #[serde(default)]
    pub analysis_fps: i32,
    #[serde(default = "default_algo_params")]
    pub algo_params: serde_json::Value,
    #[serde(default)]
    pub actual_status: i32,
    #[serde(default)]
    pub status_message: String,
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
    pub status_message: String,
    pub algorithm_id: String,
    pub analysis_fps: i32,
    pub algo_params: serde_json::Value,
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
        let algo_params: serde_json::Value =
            serde_json::from_str(&m.algo_params_json).unwrap_or_else(|_| default_algo_params());

        Self {
            id: m.id,
            camera_id: m.camera_id,
            name: m.name,
            desired_enabled: m.desired_enabled,
            actual_status: m.actual_status,
            status_message: m.status_message,
            algorithm_id: m.algorithm_id,
            analysis_fps: m.analysis_fps,
            algo_params,
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
        .route("/instances", get(list_instances).post(create_instance))
        .route(
            "/instances/{instance_id}",
            axum::routing::put(update_instance).delete(delete_instance),
        )
        .route(
            "/instances/{instance_id}/enabled",
            axum::routing::put(set_instance_enabled),
        )
        .route(
            "/{camera_id}",
            get(get_task).put(update_task).delete(delete_task),
        )
}

async fn list_tasks(
    State(state): State<AppState>,
) -> Result<ApiResponse<Vec<TaskSummaryDto>>, ApiError> {
    let list = TaskRepo::list_all(&state.db).await?;
    let dtos = list.into_iter().map(TaskSummaryDto::from).collect();
    Ok(ApiResponse::success(dtos))
}

async fn get_task(
    State(state): State<AppState>,
    Path(camera_id): Path<String>,
) -> Result<ApiResponse<TaskConfigDto>, ApiError> {
    if let Some(task) = TaskRepo::find_by_camera_id(&state.db, &camera_id).await? {
        let rules: Vec<DetectionRule> = serde_json::from_str(&task.rules_json).unwrap_or_default();
        let motion_gate: Option<MotionGateConfig> =
            serde_json::from_str(&task.motion_gate_json).ok();
        let algo_params: serde_json::Value =
            serde_json::from_str(&task.algo_params_json).unwrap_or_else(|_| default_algo_params());

        Ok(ApiResponse::success(TaskConfigDto {
            camera_id: task.camera_id,
            name: task.name,
            desired_enabled: task.desired_enabled,
            algorithm_id: task.algorithm_id,
            analysis_fps: task.analysis_fps,
            algo_params,
            actual_status: task.actual_status,
            status_message: task.status_message,
            rules,
            motion_gate,
        }))
    } else {
        // 未配置时返回默认结构
        Ok(ApiResponse::success(TaskConfigDto {
            camera_id: camera_id.clone(),
            name: format!("Task-{camera_id}"),
            desired_enabled: false,
            algorithm_id: String::new(),
            analysis_fps: 0,
            algo_params: default_algo_params(),
            actual_status: 0,
            status_message: String::new(),
            rules: Vec::new(),
            motion_gate: Some(MotionGateConfig::default()),
        }))
    }
}

async fn update_task(
    State(state): State<AppState>,
    Path(camera_id): Path<String>,
    Json(dto): Json<TaskConfigDto>,
) -> Result<ApiResponse<TaskConfigDto>, ApiError> {
    let cam_id = camera_id.trim();
    if cam_id.is_empty() || cam_id.len() > 128 || cam_id.contains('\0') {
        return Err(ApiError::BadRequest("cameraId 非法".to_string()));
    }

    if dto.analysis_fps < 0 || dto.analysis_fps > 60 {
        return Err(ApiError::BadRequest(
            "analysisFps 必须在 0..=60 之间".to_string(),
        ));
    }

    if !dto.algo_params.is_object() {
        return Err(ApiError::BadRequest(
            "algoParams 必须为 JSON Object 对象".to_string(),
        ));
    }

    let algo_params_json =
        serde_json::to_string(&dto.algo_params).unwrap_or_else(|_| "{}".to_string());
    if algo_params_json.len() > 64 * 1024 || algo_params_json.contains('\0') {
        return Err(ApiError::BadRequest(
            "algoParams 序列化过大或包含非法字符".to_string(),
        ));
    }

    let camera = db::CameraRepo::find_by_camera_id(&state.db, &camera_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound(format!("关联摄像头未找到: {camera_id}")))?;

    let target_algo_id = task_service::resolve_algorithm_id(
        &state.db,
        &state.algo_registry,
        &camera_id,
        &dto.algorithm_id,
        dto.desired_enabled,
    )
    .await?;

    let rules_json = serde_json::to_string(&dto.rules).unwrap_or_else(|_| "[]".to_string());
    let mg = dto.motion_gate.clone().unwrap_or_default();
    let mg_json = serde_json::to_string(&mg).unwrap_or_else(|_| "{}".to_string());

    // 1. 事务保存期望配置至数据库并双写关联主算法实例
    let saved = TaskRepo::save_task_and_sync_instance(
        &state.db,
        db::SaveTaskParams {
            camera_id: camera_id.clone(),
            name: dto.name.clone(),
            desired_enabled: dto.desired_enabled,
            algorithm_id: target_algo_id.clone(),
            analysis_fps: dto.analysis_fps,
            algo_params_json: algo_params_json.clone(),
            rules_json,
            motion_gate_json: mg_json,
        },
    )
    .await
    .map_err(|e| match e {
        db::DbError::Validation(msg) => ApiError::BadRequest(msg),
        db::DbError::NotFound { entity, key } => {
            ApiError::NotFound(format!("未找到{entity}: {key}"))
        }
        other => ApiError::Internal(other.to_string()),
    })?;

    // 2. 同步更新 PipelineManager 的空间几何规则
    state
        .pipeline
        .set_camera_rules(&camera_id, dto.rules.clone())
        .await;

    // 3. 编排运行时启停并同步实际状态
    let (final_actual_status, final_status_message) = if dto.desired_enabled {
        let main_url = camera.rtsp_url.trim().to_string();
        if main_url.is_empty() {
            let err_msg = "摄像头主码流 RTSP 地址为空".to_string();
            TaskRepo::update_status(
                &state.db,
                &camera_id,
                types::TaskStatus::Error.as_i32(),
                &err_msg,
            )
            .await?;
            state.pipeline.set_ai_active(&camera_id, false).await;
            (types::TaskStatus::Error.as_i32(), err_msg)
        } else {
            let params = task_service::build_start_params(
                &camera_id,
                &camera,
                target_algo_id.clone(),
                dto.algo_params.clone(),
                dto.analysis_fps,
                dto.motion_gate.as_ref(),
            );

            match state.task_coordinator.start_camera_pipeline(params).await {
                Ok(_) => {
                    if let Err(err) = TaskRepo::update_status(
                        &state.db,
                        &camera_id,
                        types::TaskStatus::Running.as_i32(),
                        "",
                    )
                    .await
                    {
                        if let Err(stop_err) = state
                            .task_coordinator
                            .stop_camera_pipeline(&camera_id)
                            .await
                        {
                            tracing::error!(
                                camera_id = %camera_id,
                                error = %stop_err,
                                "运行状态写入失败后回滚分析管线也失败"
                            );
                        }
                        state.pipeline.set_ai_active(&camera_id, false).await;
                        return Err(ApiError::Db(err));
                    }
                    state.pipeline.set_ai_active(&camera_id, true).await;
                    (types::TaskStatus::Running.as_i32(), String::new())
                }
                Err(e) => {
                    let err_msg = format!("启动分析管线失败: {e}");
                    tracing::warn!(
                        camera_id = %camera_id,
                        error = %err_msg,
                        "分析管线启动失败，保留期望状态并记录错误"
                    );
                    TaskRepo::update_status(
                        &state.db,
                        &camera_id,
                        types::TaskStatus::Error.as_i32(),
                        &err_msg,
                    )
                    .await?;
                    state.pipeline.set_ai_active(&camera_id, false).await;
                    (types::TaskStatus::Error.as_i32(), err_msg)
                }
            }
        }
    } else {
        // 停用分析管线
        match state
            .task_coordinator
            .stop_camera_pipeline(&camera_id)
            .await
        {
            Ok(_) => {
                state.pipeline.set_ai_active(&camera_id, false).await;
                TaskRepo::update_status(
                    &state.db,
                    &camera_id,
                    types::TaskStatus::Stopped.as_i32(),
                    "",
                )
                .await?;
                (types::TaskStatus::Stopped.as_i32(), String::new())
            }
            Err(e) => {
                let err_msg = format!("停止分析管线失败: {e}");
                tracing::warn!(
                    camera_id = %camera_id,
                    error = %err_msg,
                    "分析管线停止失败，保留期望状态并记录错误"
                );
                TaskRepo::update_status(
                    &state.db,
                    &camera_id,
                    types::TaskStatus::Error.as_i32(),
                    &err_msg,
                )
                .await?;
                (types::TaskStatus::Error.as_i32(), err_msg)
            }
        }
    };

    let saved_algo_params: serde_json::Value =
        serde_json::from_str(&saved.algo_params_json).unwrap_or_else(|_| default_algo_params());

    Ok(ApiResponse::success(TaskConfigDto {
        camera_id: saved.camera_id,
        name: saved.name,
        desired_enabled: saved.desired_enabled,
        algorithm_id: saved.algorithm_id,
        analysis_fps: saved.analysis_fps,
        algo_params: saved_algo_params,
        actual_status: final_actual_status,
        status_message: final_status_message,
        rules: dto.rules,
        motion_gate: dto.motion_gate,
    }))
}

async fn delete_task(
    State(state): State<AppState>,
    Path(camera_id): Path<String>,
) -> Result<ApiResponse<()>, ApiError> {
    // 1. 严格先停止运行时与媒体订阅，确保解码器与分析泵安全回收
    state
        .task_coordinator
        .stop_camera_pipeline(&camera_id)
        .await
        .map_err(ApiError::Coordinator)?;

    // 2. 清理 pipeline 规则、停止底层任务上下文
    if let Err(err) = state.pipeline.stop_task(&camera_id).await {
        if !matches!(err, pipeline::PipelineError::PipelineNotFound { .. }) {
            return Err(ApiError::Pipeline(err));
        }
    }
    state
        .pipeline
        .set_camera_rules(&camera_id, Vec::new())
        .await;
    state.pipeline.set_ai_active(&camera_id, false).await;

    // 3. 删除数据库记录 (原子级联删除 analysis_tasks 与 algorithm_instances)
    let rows = TaskRepo::delete_task_and_instance(&state.db, &camera_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    if rows == 0 {
        return Err(ApiError::NotFound(format!("任务未找到: {camera_id}")));
    }

    Ok(ApiResponse::success(()))
}

// -------------------------------------------------------------
// 算法实例路由处理函数
// -------------------------------------------------------------

async fn list_instances(
    State(state): State<AppState>,
    Query(query): Query<ListInstancesQuery>,
) -> Result<ApiResponse<Vec<AlgorithmInstanceDto>>, ApiError> {
    let list = if let Some(cid) = query.camera_id {
        AlgorithmInstanceRepo::list_by_camera_id(&state.db, &cid).await?
    } else {
        AlgorithmInstanceRepo::list_all(&state.db).await?
    };

    let dtos = list.into_iter().map(AlgorithmInstanceDto::from).collect();
    Ok(ApiResponse::success(dtos))
}

async fn create_instance(
    State(state): State<AppState>,
    Json(req): Json<CreateInstanceRequest>,
) -> Result<ApiResponse<AlgorithmInstanceDto>, ApiError> {
    let instance_id = uuid::Uuid::new_v4().to_string();
    let params_json = req
        .params
        .as_ref()
        .map(|v| v.to_string())
        .unwrap_or_else(|| "{}".into());
    let rules_json = req
        .rules
        .as_ref()
        .map(|v| v.to_string())
        .unwrap_or_else(|| "[]".into());
    let motion_gate_json = req
        .motion_gate
        .as_ref()
        .map(|v| v.to_string())
        .unwrap_or_else(|| "{}".into());
    let enabled = req.enabled.unwrap_or(false);
    let fps = req.analysis_fps.unwrap_or(0);

    let created = AlgorithmInstanceRepo::create(
        &state.db,
        CreateInstanceParams {
            instance_id: instance_id.clone(),
            camera_id: req.camera_id.clone(),
            algorithm_id: req.algorithm_id.clone(),
            analysis_fps: fps,
            params_json,
            rules_json,
            motion_gate_json,
            enabled,
        },
    )
    .await?;

    Ok(ApiResponse::success(AlgorithmInstanceDto::from(created)))
}

async fn update_instance(
    State(state): State<AppState>,
    Path(instance_id): Path<String>,
    Json(req): Json<UpdateInstanceRequest>,
) -> Result<ApiResponse<AlgorithmInstanceDto>, ApiError> {
    let updated = AlgorithmInstanceRepo::update(
        &state.db,
        &instance_id,
        UpdateInstanceParams {
            analysis_fps: req.analysis_fps,
            params_json: req.params.map(|v| v.to_string()),
            rules_json: req.rules.map(|v| v.to_string()),
            motion_gate_json: req.motion_gate.map(|v| v.to_string()),
            enabled: req.enabled,
        },
    )
    .await?;

    Ok(ApiResponse::success(AlgorithmInstanceDto::from(updated)))
}

async fn set_instance_enabled(
    State(state): State<AppState>,
    Path(instance_id): Path<String>,
    Json(req): Json<SetInstanceEnabledRequest>,
) -> Result<ApiResponse<Option<()>>, ApiError> {
    AlgorithmInstanceRepo::set_enabled(&state.db, &instance_id, req.enabled).await?;

    Ok(ApiResponse::success(None))
}

async fn delete_instance(
    State(state): State<AppState>,
    Path(instance_id): Path<String>,
) -> Result<ApiResponse<Option<()>>, ApiError> {
    let rows = AlgorithmInstanceRepo::delete(&state.db, &instance_id).await?;
    if rows == 0 {
        return Err(ApiError::NotFound(format!("算法实例未找到: {instance_id}")));
    }

    Ok(ApiResponse::success(None))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use pipeline::TaskRuntimeService;
    use sea_orm::Set;
    use std::sync::Arc;
    use tower::ServiceExt;

    #[derive(Debug, Default)]
    struct MockTaskRuntimeService {
        started: std::sync::Mutex<Vec<pipeline::StartCameraPipelineParams>>,
        stopped: std::sync::Mutex<Vec<String>>,
        active_cameras: std::sync::Mutex<std::collections::HashSet<String>>,
        active_params: std::sync::Mutex<
            std::collections::HashMap<String, pipeline::StartCameraPipelineParams>,
        >,
        should_fail: std::sync::Mutex<Option<String>>,
        stop_should_fail: std::sync::Mutex<Option<String>>,
    }

    #[allow(dead_code)]
    impl MockTaskRuntimeService {
        fn new() -> Self {
            Self::default()
        }

        fn set_fail(&self, reason: &str) {
            *self.should_fail.lock().unwrap() = Some(reason.to_string());
        }

        fn clear_fail(&self) {
            *self.should_fail.lock().unwrap() = None;
        }

        fn set_stop_fail(&self, reason: &str) {
            *self.stop_should_fail.lock().unwrap() = Some(reason.to_string());
        }

        fn clear_stop_fail(&self) {
            *self.stop_should_fail.lock().unwrap() = None;
        }

        fn started_params(&self) -> Vec<pipeline::StartCameraPipelineParams> {
            self.started.lock().unwrap().clone()
        }

        fn stopped_cameras(&self) -> Vec<String> {
            self.stopped.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl pipeline::TaskRuntimeService for MockTaskRuntimeService {
        async fn start_camera_pipeline(
            &self,
            params: pipeline::StartCameraPipelineParams,
        ) -> Result<u64, pipeline::CoordinatorError> {
            if let Some(reason) = self.should_fail.lock().unwrap().as_ref() {
                return Err(pipeline::CoordinatorError::Validation {
                    reason: reason.clone(),
                });
            }

            let camera_id = params.camera_id.clone();
            let mut active_params = self.active_params.lock().unwrap();
            if active_params
                .get(&camera_id)
                .is_some_and(|active| active == &params)
            {
                return Ok(1);
            }
            if active_params.remove(&camera_id).is_some() {
                self.stopped.lock().unwrap().push(camera_id.clone());
            }
            active_params.insert(camera_id.clone(), params.clone());
            drop(active_params);
            self.active_cameras.lock().unwrap().insert(camera_id);
            self.started.lock().unwrap().push(params);
            Ok(1)
        }

        async fn stop_camera_pipeline(
            &self,
            camera_id: &str,
        ) -> Result<bool, pipeline::CoordinatorError> {
            if let Some(reason) = self.stop_should_fail.lock().unwrap().as_ref() {
                return Err(pipeline::CoordinatorError::Validation {
                    reason: reason.clone(),
                });
            }
            let had_runtime = self
                .active_params
                .lock()
                .unwrap()
                .remove(camera_id)
                .is_some();
            self.active_cameras.lock().unwrap().remove(camera_id);
            self.stopped.lock().unwrap().push(camera_id.to_string());
            Ok(had_runtime)
        }

        async fn stop_all(&self) {
            self.active_params.lock().unwrap().clear();
            self.active_cameras.lock().unwrap().clear();
        }

        async fn get_runtime_info(
            &self,
            camera_id: &str,
        ) -> Option<pipeline::CameraPipelineRuntimeInfo> {
            if self.active_cameras.lock().unwrap().contains(camera_id) {
                Some(pipeline::CameraPipelineRuntimeInfo {
                    camera_id: camera_id.to_string(),
                    generation: 1,
                    algorithm_id: "test_algo".to_string(),
                    target_fps: 10,
                    motion_gate_enabled: true,
                    is_pump_running: true,
                    frames_decoded: 100,
                    frames_inferred: 50,
                    alarms_triggered: 2,
                    started_at_ms: 1000,
                })
            } else {
                None
            }
        }

        async fn list_runtime_infos(&self) -> Vec<pipeline::CameraPipelineRuntimeInfo> {
            let active = self.active_cameras.lock().unwrap().clone();
            active
                .into_iter()
                .map(|cid| pipeline::CameraPipelineRuntimeInfo {
                    camera_id: cid,
                    generation: 1,
                    algorithm_id: "test_algo".to_string(),
                    target_fps: 10,
                    motion_gate_enabled: true,
                    is_pump_running: true,
                    frames_decoded: 100,
                    frames_inferred: 50,
                    alarms_triggered: 2,
                    started_at_ms: 1000,
                })
                .collect()
        }

        async fn has_active_runtime(&self, camera_id: &str) -> bool {
            self.active_cameras.lock().unwrap().contains(camera_id)
        }
    }

    async fn setup_test_app() -> (axum::Router, AppState, String, Arc<MockTaskRuntimeService>) {
        let db = db::init_test_db().await.unwrap();
        let pipeline = std::sync::Arc::new(pipeline::PipelineManager::new());
        let mock_coord = Arc::new(MockTaskRuntimeService::new());
        let state = AppState::new(db, pipeline).with_task_coordinator(mock_coord.clone());
        crate::sync_auth_state(&state).await;

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
        (app, state, token, mock_coord)
    }

    #[tokio::test]
    async fn test_task_crud_lifecycle() {
        let (app, state, token, _mock_coord) = setup_test_app().await;

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

        db::AlgorithmRepo::upsert_algorithm(
            &state.db,
            db::UpsertAlgorithmParams {
                algorithm_id: "crud_algo".to_string(),
                name: "测试算法".to_string(),
                algorithm_type: "detection".to_string(),
                alarm_type_id: "INTRUSION".to_string(),
                active_version: "1.0.0".to_string(),
                description: "测试算法".to_string(),
                is_builtin: true,
            },
        )
        .await
        .unwrap();

        // 1. 禁用任务不应凭空选择默认算法或创建算法实例。
        let disable_without_algorithm = serde_json::json!({
            "cameraId": "CAM-TASK-01",
            "name": "未启用任务",
            "desiredEnabled": false,
            "rules": []
        });
        let req = Request::builder()
            .uri("/api/v1/tasks/CAM-TASK-01")
            .method("PUT")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&disable_without_algorithm).unwrap(),
            ))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let instances = db::AlgorithmInstanceRepo::list_by_camera_id(&state.db, "CAM-TASK-01")
            .await
            .unwrap();
        assert!(instances.is_empty());

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
            "algorithmId": "crud_algo",
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

    #[tokio::test]
    async fn test_task_algo_binding_and_validation() {
        let (app, state, token, mock_coord) = setup_test_app().await;

        // 创建关联摄像头
        let camera_model = db::entity::camera::ActiveModel {
            id: sea_orm::ActiveValue::NotSet,
            camera_id: Set("CAM-ALGO-01".to_string()),
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

        // 预置算法
        db::AlgorithmRepo::upsert_algorithm(
            &state.db,
            db::UpsertAlgorithmParams {
                algorithm_id: "yolov8_detector".to_string(),
                name: "YOLOv8通用检测".to_string(),
                algorithm_type: "detection".to_string(),
                alarm_type_id: "INTRUSION".to_string(),
                active_version: "1.0.0".to_string(),
                description: "测试算法".to_string(),
                is_builtin: true,
            },
        )
        .await
        .unwrap();

        // 1. 负数 analysisFps 被拒绝 (400)
        let invalid_fps = serde_json::json!({
            "cameraId": "CAM-ALGO-01",
            "name": "非法帧率任务",
            "desiredEnabled": true,
            "analysisFps": -5,
            "algoParams": {},
            "rules": []
        });
        let req = Request::builder()
            .uri("/api/v1/tasks/CAM-ALGO-01")
            .method("PUT")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_vec(&invalid_fps).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        // 2. 非对象 algoParams 被拒绝 (400)
        let invalid_params = serde_json::json!({
            "cameraId": "CAM-ALGO-01",
            "name": "非法参数任务",
            "desiredEnabled": true,
            "analysisFps": 15,
            "algoParams": [1, 2, 3],
            "rules": []
        });
        let req = Request::builder()
            .uri("/api/v1/tasks/CAM-ALGO-01")
            .method("PUT")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_vec(&invalid_params).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        // 3. 不存在的 algorithmId 返回 404
        let non_existent_algo = serde_json::json!({
            "cameraId": "CAM-ALGO-01",
            "name": "不存在算法",
            "desiredEnabled": true,
            "algorithmId": "ghost_algo",
            "analysisFps": 15,
            "algoParams": {},
            "rules": []
        });
        let req = Request::builder()
            .uri("/api/v1/tasks/CAM-ALGO-01")
            .method("PUT")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_vec(&non_existent_algo).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        // 4. 未指定算法但没有已注册可运行算法时拒绝请求且不落库。
        let missing_default_algo = serde_json::json!({
            "cameraId": "CAM-ALGO-01",
            "name": "无默认算法",
            "desiredEnabled": true,
            "analysisFps": 15,
            "algoParams": {},
            "rules": []
        });
        let req = Request::builder()
            .uri("/api/v1/tasks/CAM-ALGO-01")
            .method("PUT")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&missing_default_algo).unwrap(),
            ))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert!(db::TaskRepo::find_by_camera_id(&state.db, "CAM-ALGO-01")
            .await
            .unwrap()
            .is_none());

        // 5. 正确配置成功保存并回显 camelCase 字段
        let valid_config = serde_json::json!({
            "cameraId": "CAM-ALGO-01",
            "name": "合法布防任务",
            "desiredEnabled": true,
            "algorithmId": "yolov8_detector",
            "analysisFps": 20,
            "algoParams": {"confidence": 0.65},
            "rules": []
        });
        let req = Request::builder()
            .uri("/api/v1/tasks/CAM-ALGO-01")
            .method("PUT")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_vec(&valid_config).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        let data = &resp_json["data"];
        assert_eq!(data["algorithmId"], "yolov8_detector");
        assert_eq!(data["analysisFps"], 20);
        assert_eq!(data["algoParams"]["confidence"], 0.65);
        assert_eq!(data["actualStatus"], 2);
        assert_eq!(mock_coord.started_params().len(), 1);

        // 5. GET 查询回显完整字段
        let req = Request::builder()
            .uri("/api/v1/tasks/CAM-ALGO-01")
            .method("GET")
            .header("Authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let get_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        let get_data = &get_json["data"];
        assert_eq!(get_data["algorithmId"], "yolov8_detector");
        assert_eq!(get_data["analysisFps"], 20);
        assert_eq!(get_data["algoParams"]["confidence"], 0.65);

        // 6. 验证底库自动创建了主算法实例
        let instances = db::AlgorithmInstanceRepo::list_by_camera_id(&state.db, "CAM-ALGO-01")
            .await
            .unwrap();
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].algorithm_id, "yolov8_detector");
        assert_eq!(instances[0].analysis_fps, 20);

        // 7. DELETE 任务同时级联删除算法实例
        let req = Request::builder()
            .uri("/api/v1/tasks/CAM-ALGO-01")
            .method("DELETE")
            .header("Authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let instances_after =
            db::AlgorithmInstanceRepo::list_by_camera_id(&state.db, "CAM-ALGO-01")
                .await
                .unwrap();
        assert!(instances_after.is_empty());
    }

    #[tokio::test]
    async fn test_task_enable_pipeline_failure_preserves_desired_records_error() {
        let (app, state, token, mock_coord) = setup_test_app().await;

        // 1. 预置摄像头与算法
        let camera_model = db::entity::camera::ActiveModel {
            id: sea_orm::ActiveValue::NotSet,
            camera_id: Set("CAM-FAIL-01".to_string()),
            name: Set("异常测试摄像头".to_string()),
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

        db::AlgorithmRepo::upsert_algorithm(
            &state.db,
            db::UpsertAlgorithmParams {
                algorithm_id: "test_detector".to_string(),
                name: "测试算法".to_string(),
                algorithm_type: "detection".to_string(),
                alarm_type_id: "INTRUSION".to_string(),
                active_version: "1.0.0".to_string(),
                description: "测试算法".to_string(),
                is_builtin: true,
            },
        )
        .await
        .unwrap();

        // 2. 模拟协调器启动报错 (如 NPU 显存不足)
        mock_coord.set_fail("NPU 显存申请超限 (512MB 溢出)");

        let payload = serde_json::json!({
            "cameraId": "CAM-FAIL-01",
            "name": "故障布防任务",
            "desiredEnabled": true,
            "algorithmId": "test_detector",
            "analysisFps": 15,
            "algoParams": {},
            "rules": []
        });
        let req = Request::builder()
            .uri("/api/v1/tasks/CAM-FAIL-01")
            .method("PUT")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_vec(&payload).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        let data = &resp_json["data"];

        // 3. 验证返回保持 desiredEnabled=true，实际状态为 Error(5)，附带诊断错误说明
        assert_eq!(data["desiredEnabled"], true);
        assert_eq!(data["actualStatus"], 5);
        assert!(data["statusMessage"]
            .as_str()
            .unwrap()
            .contains("NPU 显存申请超限"));

        // 4. 验证数据库中 Task 与 AlgorithmInstance 均写入 Error(5) 状态，杜绝假激活
        let task_in_db = db::TaskRepo::find_by_camera_id(&state.db, "CAM-FAIL-01")
            .await
            .unwrap()
            .unwrap();
        assert!(task_in_db.desired_enabled);
        assert_eq!(task_in_db.actual_status, 5);
        assert!(task_in_db.status_message.contains("NPU 显存申请超限"));

        let instances = db::AlgorithmInstanceRepo::list_by_camera_id(&state.db, "CAM-FAIL-01")
            .await
            .unwrap();
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].actual_status, 5);
        assert!(instances[0].status_message.contains("NPU 显存申请超限"));
    }

    #[tokio::test]
    async fn test_task_disable_stops_pipeline_and_sets_stopped() {
        let (app, state, token, mock_coord) = setup_test_app().await;

        let camera_model = db::entity::camera::ActiveModel {
            id: sea_orm::ActiveValue::NotSet,
            camera_id: Set("CAM-STOP-01".to_string()),
            name: Set("启停测试摄像头".to_string()),
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

        db::AlgorithmRepo::upsert_algorithm(
            &state.db,
            db::UpsertAlgorithmParams {
                algorithm_id: "stop_algo".to_string(),
                name: "测试算法".to_string(),
                algorithm_type: "detection".to_string(),
                alarm_type_id: "INTRUSION".to_string(),
                active_version: "1.0.0".to_string(),
                description: "测试算法".to_string(),
                is_builtin: true,
            },
        )
        .await
        .unwrap();

        // 1. 先启用
        let enable_payload = serde_json::json!({
            "cameraId": "CAM-STOP-01",
            "name": "启停任务",
            "desiredEnabled": true,
            "algorithmId": "stop_algo",
            "analysisFps": 10,
            "algoParams": {},
            "rules": []
        });
        let req = Request::builder()
            .uri("/api/v1/tasks/CAM-STOP-01")
            .method("PUT")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_vec(&enable_payload).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(mock_coord.has_active_runtime("CAM-STOP-01").await);

        // 2. 首次停用模拟运行时停止失败，不能伪造 Stopped。
        let disable_payload = serde_json::json!({
            "cameraId": "CAM-STOP-01",
            "name": "启停任务",
            "desiredEnabled": false,
            "algorithmId": "stop_algo",
            "analysisFps": 10,
            "algoParams": {},
            "rules": []
        });
        mock_coord.set_stop_fail("停止 worker 超时");
        let disable_req = Request::builder()
            .uri("/api/v1/tasks/CAM-STOP-01")
            .method("PUT")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_vec(&disable_payload).unwrap()))
            .unwrap();
        let failed_resp = app.clone().oneshot(disable_req).await.unwrap();
        assert_eq!(failed_resp.status(), StatusCode::OK);
        let failed_body = axum::body::to_bytes(failed_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let failed_json: serde_json::Value = serde_json::from_slice(&failed_body).unwrap();
        assert_eq!(failed_json["data"]["actualStatus"], 5);
        assert!(mock_coord.has_active_runtime("CAM-STOP-01").await);

        mock_coord.clear_stop_fail();

        // 3. 停止成功后才返回 Stopped。
        let req = Request::builder()
            .uri("/api/v1/tasks/CAM-STOP-01")
            .method("PUT")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_vec(&disable_payload).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        let data = &resp_json["data"];
        assert_eq!(data["desiredEnabled"], false);
        assert_eq!(data["actualStatus"], 0);

        // 4. 验证 coordinator 已被调用停止
        assert!(!mock_coord.has_active_runtime("CAM-STOP-01").await);
        assert!(mock_coord
            .stopped_cameras()
            .contains(&"CAM-STOP-01".to_string()));

        // 5. 验证数据库状态已转为 Stopped(0)
        let task_in_db = db::TaskRepo::find_by_camera_id(&state.db, "CAM-STOP-01")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(task_in_db.actual_status, 0);
    }

    #[tokio::test]
    async fn test_task_idempotent_enable() {
        let (app, state, token, mock_coord) = setup_test_app().await;

        let camera_model = db::entity::camera::ActiveModel {
            id: sea_orm::ActiveValue::NotSet,
            camera_id: Set("CAM-IDEMP-01".to_string()),
            name: Set("幂等摄像头".to_string()),
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

        db::AlgorithmRepo::upsert_algorithm(
            &state.db,
            db::UpsertAlgorithmParams {
                algorithm_id: "idemp_algo".to_string(),
                name: "测试算法".to_string(),
                algorithm_type: "detection".to_string(),
                alarm_type_id: "INTRUSION".to_string(),
                active_version: "1.0.0".to_string(),
                description: "测试算法".to_string(),
                is_builtin: true,
            },
        )
        .await
        .unwrap();

        let payload = serde_json::json!({
            "cameraId": "CAM-IDEMP-01",
            "name": "幂等任务",
            "desiredEnabled": true,
            "algorithmId": "idemp_algo",
            "analysisFps": 10,
            "algoParams": {},
            "rules": []
        });

        // 第一次 PUT
        let req1 = Request::builder()
            .uri("/api/v1/tasks/CAM-IDEMP-01")
            .method("PUT")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_vec(&payload).unwrap()))
            .unwrap();
        let resp1 = app.clone().oneshot(req1).await.unwrap();
        assert_eq!(resp1.status(), StatusCode::OK);

        // 第二次 PUT (完全相同配置)
        let req2 = Request::builder()
            .uri("/api/v1/tasks/CAM-IDEMP-01")
            .method("PUT")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_vec(&payload).unwrap()))
            .unwrap();
        let resp2 = app.clone().oneshot(req2).await.unwrap();
        assert_eq!(resp2.status(), StatusCode::OK);

        // 均成功返回 Running 状态
        let body_bytes = axum::body::to_bytes(resp2.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(resp_json["data"]["actualStatus"], 2);

        // coordinator 识别相同参数并复用既有运行时，不应重复创建 worker。
        assert_eq!(mock_coord.started_params().len(), 1);

        // 变更 FPS 后，coordinator 应先回收旧运行时再启动新配置。
        let changed_payload = serde_json::json!({
            "cameraId": "CAM-IDEMP-01",
            "name": "幂等任务",
            "desiredEnabled": true,
            "algorithmId": "idemp_algo",
            "analysisFps": 20,
            "algoParams": {},
            "rules": []
        });
        let req3 = Request::builder()
            .uri("/api/v1/tasks/CAM-IDEMP-01")
            .method("PUT")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_vec(&changed_payload).unwrap()))
            .unwrap();
        let resp3 = app.clone().oneshot(req3).await.unwrap();
        assert_eq!(resp3.status(), StatusCode::OK);
        assert_eq!(mock_coord.started_params().len(), 2);
        assert!(mock_coord
            .stopped_cameras()
            .contains(&"CAM-IDEMP-01".to_string()));
    }

    #[tokio::test]
    async fn test_task_delete_stops_pipeline_before_db_removal() {
        let (app, state, token, mock_coord) = setup_test_app().await;

        let camera_model = db::entity::camera::ActiveModel {
            id: sea_orm::ActiveValue::NotSet,
            camera_id: Set("CAM-DEL-01".to_string()),
            name: Set("删除测试摄像头".to_string()),
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

        db::AlgorithmRepo::upsert_algorithm(
            &state.db,
            db::UpsertAlgorithmParams {
                algorithm_id: "del_algo".to_string(),
                name: "测试算法".to_string(),
                algorithm_type: "detection".to_string(),
                alarm_type_id: "INTRUSION".to_string(),
                active_version: "1.0.0".to_string(),
                description: "测试算法".to_string(),
                is_builtin: true,
            },
        )
        .await
        .unwrap();

        // 启用任务
        let payload = serde_json::json!({
            "cameraId": "CAM-DEL-01",
            "name": "待删除任务",
            "desiredEnabled": true,
            "algorithmId": "del_algo",
            "analysisFps": 10,
            "algoParams": {},
            "rules": []
        });
        let req = Request::builder()
            .uri("/api/v1/tasks/CAM-DEL-01")
            .method("PUT")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_vec(&payload).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(mock_coord.has_active_runtime("CAM-DEL-01").await);

        // 删除任务
        let req = Request::builder()
            .uri("/api/v1/tasks/CAM-DEL-01")
            .method("DELETE")
            .header("Authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // 验证停止协调器被调用且资源已无
        assert!(!mock_coord.has_active_runtime("CAM-DEL-01").await);
        assert!(mock_coord
            .stopped_cameras()
            .contains(&"CAM-DEL-01".to_string()));

        // 验证数据库任务与实例已全部删除
        let task_in_db = db::TaskRepo::find_by_camera_id(&state.db, "CAM-DEL-01")
            .await
            .unwrap();
        assert!(task_in_db.is_none());

        let instances = db::AlgorithmInstanceRepo::list_by_camera_id(&state.db, "CAM-DEL-01")
            .await
            .unwrap();
        assert!(instances.is_empty());
    }

    #[tokio::test]
    async fn test_task_camera_missing_returns_not_found() {
        let (app, _state, token, _mock_coord) = setup_test_app().await;

        // 对不存在的摄像头配置并启用任务，预期返回 404 Not Found
        let payload = serde_json::json!({
            "cameraId": "CAM-NONEXISTENT",
            "name": "不存在摄像头任务",
            "desiredEnabled": true,
            "algorithmId": "some_algo",
            "analysisFps": 10,
            "algoParams": {},
            "rules": []
        });
        let req = Request::builder()
            .uri("/api/v1/tasks/CAM-NONEXISTENT")
            .method("PUT")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_vec(&payload).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_task_empty_rtsp_url_records_error_status() {
        let (app, state, token, _mock_coord) = setup_test_app().await;

        let camera_model = db::entity::camera::ActiveModel {
            id: sea_orm::ActiveValue::NotSet,
            camera_id: Set("CAM-EMPTY-URL".to_string()),
            name: Set("空地址摄像头".to_string()),
            protocol: Set("rtsp".to_string()),
            rtsp_url: Set("".to_string()), // 空主码流地址
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

        db::AlgorithmRepo::upsert_algorithm(
            &state.db,
            db::UpsertAlgorithmParams {
                algorithm_id: "empty_url_algo".to_string(),
                name: "测试算法".to_string(),
                algorithm_type: "detection".to_string(),
                alarm_type_id: "INTRUSION".to_string(),
                active_version: "1.0.0".to_string(),
                description: "测试算法".to_string(),
                is_builtin: true,
            },
        )
        .await
        .unwrap();

        let payload = serde_json::json!({
            "cameraId": "CAM-EMPTY-URL",
            "name": "空地址布防任务",
            "desiredEnabled": true,
            "algorithmId": "empty_url_algo",
            "analysisFps": 10,
            "algoParams": {},
            "rules": []
        });
        let req = Request::builder()
            .uri("/api/v1/tasks/CAM-EMPTY-URL")
            .method("PUT")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_vec(&payload).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let resp_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        let data = &resp_json["data"];

        assert_eq!(data["desiredEnabled"], true);
        assert_eq!(data["actualStatus"], 5);
        assert_eq!(data["statusMessage"], "摄像头主码流 RTSP 地址为空");

        let task_in_db = db::TaskRepo::find_by_camera_id(&state.db, "CAM-EMPTY-URL")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(task_in_db.actual_status, 5);
        assert_eq!(task_in_db.status_message, "摄像头主码流 RTSP 地址为空");
    }
}
