use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use db::{AlgorithmInstanceRepo, TaskRepo};
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

fn deserialize_null_as_empty_object<'de, D>(deserializer: D) -> Result<serde_json::Value, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt = Option::<serde_json::Value>::deserialize(deserializer)?;
    match opt {
        Some(serde_json::Value::Null) | None => Ok(default_algo_params()),
        Some(v) => Ok(v),
    }
}

/// 响应：算法实例完整信息
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskAlgorithmInstanceDto {
    #[serde(default)]
    pub instance_id: String,
    pub algorithm_id: String,
    #[serde(default)]
    pub analysis_fps: i32,
    #[serde(
        default = "default_algo_params",
        deserialize_with = "deserialize_null_as_empty_object",
        alias = "params"
    )]
    pub algo_params: serde_json::Value,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub actual_status: i32,
    #[serde(default)]
    pub status_message: String,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

impl From<&db::entity::algorithm_instance::Model> for TaskAlgorithmInstanceDto {
    fn from(m: &db::entity::algorithm_instance::Model) -> Self {
        let algo_params =
            serde_json::from_str(&m.params_json).unwrap_or_else(|_| serde_json::json!({}));
        Self {
            instance_id: m.instance_id.clone(),
            algorithm_id: m.algorithm_id.clone(),
            analysis_fps: m.analysis_fps,
            algo_params,
            enabled: Some(m.enabled),
            actual_status: m.actual_status,
            status_message: m.status_message.clone(),
            created_at: m.created_at.timestamp_millis(),
            updated_at: m.updated_at.timestamp_millis(),
        }
    }
}

impl From<db::entity::algorithm_instance::Model> for TaskAlgorithmInstanceDto {
    fn from(m: db::entity::algorithm_instance::Model) -> Self {
        Self::from(&m)
    }
}

/// 响应：任务摘要中的实例简要信息（不含 algoParams）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskAlgorithmInstanceSummaryDto {
    pub instance_id: String,
    pub algorithm_id: String,
    pub analysis_fps: i32,
    pub enabled: bool,
    pub actual_status: i32,
    pub status_message: String,
}

impl From<&db::entity::algorithm_instance::Model> for TaskAlgorithmInstanceSummaryDto {
    fn from(m: &db::entity::algorithm_instance::Model) -> Self {
        Self {
            instance_id: m.instance_id.clone(),
            algorithm_id: m.algorithm_id.clone(),
            analysis_fps: m.analysis_fps,
            enabled: m.enabled,
            actual_status: m.actual_status,
            status_message: m.status_message.clone(),
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
    #[serde(default)]
    pub stream_mode: Option<types::StreamMode>,
    #[serde(default)]
    pub algorithm_instances: Option<Vec<TaskAlgorithmInstanceDto>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    #[serde(default)]
    pub algorithm_instance_count: usize,
    #[serde(default)]
    pub algorithm_instances: Vec<TaskAlgorithmInstanceSummaryDto>,
    pub rules_count: usize,
    pub motion_gate_enabled: bool,
    pub rules: Vec<DetectionRule>,
    pub motion_gate: Option<MotionGateConfig>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 提取主算法实例属性（若无实例回退默认值）
fn extract_primary_instance_props(
    instances: &[db::entity::algorithm_instance::Model],
) -> (String, i32, serde_json::Value) {
    let primary = instances.first();
    let algorithm_id = primary.map(|i| i.algorithm_id.clone()).unwrap_or_default();
    let analysis_fps = primary.map(|i| i.analysis_fps).unwrap_or(0);
    let algo_params = primary
        .map(|i| serde_json::from_str(&i.params_json).unwrap_or_else(|_| default_algo_params()))
        .unwrap_or_else(default_algo_params);
    (algorithm_id, analysis_fps, algo_params)
}

/// 构建 TaskConfigDto 的辅助函数（从 task + instances 构建）
fn build_task_config_dto(
    task: &db::entity::task::Model,
    instances: &[db::entity::algorithm_instance::Model],
    stream_mode: Option<types::StreamMode>,
) -> TaskConfigDto {
    let rules = DetectionRule::parse_rules_json(&task.rules_json).unwrap_or_default();
    let motion_gate: Option<MotionGateConfig> = serde_json::from_str(&task.motion_gate_json).ok();
    let (algorithm_id, analysis_fps, algo_params) = extract_primary_instance_props(instances);

    TaskConfigDto {
        camera_id: task.camera_id.clone(),
        name: task.name.clone(),
        desired_enabled: task.desired_enabled,
        algorithm_id,
        analysis_fps,
        algo_params,
        actual_status: task.actual_status,
        status_message: task.status_message.clone(),
        rules,
        motion_gate,
        stream_mode,
        algorithm_instances: Some(
            instances
                .iter()
                .map(TaskAlgorithmInstanceDto::from)
                .collect(),
        ),
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
    let (list, instances_by_task_id) = TaskRepo::list_all_with_instances(&state.db).await?;
    let mut dtos = Vec::with_capacity(list.len());
    for task in list {
        let instances = instances_by_task_id
            .get(&task.id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let rules = DetectionRule::parse_rules_json(&task.rules_json).unwrap_or_default();
        let motion_gate: Option<MotionGateConfig> =
            serde_json::from_str(&task.motion_gate_json).ok();
        let motion_gate_enabled = motion_gate.as_ref().map(|mg| mg.enabled).unwrap_or(false);
        let rules_count = rules.len();

        let (algorithm_id, analysis_fps, algo_params) = extract_primary_instance_props(instances);

        let summary_instances: Vec<TaskAlgorithmInstanceSummaryDto> = instances
            .iter()
            .map(TaskAlgorithmInstanceSummaryDto::from)
            .collect();
        let instance_count = summary_instances.len();

        dtos.push(TaskSummaryDto {
            id: task.id,
            camera_id: task.camera_id,
            name: task.name,
            desired_enabled: task.desired_enabled,
            actual_status: task.actual_status,
            status_message: task.status_message,
            algorithm_id,
            analysis_fps,
            algo_params,
            algorithm_instance_count: instance_count,
            algorithm_instances: summary_instances,
            rules_count,
            motion_gate_enabled,
            rules,
            motion_gate,
            created_at: task.created_at.timestamp_millis(),
            updated_at: task.updated_at.timestamp_millis(),
        });
    }
    Ok(ApiResponse::success(dtos))
}

async fn get_task(
    State(state): State<AppState>,
    Path(camera_id): Path<String>,
) -> Result<ApiResponse<TaskConfigDto>, ApiError> {
    let camera = db::CameraRepo::find_by_camera_id(&state.db, &camera_id).await?;
    let stream_mode = camera
        .as_ref()
        .map(|c| types::StreamMode::from_str_loose(&c.stream_mode));

    if let Some(task) = TaskRepo::find_by_camera_id(&state.db, &camera_id).await? {
        let instances = AlgorithmInstanceRepo::list_by_camera_id(&state.db, &camera_id).await?;
        Ok(ApiResponse::success(build_task_config_dto(
            &task,
            &instances,
            stream_mode,
        )))
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
            stream_mode,
            algorithm_instances: Some(Vec::new()),
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

    if !dto.camera_id.trim().is_empty() && dto.camera_id.trim() != cam_id {
        return Err(ApiError::BadRequest(
            "路径 cameraId 与请求体 cameraId 不一致".to_string(),
        ));
    }

    let mut camera = db::CameraRepo::find_by_camera_id(&state.db, &camera_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound(format!("关联摄像头未找到: {camera_id}")))?;

    // 若请求中指定了 stream_mode 且与当前摄像头不一致，则经由 CameraRepo 仓储原子更新
    if let Some(mode) = dto.stream_mode {
        if mode.as_str() != camera.stream_mode {
            if let Some(updated_cam) =
                db::CameraRepo::update_stream_mode(&state.db, &camera_id, mode.as_str())
                    .await
                    .map_err(|e| ApiError::Internal(e.to_string()))?
            {
                camera = updated_cam;
            }
        }
    }

    // 实例列表解析与合法性校验
    let instances = resolve_task_instances_for_save(&state, &camera_id, &dto).await?;

    let rules_json = serde_json::to_string(&dto.rules).unwrap_or_else(|_| "[]".to_string());
    let mg = dto.motion_gate.clone().unwrap_or_default();
    let mg_json = serde_json::to_string(&mg).unwrap_or_else(|_| "{}".to_string());

    // 1. 事务保存任务与算法实例集合
    let saved_task = TaskRepo::save_task_with_instances(
        &state.db,
        db::SaveTaskWithInstancesParams {
            camera_id: camera_id.clone(),
            name: dto.name.clone(),
            desired_enabled: dto.desired_enabled,
            rules_json,
            motion_gate_json: mg_json,
            status_message: None,
            instances: Some(instances),
        },
    )
    .await
    .map_err(|e| match e {
        db::DbError::Validation(msg) => ApiError::BadRequest(msg),
        db::DbError::Type(err) => ApiError::BadRequest(err.to_string()),
        db::DbError::NotFound { entity, key } => {
            ApiError::NotFound(format!("未找到{entity}: {key}"))
        }
        other => ApiError::Internal(other.to_string()),
    })?;

    let saved_instances = AlgorithmInstanceRepo::list_by_camera_id(&state.db, &camera_id).await?;

    // 2. 经由 Coordinator 同步更新空间几何规则
    state
        .task_coordinator
        .set_camera_rules(&camera_id, dto.rules.clone())
        .await;

    // 3. 编排运行时启停并同步实际状态
    let (_final_actual_status, _final_status_message) = if dto.desired_enabled {
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
        } else if saved_instances.is_empty() {
            let err_msg = "任务未绑定任何可运行的算法实例".to_string();
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
            let launch_instances: Vec<pipeline::InstanceLaunchConfig> = saved_instances
                .iter()
                .filter(|i| i.enabled)
                .map(|i| {
                    pipeline::InstanceLaunchConfig::from_persisted(
                        &i.algorithm_id,
                        &i.params_json,
                        i.analysis_fps,
                    )
                    .unwrap_or_else(|_| pipeline::InstanceLaunchConfig {
                        algorithm_id: i.algorithm_id.clone(),
                        algo_params: serde_json::json!({}),
                        target_fps: 10,
                    })
                })
                .collect();

            if launch_instances.is_empty() {
                let err_msg = "任务未启用任何算法实例".to_string();
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
                let params = task_service::build_start_params_async(
                    &camera_id,
                    &camera,
                    launch_instances,
                    dto.motion_gate.as_ref(),
                )
                .await;

                match state.task_coordinator.start_camera_pipeline(params).await {
                    Ok(_) => {
                        let instance_updates: Vec<db::TaskInstanceStateUpdate> = saved_instances
                            .iter()
                            .map(|inst| db::TaskInstanceStateUpdate {
                                instance_id: inst.instance_id.clone(),
                                actual_status: if inst.enabled {
                                    types::TaskStatus::Running
                                } else {
                                    types::TaskStatus::Stopped
                                },
                                status_message: if inst.enabled {
                                    "运行中".to_string()
                                } else {
                                    "已停用".to_string()
                                },
                            })
                            .collect();
                        if let Err(err) = TaskRepo::update_task_runtime_state(
                            &state.db,
                            camera_id.clone(),
                            types::TaskStatus::Running,
                            String::new(),
                            instance_updates,
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
                        let instance_updates: Vec<db::TaskInstanceStateUpdate> = saved_instances
                            .iter()
                            .map(|inst| db::TaskInstanceStateUpdate {
                                instance_id: inst.instance_id.clone(),
                                actual_status: types::TaskStatus::Error,
                                status_message: err_msg.clone(),
                            })
                            .collect();
                        let _ = TaskRepo::update_task_runtime_state(
                            &state.db,
                            camera_id.clone(),
                            types::TaskStatus::Error,
                            err_msg.clone(),
                            instance_updates,
                        )
                        .await;
                        state.pipeline.set_ai_active(&camera_id, false).await;
                        (types::TaskStatus::Error.as_i32(), err_msg)
                    }
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
                let instance_updates: Vec<db::TaskInstanceStateUpdate> = saved_instances
                    .iter()
                    .map(|inst| db::TaskInstanceStateUpdate {
                        instance_id: inst.instance_id.clone(),
                        actual_status: types::TaskStatus::Stopped,
                        status_message: String::new(),
                    })
                    .collect();
                TaskRepo::update_task_runtime_state(
                    &state.db,
                    camera_id.clone(),
                    types::TaskStatus::Stopped,
                    String::new(),
                    instance_updates,
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
                let instance_updates: Vec<db::TaskInstanceStateUpdate> = saved_instances
                    .iter()
                    .map(|inst| db::TaskInstanceStateUpdate {
                        instance_id: inst.instance_id.clone(),
                        actual_status: types::TaskStatus::Error,
                        status_message: err_msg.clone(),
                    })
                    .collect();
                let _ = TaskRepo::update_task_runtime_state(
                    &state.db,
                    camera_id.clone(),
                    types::TaskStatus::Error,
                    err_msg.clone(),
                    instance_updates,
                )
                .await;
                (types::TaskStatus::Error.as_i32(), err_msg)
            }
        }
    };

    let latest_task = TaskRepo::find_by_camera_id(&state.db, &camera_id)
        .await?
        .unwrap_or(saved_task);
    let latest_instances = AlgorithmInstanceRepo::list_by_camera_id(&state.db, &camera_id).await?;
    let final_stream_mode = types::StreamMode::from_str_loose(&camera.stream_mode);
    Ok(ApiResponse::success(build_task_config_dto(
        &latest_task,
        &latest_instances,
        Some(final_stream_mode),
    )))
}

async fn resolve_task_instances_for_save(
    state: &AppState,
    camera_id: &str,
    dto: &TaskConfigDto,
) -> Result<Vec<db::SaveTaskAlgorithmInstanceParams>, ApiError> {
    if let Some(raw_instances) = &dto.algorithm_instances {
        if raw_instances.is_empty() {
            return Ok(Vec::new());
        }

        let configs: Vec<types::TaskAlgorithmInstanceConfig> = raw_instances
            .iter()
            .map(|item| types::TaskAlgorithmInstanceConfig {
                instance_id: Some(item.instance_id.clone()).filter(|s| !s.is_empty()),
                algorithm_id: item.algorithm_id.clone(),
                analysis_fps: Some(item.analysis_fps),
                algo_params: Some(item.algo_params.clone()),
                enabled: item.enabled.or(Some(dto.desired_enabled)),
            })
            .collect();

        types::validate_task_algorithm_instances(&configs)
            .map_err(|e| ApiError::BadRequest(e.to_string()))?;

        for cfg in &configs {
            if db::AlgorithmRepo::find_by_algorithm_id(&state.db, &cfg.algorithm_id)
                .await?
                .is_none()
            {
                return Err(ApiError::NotFound(format!(
                    "未找到算法包: {}",
                    cfg.algorithm_id
                )));
            }
        }

        return Ok(configs
            .into_iter()
            .map(|cfg| {
                let analysis_fps = cfg.normalized_analysis_fps();
                let params_json = cfg
                    .normalized_algo_params_json()
                    .unwrap_or_else(|_| "{}".to_string());
                db::SaveTaskAlgorithmInstanceParams {
                    algorithm_id: cfg.algorithm_id,
                    analysis_fps,
                    params_json,
                    enabled: cfg.enabled,
                }
            })
            .collect());
    }

    if !dto.algorithm_id.trim().is_empty() {
        let target_algo_id = dto.algorithm_id.trim().to_string();
        if db::AlgorithmRepo::find_by_algorithm_id(&state.db, &target_algo_id)
            .await?
            .is_none()
        {
            return Err(ApiError::NotFound(format!(
                "未找到算法包: {target_algo_id}"
            )));
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

        return Ok(vec![db::SaveTaskAlgorithmInstanceParams {
            algorithm_id: target_algo_id,
            analysis_fps: dto.analysis_fps,
            params_json: algo_params_json,
            enabled: Some(dto.desired_enabled),
        }]);
    }

    if !dto.desired_enabled {
        let existing = AlgorithmInstanceRepo::list_by_camera_id(&state.db, camera_id).await?;
        return Ok(existing
            .into_iter()
            .map(|inst| db::SaveTaskAlgorithmInstanceParams {
                algorithm_id: inst.algorithm_id,
                analysis_fps: inst.analysis_fps,
                params_json: inst.params_json,
                enabled: Some(inst.enabled),
            })
            .collect());
    }

    let target_algo_id =
        task_service::resolve_algorithm_id(&state.db, &state.algo_registry, camera_id, "", true)
            .await?;
    let config = types::TaskAlgorithmInstanceConfig {
        instance_id: None,
        algorithm_id: target_algo_id,
        analysis_fps: Some(dto.analysis_fps),
        algo_params: Some(dto.algo_params.clone()),
        enabled: Some(dto.desired_enabled),
    };
    config
        .validate()
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let params_json = config
        .normalized_algo_params_json()
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    if params_json.len() > 64 * 1024 || params_json.contains('\0') {
        return Err(ApiError::BadRequest(
            "algoParams 序列化过大或包含非法字符".to_string(),
        ));
    }
    let analysis_fps = config.normalized_analysis_fps();
    let algorithm_id = config.algorithm_id;
    Ok(vec![db::SaveTaskAlgorithmInstanceParams {
        algorithm_id,
        analysis_fps,
        params_json,
        enabled: Some(dto.desired_enabled),
    }])
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
        .task_coordinator
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
    let config = types::TaskAlgorithmInstanceConfig {
        instance_id: None,
        algorithm_id: req.algorithm_id,
        analysis_fps: req.analysis_fps,
        algo_params: req.params,
        enabled: Some(req.enabled.unwrap_or(false)),
    };
    config
        .validate()
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let params_json = config
        .normalized_algo_params_json()
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let analysis_fps = config.normalized_analysis_fps();

    let created = TaskRepo::add_instance_to_task(
        &state.db,
        &req.camera_id,
        db::SaveTaskAlgorithmInstanceParams {
            algorithm_id: config.algorithm_id,
            analysis_fps,
            params_json,
            enabled: config.enabled,
        },
    )
    .await
    .map_err(|e| match e {
        db::DbError::Validation(msg) => ApiError::BadRequest(msg),
        db::DbError::Type(err) => ApiError::BadRequest(err.to_string()),
        db::DbError::NotFound { entity, key } => {
            ApiError::NotFound(format!("未找到{entity}: {key}"))
        }
        other => ApiError::Internal(other.to_string()),
    })?;

    Ok(ApiResponse::success(AlgorithmInstanceDto::from(created)))
}

async fn update_instance(
    State(state): State<AppState>,
    Path(instance_id): Path<String>,
    Json(req): Json<UpdateInstanceRequest>,
) -> Result<ApiResponse<AlgorithmInstanceDto>, ApiError> {
    if let Some(fps) = req.analysis_fps {
        if !(0..=60).contains(&fps) {
            return Err(ApiError::BadRequest(
                "analysisFps 必须在 0..=60 之间".to_string(),
            ));
        }
    }

    let params_json = match req.params {
        Some(value) => {
            if !value.is_object() {
                return Err(ApiError::BadRequest(
                    "params 必须为 JSON Object 对象".to_string(),
                ));
            }
            let json = serde_json::to_string(&value)
                .map_err(|err| ApiError::BadRequest(format!("params 序列化失败: {err}")))?;
            if json.len() > 64 * 1024 || json.contains('\0') {
                return Err(ApiError::BadRequest(
                    "params 序列化过大或包含非法字符".to_string(),
                ));
            }
            Some(json)
        }
        None => None,
    };
    let rules_json = req
        .rules
        .map(|value| serde_json::to_string(&value))
        .transpose()
        .map_err(|err| ApiError::BadRequest(format!("rules 序列化失败: {err}")))?;
    let motion_gate_json = req
        .motion_gate
        .map(|value| serde_json::to_string(&value))
        .transpose()
        .map_err(|err| ApiError::BadRequest(format!("motionGate 序列化失败: {err}")))?;

    let updated = TaskRepo::update_instance_and_sync_task(
        &state.db,
        &instance_id,
        db::UpdateTaskInstanceParams {
            analysis_fps: req.analysis_fps,
            params_json,
            rules_json,
            motion_gate_json,
            enabled: req.enabled,
        },
    )
    .await
    .map_err(|e| match e {
        db::DbError::Validation(msg) => ApiError::BadRequest(msg),
        db::DbError::Type(err) => ApiError::BadRequest(err.to_string()),
        db::DbError::NotFound { entity, key } => {
            ApiError::NotFound(format!("未找到{entity}: {key}"))
        }
        other => ApiError::Internal(other.to_string()),
    })?;

    let camera_id = updated.camera_id.clone();
    sync_pipeline_for_camera(&state, &camera_id).await?;

    Ok(ApiResponse::success(AlgorithmInstanceDto::from(updated)))
}

pub(crate) async fn sync_pipeline_for_camera(
    state: &AppState,
    camera_id: &str,
) -> Result<(), ApiError> {
    let Some(task) = TaskRepo::find_by_camera_id(&state.db, camera_id).await? else {
        let _ = state.task_coordinator.stop_camera_pipeline(camera_id).await;
        state.pipeline.set_ai_active(camera_id, false).await;
        state
            .task_coordinator
            .set_camera_rules(camera_id, Vec::new())
            .await;
        return Ok(());
    };

    let has_runtime = state.task_coordinator.has_active_runtime(camera_id).await;
    if !task.desired_enabled && !has_runtime {
        return Ok(());
    }

    let Some(camera) = db::CameraRepo::find_by_camera_id(&state.db, camera_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    else {
        let _ = state.task_coordinator.stop_camera_pipeline(camera_id).await;
        state.pipeline.set_ai_active(camera_id, false).await;
        state
            .task_coordinator
            .set_camera_rules(camera_id, Vec::new())
            .await;
        return Ok(());
    };

    let instances = AlgorithmInstanceRepo::list_by_camera_id(&state.db, camera_id).await?;
    let launch_instances: Vec<pipeline::InstanceLaunchConfig> = instances
        .iter()
        .filter(|i| i.enabled)
        .map(|i| {
            pipeline::InstanceLaunchConfig::from_persisted(
                &i.algorithm_id,
                &i.params_json,
                i.analysis_fps,
            )
            .unwrap_or_else(|_| pipeline::InstanceLaunchConfig {
                algorithm_id: i.algorithm_id.clone(),
                algo_params: serde_json::json!({}),
                target_fps: 10,
            })
        })
        .collect();

    if !task.desired_enabled || launch_instances.is_empty() {
        let _ = state.task_coordinator.stop_camera_pipeline(camera_id).await;
        state.pipeline.set_ai_active(camera_id, false).await;
        state
            .task_coordinator
            .set_camera_rules(camera_id, Vec::new())
            .await;
        let msg = if !task.desired_enabled {
            ""
        } else {
            "无已启用的算法实例"
        };
        let status = if !task.desired_enabled {
            types::TaskStatus::Stopped
        } else {
            types::TaskStatus::Error
        };
        TaskRepo::update_status(&state.db, camera_id, status.as_i32(), msg).await?;
    } else {
        let motion_gate = serde_json::from_str::<MotionGateConfig>(&task.motion_gate_json).ok();
        let params = task_service::build_start_params_async(
            camera_id,
            &camera,
            launch_instances,
            motion_gate.as_ref(),
        )
        .await;
        state
            .task_coordinator
            .start_camera_pipeline(params)
            .await
            .map_err(ApiError::Coordinator)?;
        state.pipeline.set_ai_active(camera_id, true).await;

        let rules = match DetectionRule::parse_rules_json(&task.rules_json) {
            Ok(r) => r,
            Err(err) => {
                tracing::warn!(
                    camera_id = %camera_id,
                    error = %err,
                    "任务空间布防规则解析失败，回退为空规则"
                );
                Vec::new()
            }
        };
        state
            .task_coordinator
            .set_camera_rules(camera_id, rules)
            .await;
    }
    Ok(())
}

async fn set_instance_enabled(
    State(state): State<AppState>,
    Path(instance_id): Path<String>,
    Json(req): Json<SetInstanceEnabledRequest>,
) -> Result<ApiResponse<()>, ApiError> {
    let instance = AlgorithmInstanceRepo::find_by_instance_id(&state.db, &instance_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("算法实例未找到: {instance_id}")))?;
    let camera_id = instance.camera_id;

    TaskRepo::set_instance_enabled_and_sync_task(&state.db, &instance_id, req.enabled)
        .await
        .map_err(|e| match e {
            db::DbError::NotFound { entity, key } => {
                ApiError::NotFound(format!("未找到{entity}: {key}"))
            }
            other => ApiError::Internal(other.to_string()),
        })?;

    sync_pipeline_for_camera(&state, &camera_id).await?;

    Ok(ApiResponse::success(()))
}

async fn delete_instance(
    State(state): State<AppState>,
    Path(instance_id): Path<String>,
) -> Result<ApiResponse<()>, ApiError> {
    let instance = AlgorithmInstanceRepo::find_by_instance_id(&state.db, &instance_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("算法实例未找到: {instance_id}")))?;
    let camera_id = instance.camera_id;

    let rows = TaskRepo::delete_instance_and_sync_task(&state.db, &instance_id)
        .await
        .map_err(|e| match e {
            db::DbError::NotFound { entity, key } => {
                ApiError::NotFound(format!("未找到{entity}: {key}"))
            }
            other => ApiError::Internal(other.to_string()),
        })?;
    if rows == 0 {
        return Err(ApiError::NotFound(format!("算法实例未找到: {instance_id}")));
    }

    sync_pipeline_for_camera(&state, &camera_id).await?;

    Ok(ApiResponse::success(()))
}

// -------------------------------------------------------------
// 测试
// -------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use pipeline::TaskRuntimeService;
    use sea_orm::Set;
    use std::collections::{HashMap, HashSet};
    use std::sync::{Arc, Mutex};
    use tower::ServiceExt;

    #[derive(Debug, Default)]
    struct MockTaskRuntimeService {
        active_params: Mutex<HashMap<String, pipeline::StartCameraPipelineParams>>,
        active_cameras: Mutex<HashSet<String>>,
        started: Mutex<Vec<pipeline::StartCameraPipelineParams>>,
        stopped: Mutex<Vec<String>>,
        camera_rules: Mutex<HashMap<String, Vec<types::DetectionRule>>>,
        should_fail: Mutex<Option<String>>,
        stop_should_fail: Mutex<Option<String>>,
    }

    impl MockTaskRuntimeService {
        fn new() -> Self {
            Self::default()
        }

        fn set_fail(&self, reason: impl Into<String>) {
            *self.should_fail.lock().unwrap() = Some(reason.into());
        }

        fn set_stop_fail(&self, reason: impl Into<String>) {
            *self.stop_should_fail.lock().unwrap() = Some(reason.into());
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

        fn camera_rules(&self, camera_id: &str) -> Option<Vec<types::DetectionRule>> {
            self.camera_rules.lock().unwrap().get(camera_id).cloned()
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
            self.camera_rules.lock().unwrap().remove(camera_id);
            self.stopped.lock().unwrap().push(camera_id.to_string());
            Ok(had_runtime)
        }

        async fn stop_all(&self) {
            self.active_params.lock().unwrap().clear();
            self.active_cameras.lock().unwrap().clear();
            self.camera_rules.lock().unwrap().clear();
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
                    instances: vec![pipeline::InstanceRuntimeInfo {
                        algorithm_id: "test_algo".to_string(),
                        target_fps: 10,
                    }],
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
                    instances: vec![pipeline::InstanceRuntimeInfo {
                        algorithm_id: "test_algo".to_string(),
                        target_fps: 10,
                    }],
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

        async fn set_camera_rules(&self, camera_id: &str, rules: Vec<types::DetectionRule>) {
            self.camera_rules
                .lock()
                .unwrap()
                .insert(camera_id.to_string(), rules);
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

    fn test_camera_model(camera_id: &str, rtsp_url: &str) -> db::entity::camera::ActiveModel {
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
        }
    }

    #[tokio::test]
    async fn test_task_crud_lifecycle() {
        let (app, state, token, _mock_coord) = setup_test_app().await;

        // 1. 创建关联摄像头
        let camera_model = test_camera_model("CAM-TASK-01", "rtsp://127.0.0.1:8554/live");
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
            "streamMode": "main",
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
        let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let update_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(update_json["data"]["streamMode"], "main");

        // 验证摄像头实体的 stream_mode 也被原子更新
        let updated_camera = db::CameraRepo::find_by_camera_id(&state.db, "CAM-TASK-01")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated_camera.stream_mode, "main");

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
        let camera_model = test_camera_model("CAM-ALGO-01", "rtsp://127.0.0.1:8554/live");
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
        let camera_model = test_camera_model("CAM-FAIL-01", "rtsp://127.0.0.1:8554/live");
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

        let camera_model = test_camera_model("CAM-STOP-01", "rtsp://127.0.0.1:8554/live");
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

        let camera_model = test_camera_model("CAM-IDEMP-01", "rtsp://127.0.0.1:8554/live");
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

        let camera_model = test_camera_model("CAM-DEL-01", "rtsp://127.0.0.1:8554/live");
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

        let camera_model = test_camera_model("CAM-EMPTY-URL", "");
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

    #[tokio::test]
    async fn test_task_multi_instance_api_full_flow() {
        let (app, state, token, _mock_coord) = setup_test_app().await;

        let camera_model = test_camera_model("CAM-MULTI-01", "rtsp://127.0.0.1:8554/live");
        db::CameraRepo::insert(&state.db, camera_model)
            .await
            .unwrap();

        for (algo_id, algo_name) in [
            ("general_detection", "通用检测"),
            ("face_recognition", "人脸识别"),
        ] {
            db::AlgorithmRepo::upsert_algorithm(
                &state.db,
                db::UpsertAlgorithmParams {
                    algorithm_id: algo_id.into(),
                    name: algo_name.into(),
                    algorithm_type: "detection".into(),
                    alarm_type_id: "intrusion".into(),
                    active_version: "1.0.0".into(),
                    description: "test".into(),
                    is_builtin: true,
                },
            )
            .await
            .unwrap();
        }

        // 2. PUT 创建多实例任务（含 null params 规范化）
        let put_body = serde_json::json!({
            "cameraId": "CAM-MULTI-01",
            "name": "多算法任务",
            "desiredEnabled": true,
            "rules": [],
            "algorithmInstances": [
                {
                    "algorithmId": "general_detection",
                    "analysisFps": 15,
                    "algoParams": { "confidence": 0.5 }
                },
                {
                    "algorithmId": "face_recognition",
                    "analysisFps": 5,
                    "algoParams": null
                }
            ]
        });
        let req = Request::builder()
            .uri("/api/v1/tasks/CAM-MULTI-01")
            .method("PUT")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_vec(&put_body).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let instances = json["data"]["algorithmInstances"].as_array().unwrap();
        assert_eq!(instances.len(), 2);
        assert_eq!(instances[1]["algoParams"], serde_json::json!({}));

        // 3. GET 验证持久化和 DTO 字段
        let req = Request::builder()
            .uri("/api/v1/tasks/CAM-MULTI-01")
            .method("GET")
            .header("Authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["data"]["name"], "多算法任务");
        assert_eq!(
            json["data"]["algorithmInstances"].as_array().unwrap().len(),
            2
        );

        // 4. PUT 重复算法应当返回 400
        let dup_body = serde_json::json!({
            "cameraId": "CAM-MULTI-01",
            "name": "重复算法任务",
            "desiredEnabled": true,
            "rules": [],
            "algorithmInstances": [
                { "algorithmId": "general_detection", "analysisFps": 10 },
                { "algorithmId": "general_detection", "analysisFps": 20 }
            ]
        });
        let req = Request::builder()
            .uri("/api/v1/tasks/CAM-MULTI-01")
            .method("PUT")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_vec(&dup_body).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        // 5. PUT 未知算法应当返回 404
        let unknown_body = serde_json::json!({
            "cameraId": "CAM-MULTI-01",
            "name": "未知算法任务",
            "desiredEnabled": true,
            "rules": [],
            "algorithmInstances": [
                { "algorithmId": "unknown_algorithm_xyz", "analysisFps": 10 }
            ]
        });
        let req = Request::builder()
            .uri("/api/v1/tasks/CAM-MULTI-01")
            .method("PUT")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_vec(&unknown_body).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        // 6. PUT cameraId 与 URL 路径不一致返回 400
        let mismatch_body = serde_json::json!({
            "cameraId": "CAM-OTHER-99",
            "name": "ID不匹配",
            "desiredEnabled": true,
            "rules": []
        });
        let req = Request::builder()
            .uri("/api/v1/tasks/CAM-MULTI-01")
            .method("PUT")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_vec(&mismatch_body).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        // 7. PUT 清空实例集合（显式 []）
        let clear_body = serde_json::json!({
            "cameraId": "CAM-MULTI-01",
            "name": "无实例任务",
            "desiredEnabled": false,
            "rules": [],
            "algorithmInstances": []
        });
        let req = Request::builder()
            .uri("/api/v1/tasks/CAM-MULTI-01")
            .method("PUT")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_vec(&clear_body).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            json["data"]["algorithmInstances"].as_array().unwrap().len(),
            0
        );

        // 8. 兼容接口 /api/v1/tasks/instances 创建并拦截重复
        let inst_req = serde_json::json!({
            "cameraId": "CAM-MULTI-01",
            "algorithmId": "general_detection",
            "analysisFps": 10
        });
        let req = Request::builder()
            .uri("/api/v1/tasks/instances")
            .method("POST")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_vec(&inst_req).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let instance_id = db::AlgorithmInstanceRepo::list_by_camera_id(&state.db, "CAM-MULTI-01")
            .await
            .unwrap()
            .into_iter()
            .find(|instance| instance.algorithm_id == "general_detection")
            .map(|instance| instance.instance_id)
            .expect("实例创建后应可查询到实例 ID");
        let update_instance_body = serde_json::json!({
            "analysisFps": 12,
            "params": {"confidence": 0.8},
            "rules": [],
            "motionGate": {"enabled": true, "threshold": 20},
            "enabled": true
        });
        let req = Request::builder()
            .uri(format!("/api/v1/tasks/instances/{instance_id}"))
            .method("PUT")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&update_instance_body).unwrap(),
            ))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["data"]["algorithmId"], "general_detection");
        assert_eq!(json["data"]["analysisFps"], 12);

        let updated_instance =
            db::AlgorithmInstanceRepo::find_by_instance_id(&state.db, &instance_id)
                .await
                .unwrap()
                .expect("更新后的实例应存在");
        assert_eq!(updated_instance.analysis_fps, 12);
        assert_eq!(updated_instance.params_json, r#"{"confidence":0.8}"#);
        assert_eq!(updated_instance.rules_json, "[]");
        assert_eq!(
            updated_instance.motion_gate_json,
            r#"{"enabled":true,"threshold":20}"#
        );

        // 再次创建相同算法实例应当被拦截并返回 400
        let req = Request::builder()
            .uri("/api/v1/tasks/instances")
            .method("POST")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_vec(&inst_req).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_task_instance_enable_and_rules_sync_pipeline_lifecycle() {
        let (app, state, token, mock_coord) = setup_test_app().await;

        let camera_model = test_camera_model("CAM-SYNC-01", "rtsp://127.0.0.1:8554/live");
        db::CameraRepo::insert(&state.db, camera_model)
            .await
            .unwrap();

        db::AlgorithmRepo::upsert_algorithm(
            &state.db,
            db::UpsertAlgorithmParams {
                algorithm_id: "general_detection".into(),
                name: "通用检测".into(),
                algorithm_type: "detection".into(),
                alarm_type_id: "intrusion".into(),
                active_version: "1.0.0".into(),
                description: "test".into(),
                is_builtin: true,
            },
        )
        .await
        .unwrap();

        // 1. 创建任务并配置 1 条绊线规则与 1 个启用实例
        let initial_rule = serde_json::json!({
            "role": "line",
            "lineDirection": "a_to_b",
            "points": [{"x": 0.0, "y": 0.5}, {"x": 1.0, "y": 0.5}]
        });
        let put_task_body = serde_json::json!({
            "cameraId": "CAM-SYNC-01",
            "name": "布防同步任务",
            "desiredEnabled": true,
            "rules": [initial_rule],
            "algorithmInstances": [
                {
                    "algorithmId": "general_detection",
                    "analysisFps": 15,
                    "algoParams": { "confidence": 0.5 }
                }
            ]
        });
        let req = Request::builder()
            .uri("/api/v1/tasks/CAM-SYNC-01")
            .method("PUT")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_vec(&put_task_body).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // 验证管线已启动，且规则已同步至 TaskRuntimeService
        assert!(mock_coord.has_active_runtime("CAM-SYNC-01").await);
        let rules = mock_coord
            .camera_rules("CAM-SYNC-01")
            .expect("rules present");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].role, types::DetectionRuleRole::Line);

        let instances = db::AlgorithmInstanceRepo::list_by_camera_id(&state.db, "CAM-SYNC-01")
            .await
            .unwrap();
        assert_eq!(instances.len(), 1);
        let instance_id = &instances[0].instance_id;

        // 2. 禁用实例 -> 管线由于无启用实例而停机，规则被清理
        let req = Request::builder()
            .uri(format!("/api/v1/tasks/instances/{instance_id}/enabled"))
            .method("PUT")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({"enabled": false})).unwrap(),
            ))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        assert!(!mock_coord.has_active_runtime("CAM-SYNC-01").await);
        let rules_after_disable = mock_coord.camera_rules("CAM-SYNC-01").unwrap_or_default();
        assert!(rules_after_disable.is_empty());

        // 3. 停机状态下通过 update_instance 重新启用实例 -> 必须自愈唤醒拉起管线并恢复布防规则
        let update_instance_body = serde_json::json!({
            "analysisFps": 20,
            "params": {"confidence": 0.8},
            "enabled": true
        });
        let req = Request::builder()
            .uri(format!("/api/v1/tasks/instances/{instance_id}"))
            .method("PUT")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&update_instance_body).unwrap(),
            ))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        assert!(
            mock_coord.has_active_runtime("CAM-SYNC-01").await,
            "启用实例后停机的任务必须被自愈拉起"
        );
        let rules_recovered = mock_coord
            .camera_rules("CAM-SYNC-01")
            .expect("rules recovered");
        assert_eq!(rules_recovered.len(), 1);
        assert_eq!(rules_recovered[0].role, types::DetectionRuleRole::Line);

        // 4. 更新实例级规则 -> 原子同步到任务并热更新到管线
        let new_polygon_rule = serde_json::json!({
            "role": "roi",
            "points": [
                {"x": 0.1, "y": 0.1},
                {"x": 0.9, "y": 0.1},
                {"x": 0.9, "y": 0.9},
                {"x": 0.1, "y": 0.9}
            ]
        });
        let update_rules_body = serde_json::json!({
            "rules": [new_polygon_rule],
            "enabled": true
        });
        let req = Request::builder()
            .uri(format!("/api/v1/tasks/instances/{instance_id}"))
            .method("PUT")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_vec(&update_rules_body).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let rules_updated = mock_coord
            .camera_rules("CAM-SYNC-01")
            .expect("rules updated");
        assert_eq!(rules_updated.len(), 1);
        assert_eq!(rules_updated[0].role, types::DetectionRuleRole::Roi);
        assert_eq!(rules_updated[0].points.len(), 4);
    }
}
