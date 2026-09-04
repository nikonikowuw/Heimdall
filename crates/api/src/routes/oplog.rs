use axum::extract::{Query, State};
use axum::routing::get;
use axum::Router;
use serde::Deserialize;

use db::OplogRepo;

use crate::error::ApiError;
use crate::response::ApiResponse;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct OplogQuery {
    pub module: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: u64,
    #[serde(default)]
    pub offset: u64,
}

fn default_limit() -> u64 {
    20
}

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(list_oplogs))
}

async fn list_oplogs(
    State(state): State<AppState>,
    Query(params): Query<OplogQuery>,
) -> Result<ApiResponse<Vec<db::entity::oplog::Model>>, ApiError> {
    let list = OplogRepo::list_recent(
        &state.db,
        params.module.as_deref(),
        params.limit,
        params.offset,
    )
    .await?;
    Ok(ApiResponse::success(list))
}
