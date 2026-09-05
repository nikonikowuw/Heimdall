use std::path::{Path, PathBuf};

use axum::extract::{Multipart, Path as AxumPath, Query, State};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use db::{AlgorithmRepo, AlgorithmStats, OplogRepo, UpsertAlgorithmParams, UpsertVersionParams};
use infer::{
    compute_dir_size, current_platform_id, AlgoManifest, AlgoSandbox, InferError,
    ALGO_MANIFEST_FILENAME, DEFAULT_ALGO_PACKAGES_DIR,
};
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::middleware::AuthUser;
use crate::response::ApiResponse;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListAlgorithmsQuery {
    pub page: Option<u64>,
    pub page_size: Option<u64>,
    pub keyword: Option<String>,
    pub algorithm_type: Option<String>,
    pub is_builtin: Option<bool>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlgorithmVersionItemDto {
    pub id: i64,
    pub algorithm_id: String,
    pub version: String,
    pub platform_id: String,
    pub min_adapter_version: String,
    pub package_root: String,
    pub fps_tiers: serde_json::Value,
    pub config_schema: serde_json::Value,
    pub manifest_raw: serde_json::Value,
    pub package_size_bytes: i64,
    pub is_active: bool,
    pub is_builtin: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<db::entity::algorithm_version::Model> for AlgorithmVersionItemDto {
    fn from(m: db::entity::algorithm_version::Model) -> Self {
        let fps_tiers =
            serde_json::from_str(&m.fps_tiers).unwrap_or_else(|_| serde_json::json!([]));
        let config_schema =
            serde_json::from_str(&m.config_schema).unwrap_or_else(|_| serde_json::json!({}));
        let manifest_raw =
            serde_json::from_str(&m.manifest_raw).unwrap_or_else(|_| serde_json::json!({}));

        Self {
            id: m.id,
            algorithm_id: m.algorithm_id,
            version: m.version,
            platform_id: m.platform_id,
            min_adapter_version: m.min_adapter_version,
            package_root: m.package_root,
            fps_tiers,
            config_schema,
            manifest_raw,
            package_size_bytes: m.package_size_bytes,
            is_active: m.is_active,
            is_builtin: m.is_builtin,
            created_at: m.created_at.timestamp_millis(),
            updated_at: m.updated_at.timestamp_millis(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlgorithmItemDto {
    pub id: i64,
    pub algorithm_id: String,
    pub name: String,
    pub algorithm_type: String,
    pub alarm_type_id: String,
    pub active_version: String,
    pub description: String,
    pub is_builtin: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub versions: Vec<AlgorithmVersionItemDto>,
}

impl AlgorithmItemDto {
    fn from_model(m: db::entity::algorithm::Model, versions: Vec<AlgorithmVersionItemDto>) -> Self {
        Self {
            id: m.id,
            algorithm_id: m.algorithm_id,
            name: m.name,
            algorithm_type: m.algorithm_type,
            alarm_type_id: m.alarm_type_id,
            active_version: m.active_version,
            description: m.description,
            is_builtin: m.is_builtin,
            created_at: m.created_at.timestamp_millis(),
            updated_at: m.updated_at.timestamp_millis(),
            versions,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaginatedAlgorithmsDto {
    pub items: Vec<AlgorithmItemDto>,
    pub total: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadVersionInfo {
    pub algorithm_id: String,
    pub version: String,
    pub platform_id: String,
    pub package_root: String,
    pub is_active: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxCheckResultDto {
    pub passed: bool,
    pub steps_total: usize,
    pub steps_passed: usize,
    pub steps: Vec<String>,
    pub error_message: Option<String>,
    pub version: Option<UploadVersionInfo>,
    pub manifest: Option<AlgoManifest>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyPackageRequest {
    pub package_path: Option<String>,
}

fn get_standard_steps() -> Vec<String> {
    vec![
        "1. 路径防穿透与目录结构检查".to_string(),
        "2. SHA256 完整性与安全指纹校验".to_string(),
        "3. 解析 Manifest 与平台拓扑匹配".to_string(),
        "4. Config Schema 参数格式校验".to_string(),
        "5. 派生隔离子进程与超时守护".to_string(),
        "6. 算法库 C ABI 导出符号核对".to_string(),
        "7. 真实前向推理自测与内存复核".to_string(),
    ]
}

fn parse_failed_step_index(err: &InferError) -> usize {
    if let InferError::SandboxValidation { step, .. } = err {
        if let Some(first_char) = step.chars().next() {
            if let Some(digit) = first_char.to_digit(10) {
                return (digit as usize).saturating_sub(1);
            }
        }
    }
    3
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_algorithms))
        .route("/stats", get(get_stats))
        .route("/upload", post(upload_package))
        .route("/{id}", get(get_algorithm))
        .route("/{id}/versions", get(list_versions))
        .route("/{id}/versions/{version}/activate", put(activate_version))
        .route("/{id}/versions/{version}", delete(uninstall_version))
        // 兼容已有端点
        .route("/packages", get(legacy_list_packages))
        .route("/verify", post(verify_package))
        .route("/scan", post(scan_packages))
}

/// 解析 ID 参数：可能是数字主键，也可能是字符串 algorithm_id
async fn resolve_algorithm_id(
    db: &db::DatabaseConnection,
    param: &str,
) -> Result<String, ApiError> {
    if let Ok(num_id) = param.parse::<i64>() {
        if let Some(m) = AlgorithmRepo::find_by_id(db, num_id).await? {
            return Ok(m.algorithm_id);
        }
    }
    if let Some(m) = AlgorithmRepo::find_by_algorithm_id(db, param).await? {
        return Ok(m.algorithm_id);
    }
    Err(ApiError::NotFound(format!("未找到指定的算法: {param}")))
}

/// 分页查询算法列表（包含版本树）
async fn list_algorithms(
    State(state): State<AppState>,
    _user: AuthUser,
    Query(q): Query<ListAlgorithmsQuery>,
) -> Result<ApiResponse<PaginatedAlgorithmsDto>, ApiError> {
    let page = q.page.unwrap_or(1);
    let page_size = q.page_size.unwrap_or(20);

    let (algos, total) = AlgorithmRepo::list_algorithms(
        &state.db,
        page,
        page_size,
        q.keyword.as_deref(),
        q.algorithm_type.as_deref(),
        q.is_builtin,
    )
    .await?;

    let mut items = Vec::with_capacity(algos.len());
    for a in algos {
        let versions = load_version_dtos(&state.db, &a.algorithm_id).await?;
        items.push(AlgorithmItemDto::from_model(a, versions));
    }

    Ok(ApiResponse::success(PaginatedAlgorithmsDto {
        items,
        total,
    }))
}

/// 获取全局算法与算力统计指标
async fn get_stats(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<ApiResponse<AlgorithmStats>, ApiError> {
    let stats = AlgorithmRepo::stats(&state.db).await?;
    Ok(ApiResponse::success(stats))
}

/// 查询单个算法详情
async fn get_algorithm(
    State(state): State<AppState>,
    _user: AuthUser,
    AxumPath(id): AxumPath<String>,
) -> Result<ApiResponse<AlgorithmItemDto>, ApiError> {
    let aid = resolve_algorithm_id(&state.db, &id).await?;
    let algo = AlgorithmRepo::find_by_algorithm_id(&state.db, &aid)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("未找到算法: {aid}")))?;

    let versions = load_version_dtos(&state.db, &aid).await?;
    Ok(ApiResponse::success(AlgorithmItemDto::from_model(
        algo, versions,
    )))
}

/// 查询指定算法的版本列表
async fn list_versions(
    State(state): State<AppState>,
    _user: AuthUser,
    AxumPath(id): AxumPath<String>,
) -> Result<ApiResponse<Vec<AlgorithmVersionItemDto>>, ApiError> {
    let aid = resolve_algorithm_id(&state.db, &id).await?;
    let versions = load_version_dtos(&state.db, &aid).await?;
    Ok(ApiResponse::success(versions))
}

/// 从数据库加载指定算法的所有版本并转换为 DTO
async fn load_version_dtos(
    db: &db::DatabaseConnection,
    algorithm_id: &str,
) -> Result<Vec<AlgorithmVersionItemDto>, ApiError> {
    let raw = AlgorithmRepo::list_versions_by_algorithm_id(db, algorithm_id).await?;
    Ok(raw.into_iter().map(AlgorithmVersionItemDto::from).collect())
}

#[derive(Debug)]
pub struct ProcessedUploadResult {
    pub manifest: AlgoManifest,
    pub target_dir_str: String,
    pub pkg_size: i64,
    pub config_schema_json: String,
    pub manifest_raw_json: String,
    pub fps_tiers_json: String,
}

#[derive(Debug)]
pub struct ProcessedUploadError {
    pub failed_idx: usize,
    pub message: String,
    pub manifest: Option<AlgoManifest>,
}

/// RAII 临时目录守卫，Drop 时自动清理
struct TempDirGuard(PathBuf);

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 同步执行归档解压、沙箱进程驱动、文件复制与资源配置提取 (供 spawn_blocking 调度)
pub fn process_uploaded_package_archive_sync(
    archive_bytes: Vec<u8>,
    upload_filename: Option<String>,
) -> Result<ProcessedUploadResult, Box<ProcessedUploadError>> {
    let temp_dir =
        std::env::temp_dir().join(format!("argus_pkg_{}", uuid::Uuid::new_v4().simple()));
    if let Err(e) = std::fs::create_dir_all(&temp_dir) {
        return Err(Box::new(ProcessedUploadError {
            failed_idx: 0,
            message: format!("创建临时解压目录失败: {e}"),
            manifest: None,
        }));
    }
    let _temp_guard = TempDirGuard(temp_dir.clone());

    let src_pkg_dir =
        match extract_archive_package(&archive_bytes, upload_filename.as_deref(), &temp_dir) {
            Ok(dir) => dir,
            Err(err) => {
                return Err(Box::new(ProcessedUploadError {
                    failed_idx: 0,
                    message: err,
                    manifest: None,
                }));
            }
        };

    let manifest_path = src_pkg_dir.join(ALGO_MANIFEST_FILENAME);
    let manifest_bytes = match std::fs::read(&manifest_path) {
        Ok(b) => b,
        Err(e) => {
            return Err(Box::new(ProcessedUploadError {
                failed_idx: 0,
                message: format!("读取 {ALGO_MANIFEST_FILENAME} 失败: {e}"),
                manifest: None,
            }));
        }
    };
    let manifest: AlgoManifest = match serde_json::from_slice(&manifest_bytes) {
        Ok(m) => m,
        Err(e) => {
            return Err(Box::new(ProcessedUploadError {
                failed_idx: 0,
                message: format!("解析 {ALGO_MANIFEST_FILENAME} 格式失败: {e}"),
                manifest: None,
            }));
        }
    };

    let validated_manifest = match AlgoSandbox::validate_package(&src_pkg_dir, false) {
        Ok(m) => m,
        Err(e) => {
            let failed_idx = parse_failed_step_index(&e);
            return Err(Box::new(ProcessedUploadError {
                failed_idx,
                message: e.to_string(),
                manifest: Some(manifest),
            }));
        }
    };

    let target_dir = PathBuf::from("var/packages")
        .join(&validated_manifest.algorithm_id)
        .join(&validated_manifest.version);

    if target_dir.exists() {
        let _ = std::fs::remove_dir_all(&target_dir);
    }
    if let Some(parent) = target_dir.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let copy_res = copy_dir_all(&src_pkg_dir, &target_dir);
    // _temp_guard drops here (or at function exit) to clean up temp_dir

    if let Err(e) = copy_res {
        return Err(Box::new(ProcessedUploadError {
            failed_idx: 0,
            message: format!("安装算法包失败: {e}"),
            manifest: Some(validated_manifest),
        }));
    }

    let pkg_size = compute_dir_size(&target_dir);
    let schema_path = target_dir.join("config.schema.json");
    let config_schema_json = if schema_path.is_file() {
        std::fs::read_to_string(&schema_path).unwrap_or_else(|_| "{}".to_string())
    } else {
        "{}".to_string()
    };

    let manifest_raw_json = std::fs::read_to_string(target_dir.join(ALGO_MANIFEST_FILENAME))
        .unwrap_or_else(|_| "{}".to_string());
    let manifest_val: serde_json::Value =
        serde_json::from_str(&manifest_raw_json).unwrap_or_default();
    let fps_tiers_json = manifest_val
        .get("resource_profile")
        .and_then(|rp| rp.get("fps_tiers"))
        .map(|ft| ft.to_string())
        .unwrap_or_else(|| "[]".to_string());

    let target_dir_str = target_dir.to_string_lossy().to_string();

    Ok(ProcessedUploadResult {
        manifest: validated_manifest,
        target_dir_str,
        pkg_size,
        config_schema_json,
        manifest_raw_json,
        fps_tiers_json,
    })
}

/// 上传 .tar.gz / .zip / .tar 算法包并在物理沙箱中自检、落盘至 var/packages、入库并热加载
async fn upload_package(
    State(state): State<AppState>,
    user: AuthUser,
    mut multipart: Multipart,
) -> Result<ApiResponse<SandboxCheckResultDto>, ApiError> {
    let steps = get_standard_steps();
    let mut archive_bytes: Option<Vec<u8>> = None;
    let mut upload_filename: Option<String> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("解析上传表单失败: {e}")))?
    {
        let name = field.name().unwrap_or_default().to_string();
        if name == "file" || name == "package" {
            if let Some(fname) = field.file_name() {
                upload_filename = Some(fname.to_string());
            }
            let data = field
                .bytes()
                .await
                .map_err(|e| ApiError::BadRequest(format!("读取文件流失败: {e}")))?;
            archive_bytes = Some(data.to_vec());
            break;
        }
    }

    let bytes = match archive_bytes {
        Some(b) if !b.is_empty() => b,
        _ => return Err(ApiError::BadRequest("未找到有效的算法包文件流".to_string())),
    };

    // 核心 CPU 密集工作通过 spawn_blocking 卸载出 Tokio Worker 线程
    let process_res = tokio::task::spawn_blocking(move || {
        process_uploaded_package_archive_sync(bytes, upload_filename)
    })
    .await
    .map_err(|e| ApiError::Internal(format!("执行沙箱解包自检任务异常: {e}")))?;

    let processed = match process_res {
        Ok(res) => res,
        Err(boxed_err) => {
            return Ok(ApiResponse::success(SandboxCheckResultDto {
                passed: false,
                steps_total: 7,
                steps_passed: boxed_err.failed_idx,
                steps,
                error_message: Some(boxed_err.message),
                version: None,
                manifest: boxed_err.manifest,
            }));
        }
    };

    let validated_manifest = processed.manifest;
    let target_dir_str = processed.target_dir_str;

    // 写入 algorithms
    AlgorithmRepo::upsert_algorithm(
        &state.db,
        UpsertAlgorithmParams {
            algorithm_id: validated_manifest.algorithm_id.clone(),
            name: validated_manifest.name.clone(),
            algorithm_type: validated_manifest.algorithm_type.clone(),
            alarm_type_id: validated_manifest.alarm_type_id.clone(),
            active_version: validated_manifest.version.clone(),
            description: validated_manifest.description.clone().unwrap_or_default(),
            is_builtin: false,
        },
    )
    .await?;

    // 写入 algorithm_versions
    AlgorithmRepo::upsert_version(
        &state.db,
        UpsertVersionParams {
            algorithm_id: validated_manifest.algorithm_id.clone(),
            version: validated_manifest.version.clone(),
            platform_id: validated_manifest.platform_id.clone(),
            min_adapter_version: validated_manifest
                .min_adapter_version
                .clone()
                .unwrap_or_default(),
            package_root: target_dir_str.clone(),
            fps_tiers: processed.fps_tiers_json,
            config_schema: processed.config_schema_json,
            manifest_raw: processed.manifest_raw_json,
            package_size_bytes: processed.pkg_size,
            is_active: true,
            is_builtin: false,
        },
    )
    .await?;

    // 在事务中激活该版本
    AlgorithmRepo::activate_version(
        &state.db,
        &validated_manifest.algorithm_id,
        &validated_manifest.version,
        Some(&validated_manifest.platform_id),
    )
    .await?;

    // 热加载至内存注册中心，并热重载给运行中管线 (零停机、不断流)
    let target_dir = PathBuf::from(&target_dir_str);
    if let Ok(pkg) = state
        .algo_registry
        .load_and_register(&target_dir, false)
        .await
    {
        tracing::info!(
            algorithm_id = %pkg.manifest().algorithm_id,
            version = %pkg.manifest().version,
            "算法包上传成功并已即时热装载"
        );
        if let Ok(inst) = pkg.create_instance("{}", None) {
            let worker = infer::InferenceWorker::new(std::sync::Arc::new(inst));
            state
                .pipeline
                .reload_algorithm_on_pumps(worker.handle())
                .await;
        }
    }

    // 记录审计日志
    let _ = OplogRepo::record(
        &state.db,
        &user.username,
        "algorithm",
        "upload",
        "POST",
        "/api/v1/algorithms/upload",
        "",
        &serde_json::json!({
            "algorithmId": validated_manifest.algorithm_id,
            "version": validated_manifest.version,
            "platform": validated_manifest.platform_id,
            "packageRoot": target_dir_str,
        })
        .to_string(),
        200,
        0,
        "",
        "",
    )
    .await;

    Ok(ApiResponse::success(SandboxCheckResultDto {
        passed: true,
        steps_total: 7,
        steps_passed: 7,
        steps,
        error_message: None,
        version: Some(UploadVersionInfo {
            algorithm_id: validated_manifest.algorithm_id.clone(),
            version: validated_manifest.version.clone(),
            platform_id: validated_manifest.platform_id.clone(),
            package_root: target_dir_str,
            is_active: true,
        }),
        manifest: Some(validated_manifest),
    }))
}

/// 激活指定版本 (支持多平台并联动管线单进程优雅热重载)
async fn activate_version(
    State(state): State<AppState>,
    user: AuthUser,
    AxumPath((id, version)): AxumPath<(String, String)>,
) -> Result<ApiResponse<Option<()>>, ApiError> {
    let aid = resolve_algorithm_id(&state.db, &id).await?;
    let cur_plat = current_platform_id();

    // 优先匹配当前平台的版本
    let ver_model =
        match AlgorithmRepo::find_version(&state.db, &aid, &version, Some(cur_plat)).await? {
            Some(v) => v,
            None => AlgorithmRepo::find_version(&state.db, &aid, &version, None)
                .await?
                .ok_or_else(|| ApiError::NotFound(format!("未找到算法版本: {aid}:{version}")))?,
        };

    // 事务切换激活状态
    AlgorithmRepo::activate_version(&state.db, &aid, &version, Some(&ver_model.platform_id))
        .await?;

    // 检查并加载进内存注册中心，并通知 Pipeline 优雅热替换
    let root = Path::new(&ver_model.package_root);
    if root.is_dir() {
        if let Ok(pkg) = state.algo_registry.load_and_register(root, false).await {
            if let Ok(inst) = pkg.create_instance("{}", None) {
                let worker = infer::InferenceWorker::new(std::sync::Arc::new(inst));
                let count = state
                    .pipeline
                    .reload_algorithm_on_pumps(worker.handle())
                    .await;
                tracing::info!(
                    algorithm_id = %aid,
                    version = %version,
                    reloaded_pumps = count,
                    "已原子完成单进程算法版本优雅热重载"
                );
            }
        }
    }

    // 审计日志
    let _ = OplogRepo::record(
        &state.db,
        &user.username,
        "algorithm",
        "activate",
        "PUT",
        &format!("/api/v1/algorithms/{aid}/versions/{version}/activate"),
        "",
        &serde_json::json!({ "activatedVersion": version, "platform": ver_model.platform_id })
            .to_string(),
        200,
        0,
        "",
        "",
    )
    .await;

    Ok(ApiResponse::success(None))
}

/// 安全卸载指定版本 (防孤儿文件与路径逃逸保护)
async fn uninstall_version(
    State(state): State<AppState>,
    user: AuthUser,
    AxumPath((id, version)): AxumPath<(String, String)>,
) -> Result<ApiResponse<Option<()>>, ApiError> {
    let aid = resolve_algorithm_id(&state.db, &id).await?;
    let cur_plat = current_platform_id();

    // 执行卸载，内置算法或使用中算法会自动阻断并返回对应错误
    let package_root =
        match AlgorithmRepo::uninstall_version(&state.db, &aid, &version, Some(cur_plat)).await {
            Ok(r) => r,
            Err(db::DbError::NotFound { .. }) => {
                AlgorithmRepo::uninstall_version(&state.db, &aid, &version, None).await?
            }
            Err(e) => return Err(e.into()),
        };

    // 清理物理磁盘目录（防孤儿死文件），强约束仅限 var/packages 目录树内
    let root_path = PathBuf::from(&package_root);
    if !package_root.is_empty()
        && root_path.exists()
        && (root_path.starts_with("var/packages")
            || root_path
                .canonicalize()
                .map(|c| c.to_string_lossy().contains("var/packages"))
                .unwrap_or(false))
    {
        let _ = std::fs::remove_dir_all(&root_path);
    }

    // 检查剩余版本是否需要更新内存注册中心
    if let Some(algo) = AlgorithmRepo::find_by_algorithm_id(&state.db, &aid).await? {
        if !algo.active_version.is_empty() {
            if let Some(active_ver) =
                AlgorithmRepo::find_version(&state.db, &aid, &algo.active_version, None).await?
            {
                let p = Path::new(&active_ver.package_root);
                if p.is_dir() {
                    let _ = state.algo_registry.load_and_register(p, false).await;
                }
            }
        }
    } else {
        // 该算法已被彻底删除
        state.algo_registry.unregister(&aid).await;
    }

    // 审计日志
    let _ = OplogRepo::record(
        &state.db,
        &user.username,
        "algorithm",
        "uninstall",
        "DELETE",
        &format!("/api/v1/algorithms/{aid}/versions/{version}"),
        "",
        &serde_json::json!({ "uninstalledVersion": version }).to_string(),
        200,
        0,
        "",
        "",
    )
    .await;

    Ok(ApiResponse::success(None))
}

// ---------------------------------
// 历史兼容端点
// ---------------------------------

async fn legacy_list_packages(
    State(state): State<AppState>,
) -> Result<ApiResponse<Vec<AlgoManifest>>, ApiError> {
    let list = state.algo_registry.list().await;
    Ok(ApiResponse::success(list))
}

async fn verify_package(
    State(_state): State<AppState>,
    Json(req): Json<VerifyPackageRequest>,
) -> Result<ApiResponse<SandboxCheckResultDto>, ApiError> {
    let steps = get_standard_steps();

    let path_str = req.package_path.unwrap_or_else(|| {
        let cur = current_platform_id();
        format!("algo-packages/{cur}/general_detection")
    });

    let p = PathBuf::from(&path_str);
    if !p.is_dir() {
        return Ok(ApiResponse::success(SandboxCheckResultDto {
            passed: false,
            steps_total: 7,
            steps_passed: 0,
            steps,
            error_message: Some(format!("算法包目录不存在: {path_str}")),
            version: None,
            manifest: None,
        }));
    }

    match AlgoSandbox::validate_package(&p, false) {
        Ok(manifest) => Ok(ApiResponse::success(SandboxCheckResultDto {
            passed: true,
            steps_total: 7,
            steps_passed: 7,
            steps,
            error_message: None,
            version: None,
            manifest: Some(manifest),
        })),
        Err(e) => {
            let failed_idx = parse_failed_step_index(&e);
            Ok(ApiResponse::success(SandboxCheckResultDto {
                passed: false,
                steps_total: 7,
                steps_passed: failed_idx,
                steps,
                error_message: Some(e.to_string()),
                version: None,
                manifest: None,
            }))
        }
    }
}

async fn scan_packages(State(state): State<AppState>) -> Result<ApiResponse<usize>, ApiError> {
    let base = Path::new(DEFAULT_ALGO_PACKAGES_DIR);
    let count = state
        .algo_registry
        .scan_and_register(base, false)
        .await
        .unwrap_or(0);
    Ok(ApiResponse::success(count))
}

// ---------------------------------
// 归档解压与文件复制工具函数
// ---------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
    Zip,
    TarGz,
    Tar,
}

fn detect_archive_format(bytes: &[u8], filename: Option<&str>) -> Result<ArchiveFormat, String> {
    if bytes.len() >= 4 && &bytes[0..4] == b"PK\x03\x04" {
        return Ok(ArchiveFormat::Zip);
    }
    if bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b {
        return Ok(ArchiveFormat::TarGz);
    }
    if bytes.len() >= 512 && &bytes[257..262] == b"ustar" {
        return Ok(ArchiveFormat::Tar);
    }
    if let Some(name) = filename {
        let lower = name.to_lowercase();
        if lower.ends_with(".zip") {
            return Ok(ArchiveFormat::Zip);
        }
        if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
            return Ok(ArchiveFormat::TarGz);
        }
        if lower.ends_with(".tar") {
            return Ok(ArchiveFormat::Tar);
        }
    }
    Err("不支持的归档格式，仅支持 .zip、.tar.gz (.tgz) 与 .tar 格式".to_string())
}

fn extract_archive_package(
    bytes: &[u8],
    filename: Option<&str>,
    dest_dir: &Path,
) -> Result<PathBuf, String> {
    let format = detect_archive_format(bytes, filename)?;
    match format {
        ArchiveFormat::Zip => extract_zip(bytes, dest_dir)?,
        ArchiveFormat::TarGz => {
            let cursor = std::io::Cursor::new(bytes);
            let gz = flate2::read::GzDecoder::new(cursor);
            extract_tar(gz, dest_dir)?;
        }
        ArchiveFormat::Tar => {
            let cursor = std::io::Cursor::new(bytes);
            extract_tar(cursor, dest_dir)?;
        }
    }

    find_extracted_package_root(dest_dir)
}

fn extract_zip(bytes: &[u8], dest_dir: &Path) -> Result<(), String> {
    let cursor = std::io::Cursor::new(bytes);
    let mut zip = zip::ZipArchive::new(cursor).map_err(|e| format!("解析 ZIP 文件失败: {e}"))?;

    for i in 0..zip.len() {
        let mut file = zip
            .by_index(i)
            .map_err(|e| format!("读取 ZIP 条目失败: {e}"))?;
        let enclosed_name = file
            .enclosed_name()
            .ok_or_else(|| "ZIP 中包含非法路径或路径穿透攻击组件 (Zip Slip)".to_string())?;

        let out_path = dest_dir.join(enclosed_name);
        if !out_path.starts_with(dest_dir) {
            return Err("ZIP 条目解压路径超出目标沙箱目录 (Zip Slip 攻击防护拦截)".to_string());
        }

        if file.name().ends_with('/') {
            std::fs::create_dir_all(&out_path)
                .map_err(|e| format!("创建目录失败 {}: {e}", out_path.display()))?;
        } else {
            if let Some(p) = out_path.parent().filter(|p| !p.exists()) {
                std::fs::create_dir_all(p)
                    .map_err(|e| format!("创建父目录失败 {}: {e}", p.display()))?;
            }
            let mut outfile = std::fs::File::create(&out_path)
                .map_err(|e| format!("创建文件失败 {}: {e}", out_path.display()))?;
            std::io::copy(&mut file, &mut outfile)
                .map_err(|e| format!("写入文件内容失败 {}: {e}", out_path.display()))?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Some(mode) = file.unix_mode() {
                    let _ =
                        std::fs::set_permissions(&out_path, std::fs::Permissions::from_mode(mode));
                }
            }
        }
    }
    Ok(())
}

fn extract_tar<R: std::io::Read>(reader: R, dest_dir: &Path) -> Result<(), String> {
    let mut archive = tar::Archive::new(reader);
    let entries = archive
        .entries()
        .map_err(|e| format!("读取 TAR 条目失败: {e}"))?;

    for entry_res in entries {
        let mut entry = entry_res.map_err(|e| format!("解压 TAR 条目异常: {e}"))?;
        let path = entry
            .path()
            .map_err(|e| format!("读取 TAR 路径失败: {e}"))?
            .into_owned();

        // 强安全校验：严禁绝对路径及 ParentDir / RootDir 路径穿透组件
        if path.is_absolute() {
            return Err("TAR 中包含非法绝对路径 (Tar Slip 攻击防护拦截)".to_string());
        }

        for comp in path.components() {
            if matches!(
                comp,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            ) {
                return Err("TAR 中包含非法相对路径组件或前缀 (Tar Slip 攻击防护拦截)".to_string());
            }
        }

        let out_path = dest_dir.join(&path);
        if !out_path.starts_with(dest_dir) {
            return Err("TAR 条目解压路径超出目标沙箱目录 (Tar Slip 攻击防护拦截)".to_string());
        }

        if entry.header().entry_type().is_dir() {
            std::fs::create_dir_all(&out_path)
                .map_err(|e| format!("创建目录失败 {}: {e}", out_path.display()))?;
        } else {
            if let Some(parent) = out_path.parent().filter(|p| !p.exists()) {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("创建父目录失败 {}: {e}", parent.display()))?;
            }
            entry
                .unpack(&out_path)
                .map_err(|e| format!("解压文件失败 {}: {e}", out_path.display()))?;
        }
    }
    Ok(())
}

fn find_extracted_package_root(base_dir: &Path) -> Result<PathBuf, String> {
    if base_dir.join(ALGO_MANIFEST_FILENAME).is_file() {
        return Ok(base_dir.to_path_buf());
    }

    if let Ok(entries) = std::fs::read_dir(base_dir) {
        let valid_dirs: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_dir()
                    && p.file_name()
                        .is_none_or(|n| !n.to_string_lossy().starts_with('.'))
            })
            .collect();

        if valid_dirs.len() == 1 && valid_dirs[0].join(ALGO_MANIFEST_FILENAME).is_file() {
            return Ok(valid_dirs[0].clone());
        }

        for d in valid_dirs {
            if d.join(ALGO_MANIFEST_FILENAME).is_file() {
                return Ok(d);
            }
        }
    }

    Err(format!(
        "解压归档包后未在根目录或单层子目录中找到 {ALGO_MANIFEST_FILENAME} 文件"
    ))
}

fn copy_dir_all(src: &Path, dst: &Path) -> Result<(), std::io::Error> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let from = entry.path();
        let to = dst.join(entry.file_name());

        if ty.is_dir() {
            copy_dir_all(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_archive_format() {
        // Zip magic: PK\x03\x04
        let zip_magic = b"PK\x03\x04\x14\x00\x00\x00";
        assert_eq!(
            detect_archive_format(zip_magic, None).unwrap(),
            ArchiveFormat::Zip
        );
        assert_eq!(
            detect_archive_format(b"unknown", Some("model.zip")).unwrap(),
            ArchiveFormat::Zip
        );

        // Gzip magic: 0x1f, 0x8b
        let gz_magic = &[0x1f, 0x8b, 0x08, 0x00];
        assert_eq!(
            detect_archive_format(gz_magic, None).unwrap(),
            ArchiveFormat::TarGz
        );
        assert_eq!(
            detect_archive_format(b"unknown", Some("model.tar.gz")).unwrap(),
            ArchiveFormat::TarGz
        );
        assert_eq!(
            detect_archive_format(b"unknown", Some("model.tgz")).unwrap(),
            ArchiveFormat::TarGz
        );

        // Tar magic at offset 257: "ustar"
        let mut tar_magic = vec![0u8; 512];
        tar_magic[257..262].copy_from_slice(b"ustar");
        assert_eq!(
            detect_archive_format(&tar_magic, None).unwrap(),
            ArchiveFormat::Tar
        );
        assert_eq!(
            detect_archive_format(b"unknown", Some("model.tar")).unwrap(),
            ArchiveFormat::Tar
        );
    }

    #[test]
    fn test_extract_tar_gz_archive() {
        use flate2::write::GzEncoder;
        use flate2::Compression;

        let temp = std::env::temp_dir().join(format!(
            "test_argus_targz_{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&temp).unwrap();

        let mut gz_encoder = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut tar_builder = tar::Builder::new(&mut gz_encoder);
            let manifest_content = br#"{"algorithmId": "test_targz_pkg"}"#;
            let mut header = tar::Header::new_gnu();
            header.set_path("pkg_dir/manifest.json").unwrap();
            header.set_size(manifest_content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar_builder.append(&header, &manifest_content[..]).unwrap();
            tar_builder.finish().unwrap();
        }
        let gz_bytes = gz_encoder.finish().unwrap();

        let extracted_dir = extract_archive_package(&gz_bytes, Some("test.tar.gz"), &temp).unwrap();
        assert!(extracted_dir.join("manifest.json").is_file());

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn test_extract_uncompressed_tar_archive() {
        let temp =
            std::env::temp_dir().join(format!("test_argus_tar_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&temp).unwrap();

        let mut tar_bytes = Vec::new();
        {
            let mut tar_builder = tar::Builder::new(&mut tar_bytes);
            let manifest_content = br#"{"algorithmId": "test_pure_tar_pkg"}"#;
            let mut header = tar::Header::new_gnu();
            header.set_path("manifest.json").unwrap();
            header.set_size(manifest_content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar_builder.append(&header, &manifest_content[..]).unwrap();
            tar_builder.finish().unwrap();
        }

        let extracted_dir = extract_archive_package(&tar_bytes, Some("test.tar"), &temp).unwrap();
        assert!(extracted_dir.join("manifest.json").is_file());

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn test_extract_tar_rejects_absolute_path_and_parent_dir() {
        let temp = std::env::temp_dir().join(format!(
            "test_argus_tar_slip_{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&temp).unwrap();

        // 构造包含恶意绝对路径的 TAR (直接修改底层 header 名字段模拟恶意包)
        let mut tar_bytes = Vec::new();
        {
            let mut tar_builder = tar::Builder::new(&mut tar_bytes);
            let evil_content = b"malicious content";
            let mut header = tar::Header::new_gnu();
            let name_bytes = b"/etc/passwd_fake";
            header.as_mut_bytes()[..name_bytes.len()].copy_from_slice(name_bytes);
            header.set_size(evil_content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar_builder.append(&header, &evil_content[..]).unwrap();
            tar_builder.finish().unwrap();
        }

        let cursor = std::io::Cursor::new(&tar_bytes);
        let res = extract_tar(cursor, &temp);
        assert!(res.is_err(), "Must reject tar with absolute path");
        assert!(
            res.unwrap_err().contains("Tar Slip"),
            "Error message must mention Tar Slip protection"
        );

        // 构造包含 .. 相对路径穿透的 TAR
        let mut tar_bytes_slip = Vec::new();
        {
            let mut tar_builder = tar::Builder::new(&mut tar_bytes_slip);
            let evil_content = b"malicious content";
            let mut header = tar::Header::new_gnu();
            let name_bytes = b"../../etc/evil.conf";
            header.as_mut_bytes()[..name_bytes.len()].copy_from_slice(name_bytes);
            header.set_size(evil_content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar_builder.append(&header, &evil_content[..]).unwrap();
            tar_builder.finish().unwrap();
        }

        let cursor = std::io::Cursor::new(&tar_bytes_slip);
        let res_slip = extract_tar(cursor, &temp);
        assert!(
            res_slip.is_err(),
            "Must reject tar with parent dir component"
        );

        let _ = std::fs::remove_dir_all(&temp);
    }
}
