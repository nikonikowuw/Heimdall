use axum::extract::State;
use axum::routing::get;
use axum::Router;

use db::TaskRepo;

use crate::error::ApiError;
use crate::response::ApiResponse;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(list_tasks))
}

async fn list_tasks(
    State(state): State<AppState>,
) -> Result<ApiResponse<Vec<db::entity::task::Model>>, ApiError> {
    let list = TaskRepo::list_all(&state.db).await?;
    Ok(ApiResponse::success(list))
}
