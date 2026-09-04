use std::sync::Arc;
use tokio::sync::broadcast;

use pipeline::PipelineManager;
use sea_orm::DatabaseConnection;

/// WebSocket 广播事件模型
#[derive(Debug, Clone, serde::Serialize)]
pub struct WsBroadcastEvent {
    pub topic: String,
    pub payload: serde_json::Value,
    pub timestamp: i64,
}

/// Axum 共享应用状态句柄
#[derive(Clone, Debug)]
pub struct AppState {
    pub db: DatabaseConnection,
    pub pipeline: Arc<PipelineManager>,
    pub event_broadcaster: broadcast::Sender<WsBroadcastEvent>,
}

impl AppState {
    pub fn new(db: DatabaseConnection, pipeline: Arc<PipelineManager>) -> Self {
        let (event_broadcaster, _) = broadcast::channel(1024);
        Self {
            db,
            pipeline,
            event_broadcaster,
        }
    }
}
