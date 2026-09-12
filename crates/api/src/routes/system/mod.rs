mod network;
mod overview;
mod snapshot;
mod storage;
mod time;

pub use storage::DbEvictionStoreAdapter;

use axum::routing::get;
use axum::Router;

use crate::state::AppState;

#[inline]
pub(super) fn round_1dp(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/overview", get(overview::get_overview))
        .merge(network::router())
        .merge(snapshot::router())
        .merge(storage::router())
        .merge(time::router())
}
