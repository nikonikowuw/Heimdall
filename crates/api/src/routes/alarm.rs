use axum::extract::{Query, State};
use axum::routing::get;
use axum::Router;
use serde::Deserialize;

use db::AlarmRepo;

use crate::error::ApiError;
use crate::response::ApiResponse;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct AlarmQuery {
    pub camera_id: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: u64,
    #[serde(default)]
    pub offset: u64,
}

fn default_limit() -> u64 {
    20
}

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(list_alarms))
}

async fn list_alarms(
    State(state): State<AppState>,
    Query(params): Query<AlarmQuery>,
) -> Result<ApiResponse<Vec<db::entity::alarm::Model>>, ApiError> {
    let list = AlarmRepo::list_recent(
        &state.db,
        params.camera_id.as_deref(),
        params.limit,
        params.offset,
    )
    .await?;
    Ok(ApiResponse::success(list))
}
