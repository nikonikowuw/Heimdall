pub mod alarm_service;
pub mod algo;
pub mod camera_probe;
pub mod capture_service;
pub mod crypto;
pub mod error;
pub mod i18n;
pub mod metrics;
pub mod middleware;
pub mod network_service;
pub mod response;
pub mod routes;
pub mod state;
pub mod static_files;
pub mod system_info;
pub mod task_service;
pub mod time_service;
pub mod track_service;

pub use alarm_service::AlarmDispatchService;
pub use camera_probe::CameraProbeService;
pub use capture_service::CaptureDispatchService;
pub use error::ApiError;
pub use network_service::NetworkService;
pub use response::ApiResponse;
pub use routes::auth::sync_auth_state;
pub use routes::system::DbEvictionStoreAdapter;
pub use state::{AppState, WsBroadcastEvent};
pub use track_service::TrackDispatchService;

use axum::middleware::from_fn;
use axum::routing::get;
use axum::Router;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

/// 构造整体 HTTP + WebSocket + 静态前端 SPA 路由器
pub fn create_app(state: AppState) -> Router {
    let api = routes::api_router(&state).layer(from_fn(middleware::i18n_response_middleware));

    Router::new()
        .nest("/api/v1", api)
        .fallback(get(static_files::static_handler))
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
