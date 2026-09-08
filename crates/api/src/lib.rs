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
pub mod time_service;

pub use error::ApiError;
pub use network_service::NetworkService;
pub use response::ApiResponse;
pub use routes::system::DbEvictionStoreAdapter;
pub use state::{AppState, WsBroadcastEvent};

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
