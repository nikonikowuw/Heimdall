use axum::Router;

use crate::state::AppState;

pub mod alarm;
pub mod camera;
pub mod oplog;
pub mod task;
pub mod whep;
pub mod ws;

/// 组装所有 RESTful API 与流媒体/事件路由
pub fn api_router() -> Router<AppState> {
    Router::new()
        .nest("/cameras", camera::router())
        .nest("/tasks", task::router())
        .nest("/alarms", alarm::router())
        .nest("/logs/operations", oplog::router())
        .nest("/webrtc/whep", whep::router())
        .nest("/ws/events", ws::router())
}
