use axum::extract::{Path, State};
use axum::routing::get;
use axum::Router;
use serde::Serialize;

use crate::error::ApiError;
use crate::response::ApiResponse;
use crate::state::AppState;

/// `GET /api/v1/system/streams/{stream_key}`
pub async fn get_stream_health(
    State(state): State<AppState>,
    Path(stream_key): Path<String>,
) -> Result<ApiResponse<media::StreamHealthSnapshot>, ApiError> {
    let health = state
        .stream_hub
        .stream_health(&stream_key)
        .await
        .ok_or_else(|| ApiError::NotFound("流会话不存在".to_string()))?;

    Ok(ApiResponse::success(health))
}

/// 人脸识别对账队列的投递观测快照
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecognitionQueueStats {
    /// 有界队列容量上限（解释丢弃数时的参照基准）
    pub capacity: usize,
    /// 因队列满而丢弃的比对事件累计数
    pub dropped_total: u64,
    /// 观测到队满的次数，用于区分瞬时限流与持续过载
    pub full_observed_total: u64,
}

/// `GET /api/v1/system/recognition-queue`
///
/// 暴露「累计丢弃数 + 队满次数」而不是「积压深度」：
/// `Sender::capacity()` 是*剩余可用槽位*，在队满分支恒为 0，
/// 用它反推深度只会得到一个永远等于容量的常量。
pub async fn get_recognition_queue_stats(
    State(state): State<AppState>,
) -> Result<ApiResponse<RecognitionQueueStats>, ApiError> {
    let (dropped_total, full_observed_total) = state.recognition_queue_metrics.snapshot();
    Ok(ApiResponse::success(RecognitionQueueStats {
        capacity: crate::capture_service::DEFAULT_RECOGNITION_QUEUE_CAPACITY,
        dropped_total,
        full_observed_total,
    }))
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/streams/{stream_key}", get(get_stream_health))
        .route("/recognition-queue", get(get_recognition_queue_stats))
}
