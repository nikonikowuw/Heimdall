use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Router;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/", post(handle_whep_offer))
}

async fn handle_whep_offer(State(_state): State<AppState>, body: String) -> Response {
    tracing::debug!("收到 WHEP SDP Offer 请求, 长度: {}", body.len());
    // 骨架阶段：返回 501 或预备 SDP Answer 协商逻辑
    (
        StatusCode::NOT_IMPLEMENTED,
        [("Content-Type", "application/sdp")],
        "WHEP PeerConnection pending implementation",
    )
        .into_response()
}
