use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(ws_handler))
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: AppState) {
    let mut rx = state.event_broadcaster.subscribe();

    while let Ok(event) = rx.recv().await {
        if let Ok(json_str) = serde_json::to_string(&event) {
            if socket.send(Message::Text(json_str.into())).await.is_err() {
                break;
            }
        }
    }
}
