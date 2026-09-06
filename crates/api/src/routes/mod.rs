use axum::Router;

use crate::state::AppState;

pub mod alarm;
pub mod algo;
pub mod auth;
pub mod camera;
pub mod evidence;
pub mod live;
pub mod oplog;
pub mod system;
pub mod task;
pub mod ws;

/// 组装所有 RESTful API 与流媒体/事件路由
pub fn api_router(state: &AppState) -> Router<AppState> {
    let protected = Router::new()
        .nest("/cameras", camera::router())
        .nest("/tasks", task::router())
        .nest("/alarms", alarm::router())
        .nest("/evidence", evidence::router())
        .nest("/algorithms", algo::router(state.max_upload_size_bytes))
        .nest("/logs/operations", oplog::router())
        .nest("/system", system::router())
        .nest("/ws/events", ws::router())
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::middleware::require_auth,
        ));

    Router::new()
        .nest("/auth", auth::router())
        .nest("/live", live::router())
        .nest("/evidence/image", evidence::image_router())
        .merge(protected)
}
