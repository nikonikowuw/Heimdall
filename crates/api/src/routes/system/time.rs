use axum::extract::State;
use axum::Json;

use crate::error::ApiError;
use crate::response::ApiResponse;
use crate::state::AppState;

/// `GET /api/v1/system/time/status`
pub async fn get_time_status(
    State(_state): State<AppState>,
) -> Result<ApiResponse<types::TimeStatus>, ApiError> {
    let status = tokio::task::spawn_blocking(crate::time_service::TimeService::get_status_sync)
        .await
        .map_err(|e| ApiError::SystemInfo(format!("spawn_blocking 失败: {e}")))??;
    Ok(ApiResponse::success(status))
}

/// `GET /api/v1/system/time/config`
pub async fn get_time_config(
    State(state): State<AppState>,
) -> Result<ApiResponse<types::TimeConfig>, ApiError> {
    let config = crate::time_service::TimeService::get_config(&state.db).await?;
    Ok(ApiResponse::success(config))
}

/// `PUT /api/v1/system/time/config`
pub async fn update_time_config(
    State(state): State<AppState>,
    Json(config): Json<types::TimeConfig>,
) -> Result<ApiResponse<types::TimeConfig>, ApiError> {
    let timezone = config.timezone.clone();
    let ntp_enabled = config.ntp_enabled;
    tokio::task::spawn_blocking(move || {
        crate::time_service::TimeService::apply_time_config_sync(&timezone, ntp_enabled)
    })
    .await
    .map_err(|e| ApiError::SystemInfo(format!("spawn_blocking 失败: {e}")))??;

    let json = serde_json::to_string(&config)
        .map_err(|e| ApiError::TimeConfig(format!("序列化配置失败: {e}")))?;
    db::SystemConfigRepo::set(&state.db, "time_config", &json)
        .await
        .map_err(|e| ApiError::TimeConfig(format!("保存配置失败: {e}")))?;

    Ok(ApiResponse::success(config))
}

/// `POST /api/v1/system/time/sync`
pub async fn force_time_sync(
    State(_state): State<AppState>,
) -> Result<ApiResponse<types::ForceSyncResponse>, ApiError> {
    let synced = tokio::task::spawn_blocking(crate::time_service::TimeService::force_sync_sync)
        .await
        .map_err(|e| ApiError::TimeSyncFailed(format!("spawn_blocking 失败: {e}")))??;
    Ok(ApiResponse::success(types::ForceSyncResponse { synced }))
}

/// `POST /api/v1/system/time/set`
pub async fn set_system_time(
    State(_state): State<AppState>,
    Json(body): Json<crate::time_service::SetTimeRequest>,
) -> Result<ApiResponse<types::SetTimeResponse>, ApiError> {
    let time_str = body.time.clone();
    let (previous_time, new_time) = tokio::task::spawn_blocking(move || {
        crate::time_service::TimeService::set_system_time_sync(&time_str)
    })
    .await
    .map_err(|e| ApiError::SystemInfo(format!("spawn_blocking 失败: {e}")))??;

    Ok(ApiResponse::success(types::SetTimeResponse {
        applied: true,
        previous_time,
        new_time,
    }))
}

pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        .route("/time/status", axum::routing::get(get_time_status))
        .route(
            "/time/config",
            axum::routing::get(get_time_config).put(update_time_config),
        )
        .route("/time/sync", axum::routing::post(force_time_sync))
        .route("/time/set", axum::routing::post(set_system_time))
}
