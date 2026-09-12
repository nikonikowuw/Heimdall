use axum::extract::{Path, State};
use axum::routing::get;
use axum::Router;

use crate::error::ApiError;
use crate::response::ApiResponse;
use crate::state::AppState;

/// `GET /api/v1/system/media/streams/{stream_key}`
pub async fn get_stream_health(
    State(state): State<AppState>,
    Path(stream_key): Path<String>,
) -> Result<ApiResponse<media::StreamHealthSnapshot>, ApiError> {
    let health = state
        .stream_hub
        .stream_health(&stream_key)
        .await
        .ok_or_else(|| ApiError::NotFound("流会话不存在".to_string()))?;

    Ok(ApiResponse::success(health))
}

pub fn router() -> Router<AppState> {
    Router::new().route("/streams/{stream_key}", get(get_stream_health))
}
