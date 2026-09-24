use axum::extract::{Path, State};
use axum::routing::{get, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use db::{AlgorithmInstanceRepo, TaskRepo};
use types::{DetectionRule, MotionGateConfig};

use crate::error::ApiError;
use crate::response::ApiResponse;
use crate::state::AppState;
use crate::task_service;

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
    pub desired_revision: i64,
    #[serde(default)]
    pub applied_revision: i64,
    #[serde(default)]
    pub apply_state: types::InstanceApplyState,
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
            desired_revision: m.desired_revision,
            applied_revision: m.applied_revision,
            apply_state: m.runtime_apply_state(),
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
    pub apply_state: types::InstanceApplyState,
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
            apply_state: m.runtime_apply_state(),
            status_message: m.status_message.clone(),
        }
    }
}

fn default_algo_params() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
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
    /// 任务配置版本号。
    ///
    /// 请求：客户端读取到的版本号，服务端不匹配时返回 409（整体下发的乐观并发控制）；
    /// 省略表示不做版本校验（新建任务等场景）。
    /// 响应：当前版本号，客户端必须用它更新本地快照。
    #[serde(default)]
    pub config_revision: Option<i64>,
}

/// 布防开关请求体：只表达「该通道的 AI 分析应该运行 / 停止」这一个期望。
///
/// `enabled` 刻意不给默认值：缺失字段不得被静默当成撤防，意图必须显式。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetTaskEnabledRequest {
    pub enabled: bool,
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
    /// 任务配置版本号：列表返回的快照版本，启停等整体下发时必须原样回传
    pub config_revision: i64,
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
        config_revision: Some(task.config_revision),
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_tasks))
        .route(
            "/{camera_id}",
            get(get_task).put(update_task).delete(delete_task),
        )
        // 布防开关：任务级状态动词的子资源，避免为翻转一个布尔值而重发整份配置
        .route("/{camera_id}/enabled", put(set_task_enabled))
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
            config_revision: task.config_revision,
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
            config_revision: None,
        }))
    }
}

async fn update_task(
    State(state): State<AppState>,
    Path(camera_id): Path<String>,
    Json(dto): Json<TaskConfigDto>,
) -> Result<ApiResponse<TaskConfigDto>, ApiError> {
    let camera_id = camera_id.trim();
    if camera_id.is_empty() || camera_id.len() > 128 || camera_id.contains('\0') {
        return Err(ApiError::BadRequest("cameraId 非法".to_string()));
    }

    if !dto.camera_id.trim().is_empty() && dto.camera_id.trim() != camera_id {
        return Err(ApiError::BadRequest(
            "路径 cameraId 与请求体 cameraId 不一致".to_string(),
        ));
    }

    let mut camera = db::CameraRepo::find_by_camera_id(&state.db, camera_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
        .ok_or_else(|| ApiError::NotFound(format!("关联摄像头未找到: {camera_id}")))?;

    // 若请求中指定了 stream_mode 且与当前摄像头不一致，则经由 CameraRepo 仓储原子更新
    if let Some(mode) = dto.stream_mode {
        if mode.as_str() != camera.stream_mode {
            if let Some(updated_cam) =
                db::CameraRepo::update_stream_mode(&state.db, camera_id, mode.as_str())
                    .await
                    .map_err(|e| ApiError::Internal(e.to_string()))?
            {
                camera = updated_cam;
            }
        }
    }

    // 实例列表解析与合法性校验
    let instances = resolve_task_instances_for_save(&state, camera_id, &dto).await?;

    let rules_json = serde_json::to_string(&dto.rules).unwrap_or_else(|_| "[]".to_string());
    let mg = dto.motion_gate.clone().unwrap_or_default();
    let mg_json = serde_json::to_string(&mg).unwrap_or_else(|_| "{}".to_string());

    // 1. 事务保存任务与算法实例集合
    let saved_task = TaskRepo::save_task_with_instances(
        &state.db,
        db::SaveTaskWithInstancesParams {
            camera_id: camera_id.to_string(),
            name: dto.name.clone(),
            desired_enabled: dto.desired_enabled,
            rules_json,
            motion_gate_json: mg_json,
            status_message: None,
            instances: Some(instances),
            expected_revision: dto.config_revision,
        },
    )
    .await
    .map_err(|e| match e {
        db::DbError::Validation(msg) => ApiError::BadRequest(msg),
        db::DbError::Type(err) => ApiError::BadRequest(err.to_string()),
        db::DbError::NotFound { entity, key } => {
            ApiError::NotFound(format!("未找到{entity}: {key}"))
        }
        db::DbError::RevisionConflict { expected, actual } => {
            ApiError::ConfigRevisionConflict { expected, actual }
        }
        other => ApiError::Internal(other.to_string()),
    })?;

    let saved_instances = AlgorithmInstanceRepo::list_by_camera_id(&state.db, camera_id).await?;

    // 2. 编排运行时启停并同步实际状态（与布防开关共用同一份编排逻辑；规则设置由其统一驱动）
    apply_persisted_runtime(
        &state,
        camera_id,
        &saved_task,
        Some(&camera),
        &saved_instances,
    )
    .await?;

    let latest_task = TaskRepo::find_by_camera_id(&state.db, camera_id)
        .await?
        .unwrap_or(saved_task);
    let latest_instances = AlgorithmInstanceRepo::list_by_camera_id(&state.db, camera_id).await?;
    let final_stream_mode = types::StreamMode::from_str_loose(&camera.stream_mode);
    Ok(ApiResponse::success(build_task_config_dto(
        &latest_task,
        &latest_instances,
        Some(final_stream_mode),
    )))
}

/// 布防开关（状态动词）：只提交「该通道的 AI 分析应该运行 / 停止」这一个期望。
///
/// 与整体下发的分工：名称、防区规则、运动门控与算法参数仍由 `PUT /tasks/{cameraId}` 维护，
/// 这里不携带也不改写它们——把「一个布尔值」的表达成本降到只剩一个布尔值，
/// 同时避免为翻转开关而回传一份可能已经过期的整份配置。
///
/// 但仍然共用同一个配置版本号：布防意图变化同样推进 `configRevision`，
/// 基于旧快照的整体下发会被 409 拒绝，而不是静默把布防状态改回去。
async fn set_task_enabled(
    State(state): State<AppState>,
    Path(camera_id): Path<String>,
    Json(req): Json<SetTaskEnabledRequest>,
) -> Result<ApiResponse<TaskConfigDto>, ApiError> {
    let camera_id = camera_id.trim();
    if camera_id.is_empty() || camera_id.len() > 128 || camera_id.contains('\0') {
        return Err(ApiError::BadRequest("cameraId 非法".to_string()));
    }

    // 布防前先确认存在可运行的媒体源：撤防在任何情况下都必须成功，
    // 但当摄像头不存在时「布防成功」只会在运行时立刻失败，不如直接把原因说清楚。
    let camera = db::CameraRepo::find_by_camera_id(&state.db, camera_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    if req.enabled && camera.is_none() {
        return Err(ApiError::NotFound(format!("关联摄像头未找到: {camera_id}")));
    }

    let saved_task = TaskRepo::set_task_enabled(&state.db, camera_id, req.enabled)
        .await
        .map_err(|e| match e {
            db::DbError::NotFound { .. } => {
                ApiError::NotFound(format!("任务未找到，请先创建任务: {camera_id}"))
            }
            other => ApiError::Internal(other.to_string()),
        })?;

    let instances = AlgorithmInstanceRepo::list_by_camera_id(&state.db, camera_id).await?;
    apply_persisted_runtime(&state, camera_id, &saved_task, camera.as_ref(), &instances).await?;

    let task = TaskRepo::find_by_camera_id(&state.db, camera_id)
        .await?
        .unwrap_or(saved_task);
    let latest_instances = AlgorithmInstanceRepo::list_by_camera_id(&state.db, camera_id).await?;
    let stream_mode = camera
        .as_ref()
        .map(|c| types::StreamMode::from_str_loose(&c.stream_mode));
    Ok(ApiResponse::success(build_task_config_dto(
        &task,
        &latest_instances,
        stream_mode,
    )))
}

/// 判定请求是否携带「单实例旧字段」形式的算法意图。
///
/// `analysisFps` / `algoParams` 的 serde 缺省值（`0` / `{}`）与显式提交等值无法区分，
/// 因此与缺省等值的取值一律视为「本次未表达算法意图」。
fn carries_legacy_single_instance_intent(dto: &TaskConfigDto) -> bool {
    if !dto.algorithm_id.trim().is_empty() {
        return true;
    }
    if dto.analysis_fps != 0 {
        return true;
    }
    match &dto.algo_params {
        serde_json::Value::Null => false,
        serde_json::Value::Object(map) => !map.is_empty(),
        // 非对象取值会由旧字段路径显式拒绝（400），这里按「有意图」放行以保留既有报错行为。
        _ => true,
    }
}

/// 把既有持久化实例原样映射为保存参数（参数 / 帧率 / 逐算法启停均不变）。
fn preserve_existing_instances(
    existing: Vec<db::entity::algorithm_instance::Model>,
) -> Vec<db::SaveTaskAlgorithmInstanceParams> {
    existing
        .into_iter()
        .map(|inst| db::SaveTaskAlgorithmInstanceParams {
            algorithm_id: inst.algorithm_id,
            analysis_fps: inst.analysis_fps,
            params_json: inst.params_json,
            enabled: Some(inst.enabled),
        })
        .collect()
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

    // 未携带实例集合、也未表达单实例旧字段意图的任务级保存（改名称 / 防区 / 门控 / 布防开关），
    // 一律原样保留既有实例：参数、帧率与逐算法启停同属「本次未提交」的字段，
    // 只有显式 `algorithmInstances: []` 才表示清空。
    // 该分支防止历史缺陷回归——单实例旧字段兜底分支会用缺省空参数覆盖实例，
    // 把用户配置的阈值静默清空为算法包默认值（实机数据：仅改防区的保存把
    // `detection_confidence_threshold: 0.5` 覆盖成 `{}`，实例静默回落到默认 0.25）。
    // 若当前为撤防（!dto.desired_enabled），即便未表达算法意图也原样保留既有实例。
    let should_preserve_instances = (dto.algorithm_instances.is_none()
        && !carries_legacy_single_instance_intent(dto))
        || !dto.desired_enabled;
    if should_preserve_instances {
        let existing = AlgorithmInstanceRepo::list_by_camera_id(&state.db, camera_id).await?;
        if !existing.is_empty() || !dto.desired_enabled {
            return Ok(preserve_existing_instances(existing));
        }
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
    let task = TaskRepo::find_by_camera_id(&state.db, &camera_id).await?;

    // 1. 严格先停止运行时与媒体订阅，确保解码器与分析泵安全回收
    let was_running = state
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

    if let Some(task) = task {
        if was_running {
            pipeline::op_log::record(types::OpEvent::TaskStopped { task_id: task.id });
        } else if let Some(event) = types::OpEvent::task_status_transition(
            task.id,
            task.actual_status,
            types::TaskStatus::Stopped,
            "",
        ) {
            pipeline::op_log::record(event);
        }
    }

    Ok(ApiResponse::success(()))
}

/// 解析收敛结果对应的期望代际。
///
/// 实例级接口携带真实代际；任务级保存路径只有「实例集合 diff」信息，代际记 0，
/// 此时以数据库中当前的期望代际为准——同步成功即代表运行时已追上该代际。
async fn resolve_outcome_revision(
    state: &AppState,
    outcome: &pipeline::InstanceApplyOutcome,
) -> Result<Option<i64>, ApiError> {
    if outcome.desired_revision > 0 {
        return Ok(Some(outcome.desired_revision));
    }
    Ok(
        AlgorithmInstanceRepo::find_by_instance_id(&state.db, &outcome.instance_id)
            .await?
            .map(|model| model.desired_revision),
    )
}

/// 两阶段配置提交的第二阶段：把运行时收敛结果回写数据库。
///
/// 只回写单实例自身的结果，绝不因为某个实例失败而改写其他实例的状态。
async fn persist_instance_outcome(
    state: &AppState,
    outcome: &pipeline::InstanceApplyOutcome,
) -> Result<(), ApiError> {
    if outcome.apply_state == types::InstanceApplyState::Failed {
        AlgorithmInstanceRepo::mark_apply_failed(
            &state.db,
            &outcome.instance_id,
            &outcome.status_message,
        )
        .await?;
        return Ok(());
    }

    let Some(revision) = resolve_outcome_revision(state, outcome).await? else {
        // 实例已被删除：收敛结果无处回写，属于正常竞态
        return Ok(());
    };

    if outcome.apply_state == types::InstanceApplyState::Applied {
        AlgorithmInstanceRepo::mark_apply_applied(&state.db, &outcome.instance_id, revision)
            .await?;
    } else {
        AlgorithmInstanceRepo::mark_apply_pending(
            &state.db,
            &outcome.instance_id,
            revision,
            &outcome.status_message,
        )
        .await?;
    }
    Ok(())
}

/// 批量回写实例集合收敛结果，并把未收敛实例的原因记录到日志
async fn persist_instance_outcomes(
    state: &AppState,
    outcomes: &[pipeline::InstanceApplyOutcome],
) -> Result<(), ApiError> {
    for outcome in outcomes {
        if outcome.apply_state != types::InstanceApplyState::Applied {
            tracing::warn!(
                instance_id = %outcome.instance_id,
                apply_state = %outcome.apply_state,
                mechanism = ?outcome.mechanism,
                "算法实例期望配置尚未在运行时生效"
            );
        }
        persist_instance_outcome(state, outcome).await?;
    }
    Ok(())
}

/// 由逐实例收敛结果推导任务级状态与实例状态回写。
///
/// `applyState` 与实例健康状态正交：仅当目标 Worker 确实用上了期望配置才算运行中，
/// 未收敛的实例必须写回真实原因，不能用任务级"启动成功"掩盖单实例失败。
///
/// `restarted` 为真表示刚刚整路重建成功，此时期望集合中的启用实例都已挂载，
/// 不需要逐实例收敛结果即可判定为运行中。
fn task_runtime_updates(
    saved_instances: &[db::entity::algorithm_instance::Model],
    outcomes: &[pipeline::InstanceApplyOutcome],
    restarted: bool,
) -> (types::TaskStatus, Vec<db::TaskInstanceStateUpdate>) {
    use std::collections::HashMap;

    let by_instance: HashMap<&str, &pipeline::InstanceApplyOutcome> = outcomes
        .iter()
        .map(|outcome| (outcome.instance_id.as_str(), outcome))
        .collect();

    let mut statuses = Vec::with_capacity(saved_instances.len());
    let updates = saved_instances
        .iter()
        .map(|inst| {
            let (status, message) = match by_instance.get(inst.instance_id.as_str()) {
                Some(outcome) => match outcome.apply_state {
                    types::InstanceApplyState::Applied if inst.enabled => {
                        (types::TaskStatus::Running, "运行中".to_string())
                    }
                    types::InstanceApplyState::Applied => {
                        (types::TaskStatus::Stopped, "已停用".to_string())
                    }
                    types::InstanceApplyState::Pending => {
                        (types::TaskStatus::Starting, outcome.status_message.clone())
                    }
                    types::InstanceApplyState::Failed => {
                        (types::TaskStatus::Error, outcome.status_message.clone())
                    }
                },
                None if inst.enabled && restarted => {
                    (types::TaskStatus::Running, "运行中".to_string())
                }
                None if inst.enabled => (
                    types::TaskStatus::Starting,
                    "等待运行时挂载该算法实例".to_string(),
                ),
                None => (types::TaskStatus::Stopped, "已停用".to_string()),
            };
            statuses.push(status);
            db::TaskInstanceStateUpdate {
                instance_id: inst.instance_id.clone(),
                actual_status: status,
                status_message: message,
            }
        })
        .collect();

    (
        types::aggregate_task_instance_status(true, &statuses),
        updates,
    )
}

/// 为一组算法实例生成统一的运行状态回写记录。
fn uniform_instance_updates(
    instances: &[db::entity::algorithm_instance::Model],
    status: types::TaskStatus,
    status_message: &str,
) -> Vec<db::TaskInstanceStateUpdate> {
    instances
        .iter()
        .map(|inst| db::TaskInstanceStateUpdate {
            instance_id: inst.instance_id.clone(),
            actual_status: status,
            status_message: status_message.to_string(),
        })
        .collect()
}

/// 任务未运行（未布防或无启用实例）时收敛待生效的期望配置。
///
/// 此时期望配置就是任务启动时会使用的那一份，不存在「未生效」的可重试状态，
/// 因此把仍在 pending/failed 的代际标记为已收敛，避免界面长期显示处理中。
/// 期望配置不可运行（无码流地址 / 无实例）时，运行时必须真正停下并清理布防规则，
/// 不能只在数据库里把任务标成 Error 而让解码器继续空转、实例长期停在 pending。
async fn stop_unrunnable_runtime(state: &AppState, camera_id: &str) -> Result<(), ApiError> {
    let _ = state.task_coordinator.stop_camera_pipeline(camera_id).await;
    state.pipeline.set_ai_active(camera_id, false).await;
    state
        .task_coordinator
        .set_camera_rules(camera_id, Vec::new())
        .await;
    converge_instance_revisions(state, camera_id).await
}

/// 任务未运行、实例未启用，或整路重建已完成时，把待生效的期望配置收敛为已生效。
///
/// 这三种情况下运行时确实会用上（或已经用上）期望配置，不存在「未生效」的可重试状态，
/// 因此把仍在 pending/failed 的代际标记为已收敛，避免界面长期显示处理中。
/// 仍持有旧配置与旧 Worker 的失败路径绝不能走这里，否则「已生效」会退化成「数据库写成功」。
async fn converge_instance_revisions(state: &AppState, camera_id: &str) -> Result<(), ApiError> {
    for model in AlgorithmInstanceRepo::list_by_camera_id(&state.db, camera_id).await? {
        if model.desired_revision == model.applied_revision {
            continue;
        }
        AlgorithmInstanceRepo::mark_apply_applied(
            &state.db,
            &model.instance_id,
            model.desired_revision,
        )
        .await?;
    }
    Ok(())
}

/// 依据已提交的期望配置编排运行时启停，并把真实运行状态回写数据库。
///
/// 任务级整体下发与布防开关两个入口都只负责把期望配置写入数据库，
/// 「这一次期望变化该启动、该保留还是该停机」必须从持久化配置统一推导：
/// 两个入口各写一份编排逻辑，迟早会分叉成「同一份期望配置因入口不同而得到不同的运行时行为」。
async fn apply_persisted_runtime(
    state: &AppState,
    camera_id: &str,
    task: &db::entity::task::Model,
    camera: Option<&db::entity::camera::Model>,
    instances: &[db::entity::algorithm_instance::Model],
) -> Result<(), ApiError> {
    if task.desired_enabled {
        let err_msg = match camera {
            None => Some("关联摄像头未找到".to_string()),
            Some(camera) if camera.rtsp_url.trim().is_empty() => {
                Some("摄像头主码流 RTSP 地址为空".to_string())
            }
            Some(_) if instances.is_empty() => Some("任务未绑定任何可运行的算法实例".to_string()),
            Some(_) => None,
        };

        if let Some(err_msg) = err_msg {
            TaskRepo::update_status(
                &state.db,
                camera_id,
                types::TaskStatus::Error.as_i32(),
                &err_msg,
            )
            .await?;
            if let Some(event) = types::OpEvent::task_status_transition(
                task.id,
                task.actual_status,
                types::TaskStatus::Error,
                &err_msg,
            ) {
                pipeline::op_log::record(event);
            }
            stop_unrunnable_runtime(state, camera_id).await?;
        } else {
            let camera = camera.expect("camera is guaranteed Some when err_msg is None");
            // 复用统一收敛入口：媒体契约与实例集合都从已提交的持久化配置解析，
            // 避免用请求体临时拼装出与实例级接口不一致的媒体签名；「没有任何启用实例」
            // 也由它统一停机、清理规则并收敛代际。
            match sync_pipeline_with_models(state, camera_id, task, camera, instances).await {
                Ok(Some(sync)) => {
                    if sync.restarted {
                        tracing::info!(
                            camera_id = %camera_id,
                            "媒体输入契约已变化，触发整路重建"
                        );
                    }
                    let (task_status, instance_updates) =
                        task_runtime_updates(instances, &sync.outcomes, sync.restarted);
                    if let Err(err) = TaskRepo::update_task_runtime_state(
                        &state.db,
                        camera_id.to_string(),
                        task_status,
                        String::new(),
                        instance_updates,
                    )
                    .await
                    {
                        if let Err(stop_err) =
                            state.task_coordinator.stop_camera_pipeline(camera_id).await
                        {
                            tracing::error!(
                                camera_id = %camera_id,
                                error = %stop_err,
                                "运行状态写入失败后回滚分析管线也失败"
                            );
                        }
                        state.pipeline.set_ai_active(camera_id, false).await;
                        return Err(ApiError::Db(err));
                    }
                    let reason = sync
                        .outcomes
                        .iter()
                        .find(|outcome| outcome.apply_state != types::InstanceApplyState::Applied)
                        .map(|outcome| outcome.status_message.as_str())
                        .unwrap_or_default();
                    if let Some(event) = types::OpEvent::task_status_transition(
                        task.id,
                        task.actual_status,
                        task_status,
                        reason,
                    ) {
                        pipeline::op_log::record(event);
                    }
                }
                Ok(None) => {
                    // 本次没有可收敛的运行时：期望配置与真实运行状态保持数据库中的原值
                }
                Err(e) => {
                    let err_msg = format!("启动分析管线失败: {e}");
                    tracing::warn!(
                        camera_id = %camera_id,
                        error = %err_msg,
                        "分析管线启动失败，保留期望状态并记录错误"
                    );
                    let instance_updates =
                        uniform_instance_updates(instances, types::TaskStatus::Error, &err_msg);
                    if TaskRepo::update_task_runtime_state(
                        &state.db,
                        camera_id.to_string(),
                        types::TaskStatus::Error,
                        err_msg.clone(),
                        instance_updates,
                    )
                    .await
                    .is_ok()
                    {
                        if let Some(event) = types::OpEvent::task_status_transition(
                            task.id,
                            task.actual_status,
                            types::TaskStatus::Error,
                            &err_msg,
                        ) {
                            pipeline::op_log::record(event);
                        }
                    }
                    state.pipeline.set_ai_active(camera_id, false).await;
                }
            }
        }
    } else {
        // 停用分析管线
        match state.task_coordinator.stop_camera_pipeline(camera_id).await {
            Ok(was_running) => {
                state.pipeline.set_ai_active(camera_id, false).await;
                state
                    .task_coordinator
                    .set_camera_rules(camera_id, Vec::new())
                    .await;
                let instance_updates =
                    uniform_instance_updates(instances, types::TaskStatus::Stopped, "");
                TaskRepo::update_task_runtime_state(
                    &state.db,
                    camera_id.to_string(),
                    types::TaskStatus::Stopped,
                    String::new(),
                    instance_updates,
                )
                .await?;
                if was_running {
                    pipeline::op_log::record(types::OpEvent::TaskStopped { task_id: task.id });
                } else if let Some(event) = types::OpEvent::task_status_transition(
                    task.id,
                    task.actual_status,
                    types::TaskStatus::Stopped,
                    "",
                ) {
                    pipeline::op_log::record(event);
                }
                // 无运行时时期望配置就是下次启动要用的那一份，必须立即收敛代际；
                // 否则实例会长期停在 pending，无法区分「排队中」与「已生效」。
                converge_instance_revisions(state, camera_id).await?;
            }
            Err(e) => {
                let err_msg = format!("停止分析管线失败: {e}");
                tracing::warn!(
                    camera_id = %camera_id,
                    error = %err_msg,
                    "分析管线停止失败，保留期望状态并记录错误"
                );
                let instance_updates =
                    uniform_instance_updates(instances, types::TaskStatus::Error, &err_msg);
                if TaskRepo::update_task_runtime_state(
                    &state.db,
                    camera_id.to_string(),
                    types::TaskStatus::Error,
                    err_msg.clone(),
                    instance_updates,
                )
                .await
                .is_ok()
                {
                    if let Some(event) = types::OpEvent::task_status_transition(
                        task.id,
                        task.actual_status,
                        types::TaskStatus::Error,
                        &err_msg,
                    ) {
                        pipeline::op_log::record(event);
                    }
                }
            }
        }
    }

    Ok(())
}

/// 单一收敛入口：以数据库中的期望配置为准，把该摄像头的算法实例集合收敛到运行时。
///
/// 返回 `None` 表示本次没有可收敛的运行时（无任务、无摄像头、任务未布防或没有已启用实例），
/// 返回 `Some` 则携带逐实例收敛结果，供调用方回写状态与展示未生效原因。
///
/// 所有写接口都必须走这里：媒体契约与实例集合都从持久化配置解析，
/// 避免不同入口用请求体临时拼装出不一致的媒体签名，导致本应增量的变更触发整路重建。
pub(crate) async fn sync_pipeline_for_camera(
    state: &AppState,
    camera_id: &str,
) -> Result<Option<pipeline::CameraInstanceSyncOutcome>, ApiError> {
    let Some(task) = TaskRepo::find_by_camera_id(&state.db, camera_id).await? else {
        stop_unrunnable_runtime(state, camera_id).await?;
        return Ok(None);
    };

    let Some(camera) = db::CameraRepo::find_by_camera_id(&state.db, camera_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    else {
        stop_unrunnable_runtime(state, camera_id).await?;
        return Ok(None);
    };

    let instances = AlgorithmInstanceRepo::list_by_camera_id(&state.db, camera_id).await?;
    sync_pipeline_with_models(state, camera_id, &task, &camera, &instances).await
}

/// 依据已加载的领域模型收敛分析管线，避免在同一写请求中重复读库。
async fn sync_pipeline_with_models(
    state: &AppState,
    camera_id: &str,
    task: &db::entity::task::Model,
    camera: &db::entity::camera::Model,
    instances: &[db::entity::algorithm_instance::Model],
) -> Result<Option<pipeline::CameraInstanceSyncOutcome>, ApiError> {
    let has_runtime = state.task_coordinator.has_active_runtime(camera_id).await;
    if !task.desired_enabled && !has_runtime {
        converge_instance_revisions(state, camera_id).await?;
        return Ok(None);
    }

    let launch_instances: Vec<pipeline::InstanceLaunchConfig> = instances
        .iter()
        .filter(|i| i.enabled)
        .map(|i| {
            pipeline::InstanceLaunchConfig::from_persisted(
                &i.instance_id,
                &i.algorithm_id,
                &i.params_json,
                i.analysis_fps,
            )
            .unwrap_or_else(|_| pipeline::InstanceLaunchConfig {
                instance_id: i.instance_id.clone(),
                algorithm_id: i.algorithm_id.clone(),
                algo_params: serde_json::json!({}),
                target_fps: 10,
            })
        })
        .collect();

    if !task.desired_enabled || launch_instances.is_empty() {
        stop_unrunnable_runtime(state, camera_id).await?;
        let (status, msg) = if !task.desired_enabled {
            (types::TaskStatus::Stopped, "")
        } else {
            (types::TaskStatus::Error, "无已启用的算法实例")
        };
        TaskRepo::update_status(&state.db, camera_id, status.as_i32(), msg).await?;
        if let Some(event) =
            types::OpEvent::task_status_transition(task.id, task.actual_status, status, msg)
        {
            pipeline::op_log::record(event);
        }
        return Ok(None);
    }

    let motion_gate = serde_json::from_str::<MotionGateConfig>(&task.motion_gate_json).ok();
    let params = task_service::build_start_params_async(
        camera_id,
        camera,
        launch_instances,
        motion_gate.as_ref(),
    )
    .await;
    // 媒体输入契约未变化时只增量收敛实例集合：增删算法、改阈值、改抽帧频率
    // 都不会重启解码器与其他实例的 Worker。
    let sync = state
        .task_coordinator
        .sync_camera_instances(params)
        .await
        .map_err(ApiError::Coordinator)?;
    if sync.restarted {
        tracing::info!(camera_id = %camera_id, "媒体输入契约已变化，分析管线整路重建");
    }
    persist_instance_outcomes(state, &sync.outcomes).await?;
    if sync.restarted {
        // 整路重建成功后，期望集合中的实例都按期望配置完成了挂载，代际随之收敛；
        // 重建路径不产生逐实例 outcomes，若不在此收敛，实例会长期停在 pending。
        converge_instance_revisions(state, camera_id).await?;
    }
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
    Ok(Some(sync))
}

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
        apply_failure: Mutex<Option<String>>,
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

        /// 让实例集合收敛返回 failed，用于验证两阶段提交的失败回写
        fn set_instance_apply_failure(&self, reason: impl Into<String>) {
            *self.apply_failure.lock().unwrap() = Some(reason.into());
        }

        /// 清除收敛失败开关，模拟硬件资源恢复
        fn set_instance_apply_failure_to_none(&self) {
            *self.apply_failure.lock().unwrap() = None;
        }

        /// 读回运行时实例快照的抽帧频率（增量生效的运行时证据）
        fn active_instance_fps(&self, camera_id: &str, algorithm_id: &str) -> Option<u32> {
            self.active_params
                .lock()
                .unwrap()
                .get(camera_id)?
                .instances
                .iter()
                .find(|inst| inst.algorithm_id == algorithm_id)
                .map(|inst| inst.target_fps)
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

        async fn sync_camera_instances(
            &self,
            params: pipeline::StartCameraPipelineParams,
        ) -> Result<pipeline::CameraInstanceSyncOutcome, pipeline::CoordinatorError> {
            let camera_id = params.camera_id.clone();
            let media_changed = self
                .active_params
                .lock()
                .unwrap()
                .get(&camera_id)
                .map(|active| active.media_signature() != params.media_signature())
                .unwrap_or(true);
            if media_changed {
                let generation = self.start_camera_pipeline(params).await?;
                assert_eq!(generation, 1);
                return Ok(pipeline::CameraInstanceSyncOutcome {
                    camera_id,
                    restarted: true,
                    outcomes: Vec::new(),
                });
            }

            // 媒体契约未变化：只更新实例集合快照，逐实例上报收敛结果
            let failure = self.apply_failure.lock().unwrap().clone();
            let outcomes = {
                let mut active_params = self.active_params.lock().unwrap();
                let active = active_params
                    .get_mut(&camera_id)
                    .expect("增量收敛要求存在活跃运行时");
                *active = params.clone();
                params
                    .instances
                    .iter()
                    .map(|inst| pipeline::InstanceApplyOutcome {
                        instance_id: inst.instance_id.clone(),
                        desired_revision: 0,
                        applied_revision: failure.is_none().then_some(0),
                        apply_state: match &failure {
                            Some(_) => types::InstanceApplyState::Failed,
                            None => types::InstanceApplyState::Applied,
                        },
                        mechanism: pipeline::InstanceApplyMechanism::Noop,
                        status_message: failure.clone().unwrap_or_default(),
                    })
                    .collect()
            };
            Ok(pipeline::CameraInstanceSyncOutcome {
                camera_id,
                restarted: false,
                outcomes,
            })
        }

        async fn apply_instance_config(
            &self,
            desired: pipeline::InstanceDesiredConfig,
        ) -> Result<pipeline::InstanceApplyOutcome, pipeline::CoordinatorError> {
            let failure = self.apply_failure.lock().unwrap().clone();
            Ok(pipeline::InstanceApplyOutcome {
                instance_id: desired.instance_id,
                desired_revision: desired.desired_revision,
                applied_revision: failure.is_none().then_some(desired.desired_revision),
                apply_state: match &failure {
                    Some(_) => types::InstanceApplyState::Failed,
                    None => types::InstanceApplyState::Applied,
                },
                mechanism: pipeline::InstanceApplyMechanism::Noop,
                status_message: failure.unwrap_or_default(),
            })
        }

        async fn remove_instance_runtime(
            &self,
            _camera_id: &str,
            _instance_id: &str,
        ) -> Result<bool, pipeline::CoordinatorError> {
            Ok(false)
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
                        instance_id: "test_algo".to_string(),
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
                        instance_id: "test_algo".to_string(),
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

    /// 任务级保存是唯一写路径：把整份任务配置（含全部算法实例）下发一次。
    async fn put_task_config(
        app: &Router,
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
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    /// 布防开关（状态动词）：只提交期望的布防状态，不携带任何配置字段
    async fn put_task_enabled(
        app: &Router,
        token: &str,
        camera_id: &str,
        enabled: bool,
    ) -> (StatusCode, serde_json::Value) {
        let req = Request::builder()
            .uri(format!("/api/v1/tasks/{camera_id}/enabled"))
            .method("PUT")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from(
                serde_json::json!({ "enabled": enabled }).to_string(),
            ))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, json)
    }

    /// 读取任务配置（含逐实例 applyState / desiredRevision / appliedRevision）
    async fn get_task_config(app: &Router, token: &str, camera_id: &str) -> serde_json::Value {
        let req = Request::builder()
            .uri(format!("/api/v1/tasks/{camera_id}"))
            .method("GET")
            .header("Authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
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

        // 变更 FPS 只应在帧边界重设抽帧频率：不得回收运行时，也不得整路重建解码器。
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
        assert_eq!(
            mock_coord.started_params().len(),
            1,
            "抽帧频率变更必须走增量路径，不得整路重建"
        );
        assert!(
            !mock_coord
                .stopped_cameras()
                .contains(&"CAM-IDEMP-01".to_string()),
            "抽帧频率变更不得回收正在运行的分析运行时"
        );
        assert_eq!(
            mock_coord.active_instance_fps("CAM-IDEMP-01", "idemp_algo"),
            Some(20),
            "新的抽帧频率必须已提交到运行时实例快照"
        );
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
        let (app, state, token, mock_coord) = setup_test_app().await;

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

        // 8. 重新布防多实例任务：整体下发是唯一写路径，未变实例不触发任何重建
        let mut rearm = put_body.clone();
        rearm["name"] = serde_json::json!("重新布防多算法任务");
        let (status, json) = put_task_config(&app, &token, "CAM-MULTI-01", &rearm).await;
        assert_eq!(status, StatusCode::OK);
        let instances = json["data"]["algorithmInstances"].as_array().unwrap();
        assert_eq!(instances.len(), 2);
        for inst in instances {
            assert_eq!(inst["applyState"], "applied");
            assert_eq!(
                inst["desiredRevision"], inst["appliedRevision"],
                "已收敛实例的期望代际必须等于已生效代际"
            );
        }
        assert_eq!(
            mock_coord.started_params().len(),
            2,
            "从停机状态恢复布防需要启动一次；未变更的实例集合不得触发第二次启动"
        );

        // 9. 只改一个实例的参数：整体下发，其他实例保持原配置且不重建
        let mut tweaked = rearm.clone();
        tweaked["algorithmInstances"][0]["analysisFps"] = serde_json::json!(12);
        tweaked["algorithmInstances"][0]["algoParams"] = serde_json::json!({"confidence": 0.8});
        let (status, json) = put_task_config(&app, &token, "CAM-MULTI-01", &tweaked).await;
        assert_eq!(status, StatusCode::OK);
        let instances = json["data"]["algorithmInstances"].as_array().unwrap();
        let general = instances
            .iter()
            .find(|i| i["algorithmId"] == "general_detection")
            .expect("通用检测实例应存在");
        assert_eq!(general["analysisFps"], 12);
        assert_eq!(general["algoParams"]["confidence"], 0.8);
        assert_eq!(general["applyState"], "applied");
        let face = instances
            .iter()
            .find(|i| i["algorithmId"] == "face_recognition")
            .expect("人脸识别实例应存在");
        assert_eq!(face["analysisFps"], 5, "未变更实例必须保持原配置");
        assert_eq!(face["applyState"], "applied");
        assert_eq!(
            mock_coord.started_params().len(),
            2,
            "改单个实例参数不得重启解码器或其他实例"
        );
        assert_eq!(
            mock_coord.active_instance_fps("CAM-MULTI-01", "general_detection"),
            Some(12),
            "整体下发后目标实例的新抽帧频率必须在运行时生效"
        );

        let general_row = db::AlgorithmInstanceRepo::list_by_camera_id(&state.db, "CAM-MULTI-01")
            .await
            .unwrap()
            .into_iter()
            .find(|instance| instance.algorithm_id == "general_detection")
            .expect("通用检测实例应存在");
        assert_eq!(general_row.analysis_fps, 12);
        assert_eq!(general_row.params_json, r#"{"confidence":0.8}"#);
        assert_eq!(general_row.desired_revision, general_row.applied_revision);
    }

    /// 运行时未能让目标 Worker 用上期望配置时，接口必须如实返回 failed 并落库原因，
    /// 不得因为「数据库写成功」就显示为已应用。
    #[tokio::test]
    async fn test_instance_apply_failure_is_reported_and_persisted() {
        let (app, state, token, mock_coord) = setup_test_app().await;

        let camera_model = test_camera_model("CAM-FAIL-01", "rtsp://127.0.0.1:8554/live");
        db::CameraRepo::insert(&state.db, camera_model)
            .await
            .unwrap();

        db::AlgorithmRepo::upsert_algorithm(
            &state.db,
            db::UpsertAlgorithmParams {
                algorithm_id: "fail_algo".into(),
                name: "失败算法".into(),
                algorithm_type: "detection".into(),
                alarm_type_id: "intrusion".into(),
                active_version: "1.0.0".into(),
                description: "test".into(),
                is_builtin: true,
            },
        )
        .await
        .unwrap();

        // 1. 布防任务，实例进入运行中
        let put_body = serde_json::json!({
            "cameraId": "CAM-FAIL-01",
            "name": "失败回写任务",
            "desiredEnabled": true,
            "rules": [],
            "algorithmInstances": [
                { "algorithmId": "fail_algo", "analysisFps": 10, "algoParams": { "confidence": 0.5 } }
            ]
        });
        let req = Request::builder()
            .uri("/api/v1/tasks/CAM-FAIL-01")
            .method("PUT")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_vec(&put_body).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let instance_id = db::AlgorithmInstanceRepo::list_by_camera_id(&state.db, "CAM-FAIL-01")
            .await
            .unwrap()
            .first()
            .map(|inst| inst.instance_id.clone())
            .expect("实例应存在");

        // 2. 让运行时收敛失败（模拟 NPU 资源不足等硬件侧拒绝）
        mock_coord.set_instance_apply_failure("创建推理 Worker 失败: 设备内存不足");

        // 任务级整体下发：只改目标实例参数，其余配置保持原样
        let mut failing = put_body.clone();
        failing["algorithmInstances"][0]["analysisFps"] = serde_json::json!(20);
        failing["algorithmInstances"][0]["algoParams"] = serde_json::json!({"confidence": 0.9});
        let (status, json) = put_task_config(&app, &token, "CAM-FAIL-01", &failing).await;
        assert_eq!(status, StatusCode::OK);

        let instance = json["data"]["algorithmInstances"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["algorithmId"] == "fail_algo")
            .expect("失败算法实例应在响应中");
        assert_eq!(
            instance["applyState"], "failed",
            "运行时未生效时不得因为数据库写入成功而显示为已应用"
        );
        assert_eq!(instance["analysisFps"], 20);
        assert!(
            instance["statusMessage"]
                .as_str()
                .is_some_and(|msg| msg.contains("内存不足")),
            "失败必须带上可读原因，实际: {}",
            instance["statusMessage"]
        );
        assert_ne!(
            instance["appliedRevision"], instance["desiredRevision"],
            "未生效的期望代际不得被记为已生效"
        );

        // 3. 数据库必须保留期望配置并记录失败状态，供重启后重试
        let persisted = db::AlgorithmInstanceRepo::find_by_instance_id(&state.db, &instance_id)
            .await
            .unwrap()
            .expect("实例应仍存在");
        assert_eq!(persisted.analysis_fps, 20);
        assert_eq!(persisted.params_json, r#"{"confidence":0.9}"#);
        assert_eq!(
            persisted.runtime_apply_state(),
            types::InstanceApplyState::Failed
        );
        assert!(persisted.desired_revision > persisted.applied_revision);
        assert!(persisted.status_message.contains("内存不足"));

        // 4. 任务查询接口必须返回同样的失败信息
        let task_json = get_task_config(&app, &token, "CAM-FAIL-01").await;
        let instance = task_json["data"]["algorithmInstances"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["algorithmId"] == "fail_algo")
            .expect("失败算法实例应在任务响应中");
        assert_eq!(instance["applyState"], "failed");
        assert_eq!(instance["desiredRevision"], persisted.desired_revision);
        assert_eq!(instance["appliedRevision"], persisted.applied_revision);

        // 5. 运行时恢复后重新下发同一份配置：期望代际追上即收敛为 applied
        mock_coord.set_instance_apply_failure_to_none();
        let (status, json) = put_task_config(&app, &token, "CAM-FAIL-01", &failing).await;
        assert_eq!(status, StatusCode::OK);
        let instance = json["data"]["algorithmInstances"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["algorithmId"] == "fail_algo")
            .expect("失败算法实例应在响应中");
        assert_eq!(
            instance["applyState"], "applied",
            "运行时恢复后必须回到已应用状态"
        );
        assert_eq!(instance["desiredRevision"], instance["appliedRevision"]);
        let recovered = db::AlgorithmInstanceRepo::find_by_instance_id(&state.db, &instance_id)
            .await
            .unwrap()
            .expect("实例应仍存在");
        assert_eq!(
            recovered.runtime_apply_state(),
            types::InstanceApplyState::Applied
        );
        assert!(
            !recovered.status_message.contains("内存不足"),
            "收敛成功后必须清理过期失败原因，实际: {}",
            recovered.status_message
        );
    }

    /// 保存即生效：改一个实例的参数后，期望代际必须收敛为已生效，不能长期停留在 pending。
    #[tokio::test]
    async fn test_task_save_converges_instance_revision() {
        let (app, state, token, mock_coord) = setup_test_app().await;

        let camera_model = test_camera_model("CAM-REV-01", "rtsp://127.0.0.1:8554/live");
        db::CameraRepo::insert(&state.db, camera_model)
            .await
            .unwrap();

        db::AlgorithmRepo::upsert_algorithm(
            &state.db,
            db::UpsertAlgorithmParams {
                algorithm_id: "rev_algo".into(),
                name: "代际算法".into(),
                algorithm_type: "detection".into(),
                alarm_type_id: "intrusion".into(),
                active_version: "1.0.0".into(),
                description: "test".into(),
                is_builtin: true,
            },
        )
        .await
        .unwrap();

        let put_body = serde_json::json!({
            "cameraId": "CAM-REV-01",
            "name": "代际收敛任务",
            "desiredEnabled": true,
            "rules": [],
            "algorithmInstances": [
                { "algorithmId": "rev_algo", "analysisFps": 10, "algoParams": { "confidence": 0.5 } }
            ]
        });
        let (status, _) = put_task_config(&app, &token, "CAM-REV-01", &put_body).await;
        assert_eq!(status, StatusCode::OK);

        let instance_id = db::AlgorithmInstanceRepo::list_by_camera_id(&state.db, "CAM-REV-01")
            .await
            .unwrap()
            .first()
            .map(|inst| inst.instance_id.clone())
            .expect("实例应存在");

        // 改目标实例参数：期望代际 +1，运行时已生效 -> 必须收敛为 applied
        let mut update_body = put_body.clone();
        update_body["algorithmInstances"][0]["analysisFps"] = serde_json::json!(12);
        update_body["algorithmInstances"][0]["algoParams"] =
            serde_json::json!({ "confidence": 0.7 });
        let (status, json) = put_task_config(&app, &token, "CAM-REV-01", &update_body).await;
        assert_eq!(status, StatusCode::OK);

        let instance = &json["data"]["algorithmInstances"][0];
        assert_eq!(instance["applyState"], "applied");
        assert_eq!(instance["desiredRevision"], 1);
        assert_eq!(
            instance["appliedRevision"], 1,
            "运行中实例的期望代际必须被回写为已生效"
        );
        assert_eq!(
            instance["analysisFps"], 12,
            "实例级变更必须立即生效到运行时快照"
        );
        assert!(
            instance["statusMessage"]
                .as_str()
                .is_some_and(|msg| !msg.contains("未")),
            "已收敛的实例不得携带未生效原因，实际: {}",
            instance["statusMessage"]
        );
        assert_eq!(
            mock_coord.active_instance_fps("CAM-REV-01", "rev_algo"),
            Some(12),
            "抽帧频率必须在运行时快照中生效，且不重建 Worker"
        );
        assert_eq!(
            mock_coord.started_params().len(),
            1,
            "改参数不得重启分析管线"
        );

        let persisted = db::AlgorithmInstanceRepo::find_by_instance_id(&state.db, &instance_id)
            .await
            .unwrap()
            .expect("实例应存在");
        assert_eq!(persisted.desired_revision, 1);
        assert_eq!(persisted.applied_revision, 1);
        assert_eq!(
            persisted.runtime_apply_state(),
            types::InstanceApplyState::Applied
        );

        // 再次下发完全相同的配置：不递增代际，也不产生新的管线重建
        let (status, json) = put_task_config(&app, &token, "CAM-REV-01", &update_body).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["algorithmInstances"][0]["desiredRevision"], 1);
        assert_eq!(
            json["data"]["algorithmInstances"][0]["applyState"],
            "applied"
        );
        assert_eq!(
            mock_coord.started_params().len(),
            1,
            "重发未变更的配置不得重启分析管线"
        );
    }

    /// 布防开关是状态动词：只翻转一个布尔值，不得顺带回写名称、防区与算法参数，
    /// 也不得为「只想布防」这一个意图重新下发整份配置。
    #[tokio::test]
    async fn test_enabled_verb_arms_and_disarms_without_resending_config() {
        let (app, state, token, mock_coord) = setup_test_app().await;

        let camera_model = test_camera_model("CAM-VERB-01", "rtsp://127.0.0.1:8554/live");
        db::CameraRepo::insert(&state.db, camera_model)
            .await
            .unwrap();

        db::AlgorithmRepo::upsert_algorithm(
            &state.db,
            db::UpsertAlgorithmParams {
                algorithm_id: "verb_algo".into(),
                name: "状态动词算法".into(),
                algorithm_type: "detection".into(),
                alarm_type_id: "intrusion".into(),
                active_version: "1.0.0".into(),
                description: "test".into(),
                is_builtin: true,
            },
        )
        .await
        .unwrap();

        // 先整体下发一份带防区与参数的启用配置
        let put_body = serde_json::json!({
            "cameraId": "CAM-VERB-01",
            "name": "状态动词任务",
            "desiredEnabled": true,
            "rules": [{ "role": "line", "points": [{ "x": 0.1, "y": 0.5 }, { "x": 0.9, "y": 0.5 }] }],
            "algorithmInstances": [
                { "algorithmId": "verb_algo", "analysisFps": 12, "algoParams": { "confidence": 0.42 } }
            ]
        });
        let (status, json) = put_task_config(&app, &token, "CAM-VERB-01", &put_body).await;
        assert_eq!(status, StatusCode::OK);
        let revision_after_config = json["data"]["configRevision"].as_i64().unwrap();
        assert_eq!(mock_coord.started_params().len(), 1);

        // 撤防：只发一个布尔值，其余配置由服务端从持久化配置读取
        let (status, json) = put_task_enabled(&app, &token, "CAM-VERB-01", false).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["desiredEnabled"], false);
        assert_eq!(json["data"]["actualStatus"], 0);
        assert_eq!(
            json["data"]["configRevision"].as_i64().unwrap(),
            revision_after_config + 1,
            "布防意图同样是配置，必须推进版本号，让基于旧快照的整体下发被拒绝"
        );
        // 配置本体不被改写
        assert_eq!(json["data"]["name"], "状态动词任务");
        assert_eq!(json["data"]["rules"].as_array().unwrap().len(), 1);
        let instance = &json["data"]["algorithmInstances"][0];
        assert_eq!(instance["analysisFps"], 12);
        assert_eq!(instance["algoParams"]["confidence"], 0.42);
        assert_eq!(
            instance["enabled"], true,
            "状态动词不得连坐改写分闸意图，否则再次布防时就没有可运行的实例了"
        );
        assert!(!mock_coord.has_active_runtime("CAM-VERB-01").await);
        assert!(mock_coord
            .camera_rules("CAM-VERB-01")
            .unwrap_or_default()
            .is_empty());
        let instances = db::AlgorithmInstanceRepo::list_by_camera_id(&state.db, "CAM-VERB-01")
            .await
            .unwrap();
        assert_eq!(
            instances[0].actual_status,
            types::TaskStatus::Stopped.as_i32(),
            "撤防必须把实例状态写回停机，而不是停留在运行中"
        );
        assert_eq!(instances[0].params_json, r#"{"confidence":0.42}"#);

        // 再布防：只发一个布尔值，运行时与防区按持久化配置重新装上
        let (status, json) = put_task_enabled(&app, &token, "CAM-VERB-01", true).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["desiredEnabled"], true);
        assert_eq!(json["data"]["actualStatus"], 2);
        assert_eq!(
            json["data"]["configRevision"].as_i64().unwrap(),
            revision_after_config + 2
        );
        assert_eq!(
            mock_coord.started_params().len(),
            2,
            "布防必须真正启动分析管线"
        );
        assert!(mock_coord.has_active_runtime("CAM-VERB-01").await);
        assert_eq!(
            mock_coord.camera_rules("CAM-VERB-01").unwrap().len(),
            1,
            "布防必须把已持久化的防区规则装上"
        );
        assert_eq!(
            json["data"]["algorithmInstances"][0]["applyState"],
            "applied"
        );
    }

    /// 状态动词不得隐式创建任务，也不得为缺失的摄像头制造「布防成功」的假象。
    #[tokio::test]
    async fn test_enabled_verb_requires_existing_task_and_camera() {
        let (app, state, token, _mock_coord) = setup_test_app().await;

        // 摄像头存在但没有任务：状态动词不负责创建
        let camera_model = test_camera_model("CAM-VERB-02", "rtsp://127.0.0.1:8554/live");
        db::CameraRepo::insert(&state.db, camera_model)
            .await
            .unwrap();
        let (status, json) = put_task_enabled(&app, &token, "CAM-VERB-02", true).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(json["code"], 40401);
        assert!(
            db::TaskRepo::find_by_camera_id(&state.db, "CAM-VERB-02")
                .await
                .unwrap()
                .is_none(),
            "状态动词不得凭空创建任务"
        );

        // 摄像头不存在：布防直接失败，撤防在无任务时同样返回未找到
        let (status, _) = put_task_enabled(&app, &token, "CAM-VERB-404", true).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = put_task_enabled(&app, &token, "CAM-VERB-404", false).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    /// 缺失 enabled 字段不得被默认成撤防：意图必须显式声明。
    #[tokio::test]
    async fn test_enabled_verb_rejects_missing_intent() {
        let (app, state, token, _mock_coord) = setup_test_app().await;

        let camera_model = test_camera_model("CAM-VERB-03", "rtsp://127.0.0.1:8554/live");
        db::CameraRepo::insert(&state.db, camera_model)
            .await
            .unwrap();

        let req = Request::builder()
            .uri("/api/v1/tasks/CAM-VERB-03/enabled")
            .method("PUT")
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .body(Body::from("{}"))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    /// 回归：仅改防区/门控的任务级保存不得改写算法实例参数。
    ///
    /// 实机缺陷（RK3568，2026-09-16）：一次省略 `algorithmInstances` 的保存被单实例
    /// 旧字段兜底分支接管，实例参数被覆盖为缺省 `{}`，用户配置的
    /// `detection_confidence_threshold: 0.5` 静默回落到算法包默认 0.25，
    /// 现场表现为「低于阈值却仍有检测结果」。
    #[tokio::test]
    async fn test_partial_save_without_instances_preserves_instance_params() {
        let (app, state, token, _mock_coord) = setup_test_app().await;

        let camera_model = test_camera_model("CAM-KEEP-01", "rtsp://127.0.0.1:8554/live");
        db::CameraRepo::insert(&state.db, camera_model)
            .await
            .unwrap();

        db::AlgorithmRepo::upsert_algorithm(
            &state.db,
            db::UpsertAlgorithmParams {
                algorithm_id: "keep_algo".into(),
                name: "参数保留算法".into(),
                algorithm_type: "recognition".into(),
                alarm_type_id: "intrusion".into(),
                active_version: "1.0.0".into(),
                description: "test".into(),
                is_builtin: true,
            },
        )
        .await
        .unwrap();

        // 1. 整体下发：实例携带用户阈值参数
        let full_body = serde_json::json!({
            "cameraId": "CAM-KEEP-01",
            "name": "参数保留任务",
            "desiredEnabled": true,
            "rules": [],
            "algorithmInstances": [
                {
                    "algorithmId": "keep_algo",
                    "analysisFps": 10,
                    "algoParams": { "detection_confidence_threshold": 0.5, "similarity_threshold": 0.75 }
                }
            ]
        });
        let (status, _) = put_task_config(&app, &token, "CAM-KEEP-01", &full_body).await;
        assert_eq!(status, StatusCode::OK);

        let before = db::AlgorithmInstanceRepo::list_by_camera_id(&state.db, "CAM-KEEP-01")
            .await
            .unwrap();
        assert_eq!(before.len(), 1);

        // 2. 仅改防区与门控：刻意省略 algorithmInstances / algorithmId / analysisFps / algoParams
        let partial_body = serde_json::json!({
            "cameraId": "CAM-KEEP-01",
            "name": "参数保留任务",
            "desiredEnabled": true,
            "rules": [{ "role": "line", "points": [{ "x": 0.1, "y": 0.5 }, { "x": 0.9, "y": 0.5 }] }],
            "motionGate": {
                "enabled": true, "threshold": 25, "contourArea": 100,
                "keepaliveIntervalMs": 2000, "motionHoldFrames": 10
            }
        });
        let (status, json) = put_task_config(&app, &token, "CAM-KEEP-01", &partial_body).await;
        assert_eq!(status, StatusCode::OK);

        // 3. 实例参数 / 帧率必须原样保留，且未触发代际推进与逐算法启停改写
        let instance = &json["data"]["algorithmInstances"][0];
        assert_eq!(
            instance["algoParams"]["detection_confidence_threshold"],
            0.5
        );
        assert_eq!(instance["algoParams"]["similarity_threshold"], 0.75);
        assert_eq!(instance["analysisFps"], 10);

        let after = db::AlgorithmInstanceRepo::list_by_camera_id(&state.db, "CAM-KEEP-01")
            .await
            .unwrap();
        assert_eq!(after.len(), 1, "部分字段保存不得增删实例");
        assert_eq!(after[0].params_json, before[0].params_json);
        assert_eq!(after[0].analysis_fps, before[0].analysis_fps);
        assert_eq!(
            after[0].desired_revision, before[0].desired_revision,
            "未提交算法变更的保存不得推进实例配置代际"
        );
        assert_eq!(
            after[0].enabled, before[0].enabled,
            "总闸保存不得连坐改写逐算法分闸"
        );
        // 防区作为任务级镜像仍同步到实例行
        assert!(after[0].rules_json.contains("line"));
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

        // 2. 禁用实例（整体下发，条目保留但 enabled=false）-> 管线因无启用实例停机，规则被清理
        let mut disable_body = put_task_body.clone();
        disable_body["algorithmInstances"][0]["enabled"] = serde_json::json!(false);
        let (status, json) = put_task_config(&app, &token, "CAM-SYNC-01", &disable_body).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["algorithmInstances"][0]["enabled"], false);
        assert_eq!(
            json["data"]["algorithmInstances"][0]["applyState"], "applied",
            "停用实例后不存在可重试的未生效状态，代际必须收敛"
        );

        assert!(!mock_coord.has_active_runtime("CAM-SYNC-01").await);
        let rules_after_disable = mock_coord.camera_rules("CAM-SYNC-01").unwrap_or_default();
        assert!(rules_after_disable.is_empty());

        // 3. 停机状态下重新启用实例并改参数 -> 自愈唤醒拉起管线并恢复布防规则
        let mut rearm_body = put_task_body.clone();
        rearm_body["algorithmInstances"][0]["enabled"] = serde_json::json!(true);
        rearm_body["algorithmInstances"][0]["analysisFps"] = serde_json::json!(20);
        rearm_body["algorithmInstances"][0]["algoParams"] =
            serde_json::json!({ "confidence": 0.8 });
        let (status, json) = put_task_config(&app, &token, "CAM-SYNC-01", &rearm_body).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            json["data"]["algorithmInstances"][0]["applyState"],
            "applied"
        );

        assert!(
            mock_coord.has_active_runtime("CAM-SYNC-01").await,
            "启用实例后停机的任务必须被自愈拉起"
        );
        assert_eq!(
            mock_coord.active_instance_fps("CAM-SYNC-01", "general_detection"),
            Some(20)
        );
        let revision_after_rearm = json["data"]["algorithmInstances"][0]["desiredRevision"]
            .as_i64()
            .unwrap();
        let rules_recovered = mock_coord
            .camera_rules("CAM-SYNC-01")
            .expect("rules recovered");
        assert_eq!(rules_recovered.len(), 1);
        assert_eq!(rules_recovered[0].role, types::DetectionRuleRole::Line);

        // 4. 任务级规则是唯一来源：改规则整体下发，直接热更新到运行中的管线
        let new_polygon_rule = serde_json::json!({
            "role": "roi",
            "points": [
                {"x": 0.1, "y": 0.1},
                {"x": 0.9, "y": 0.1},
                {"x": 0.9, "y": 0.9},
                {"x": 0.1, "y": 0.9}
            ]
        });
        let mut rules_body = rearm_body.clone();
        rules_body["rules"] = serde_json::json!([new_polygon_rule]);
        let (status, json) = put_task_config(&app, &token, "CAM-SYNC-01", &rules_body).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            json["data"]["algorithmInstances"][0]["applyState"], "applied",
            "仅规则变化不得让实例偏离已应用状态"
        );
        assert_eq!(
            json["data"]["algorithmInstances"][0]["desiredRevision"], revision_after_rearm,
            "仅规则变化不构成需要运行时重建的实例配置变更"
        );

        let rules_updated = mock_coord
            .camera_rules("CAM-SYNC-01")
            .expect("rules updated");
        assert_eq!(rules_updated.len(), 1);
        assert_eq!(rules_updated[0].role, types::DetectionRuleRole::Roi);
        assert_eq!(rules_updated[0].points.len(), 4);
        assert_eq!(rules_updated[0].points.len(), 4);
    }
}
