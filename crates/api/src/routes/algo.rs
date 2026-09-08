use axum::extract::DefaultBodyLimit;
use axum::extract::{Multipart, Path as AxumPath, Query, State};
use axum::routing::{delete, get, post, put};
use axum::Router;
use db::{AlgorithmRepo, AlgorithmStats};

pub use crate::algo::dto::*;
use crate::algo::{
    activate_version as service_activate_version, handle_package_upload, load_version_dtos,
    resolve_algorithm_id, uninstall_version as service_uninstall_version,
};
use crate::error::ApiError;
use crate::response::ApiResponse;
use crate::state::AppState;

pub fn router(upload_limit_bytes: usize) -> Router<AppState> {
    Router::new()
        .route("/", get(list_algorithms))
        .route("/stats", get(get_stats))
        .route(
            "/upload",
            post(upload_package).layer(DefaultBodyLimit::max(upload_limit_bytes)),
        )
        .route("/{id}", get(get_algorithm))
        .route("/{id}/versions", get(list_versions))
        .route("/{id}/versions/{version}/activate", put(activate_version))
        .route("/{id}/versions/{version}", delete(uninstall_version))
}

/// 分页查询算法列表（包含版本树）
async fn list_algorithms(
    State(state): State<AppState>,
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
async fn get_stats(State(state): State<AppState>) -> Result<ApiResponse<AlgorithmStats>, ApiError> {
    let stats = AlgorithmRepo::stats(&state.db).await?;
    Ok(ApiResponse::success(stats))
}

/// 查询单个算法详情
async fn get_algorithm(
    State(state): State<AppState>,
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
    AxumPath(id): AxumPath<String>,
) -> Result<ApiResponse<Vec<AlgorithmVersionItemDto>>, ApiError> {
    let aid = resolve_algorithm_id(&state.db, &id).await?;
    let versions = load_version_dtos(&state.db, &aid).await?;
    Ok(ApiResponse::success(versions))
}

/// 上传 .tar.gz / .zip / .tar 算法包并在物理沙箱中自检、落盘至 var/packages、入库并热加载
async fn upload_package(
    State(state): State<AppState>,
    multipart: Multipart,
) -> Result<ApiResponse<SandboxCheckResultDto>, ApiError> {
    let result = handle_package_upload(&state, multipart).await?;
    Ok(ApiResponse::success(result))
}

/// 激活指定版本 (支持多平台并联动管线单进程优雅热重载)
async fn activate_version(
    State(state): State<AppState>,
    AxumPath((id, version)): AxumPath<(String, String)>,
) -> Result<ApiResponse<Option<()>>, ApiError> {
    service_activate_version(&state, &id, &version).await?;
    Ok(ApiResponse::success(None))
}

/// 安全卸载指定版本 (防孤儿文件与路径逃逸保护)
async fn uninstall_version(
    State(state): State<AppState>,
    AxumPath((id, version)): AxumPath<(String, String)>,
) -> Result<ApiResponse<Option<()>>, ApiError> {
    service_uninstall_version(&state, &id, &version).await?;
    Ok(ApiResponse::success(None))
}
