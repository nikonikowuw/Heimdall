use axum::extract::State;
use axum::routing::get;
use axum::Router;

use db::CameraRepo;

use crate::error::ApiError;
use crate::response::ApiResponse;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(list_cameras))
}

async fn list_cameras(
    State(state): State<AppState>,
) -> Result<ApiResponse<Vec<db::entity::camera::Model>>, ApiError> {
    let list = CameraRepo::list_all(&state.db).await?;
    Ok(ApiResponse::success(list))
}
