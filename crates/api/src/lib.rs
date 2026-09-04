pub mod error;
pub mod response;
pub mod routes;
pub mod state;
pub mod static_files;

pub use error::ApiError;
pub use response::ApiResponse;
pub use state::{AppState, WsBroadcastEvent};

use axum::routing::get;
use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

/// 构造整体 HTTP + WebSocket + 静态前端 SPA 路由器
pub fn create_app(state: AppState) -> Router {
    Router::new()
        .nest("/api/v1", routes::api_router())
        .fallback(get(static_files::static_handler))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
