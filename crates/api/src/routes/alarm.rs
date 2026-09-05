use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use types::{AlarmSeverity, AlarmStatus, TOPIC_ALARM_STATUS_CHANGED};

use db::AlarmRepo;

use crate::error::ApiError;
use crate::middleware::AuthUser;
use crate::response::ApiResponse;
use crate::state::{AppState, WsBroadcastEvent};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlarmDto {
    pub id: i64,
    pub event_id: String,
    pub camera_id: String,
    pub alarm_type_id: String,
    pub occurred_at: i64,
    pub target_label: String,
    pub confidence: f32,
    pub track_id: i64,
    pub bbox_json: String,
    pub image_id: String,
    pub image_rel_path: String,
    pub crop_image_id: String,
    pub crop_image_rel_path: String,
    pub rule_type: String,
    pub severity: AlarmSeverity,
    pub status: AlarmStatus,
    pub handled_at: Option<i64>,
    pub created_at: i64,
}

impl From<db::entity::alarm::Model> for AlarmDto {
    fn from(m: db::entity::alarm::Model) -> Self {
        Self {
            id: m.id,
            event_id: m.event_id,
            camera_id: m.camera_id,
            alarm_type_id: m.alarm_type_id,
            occurred_at: m.occurred_at.timestamp_millis(),
            target_label: m.target_label,
            confidence: m.confidence,
            track_id: m.track_id,
            bbox_json: m.bbox_json,
            image_id: m.image_id,
            image_rel_path: m.image_rel_path,
            crop_image_id: m.crop_image_id,
            crop_image_rel_path: m.crop_image_rel_path,
            rule_type: m.rule_type,
            severity: AlarmSeverity::from_str_loose(&m.severity),
            status: AlarmStatus::from_str_loose(&m.status),
            handled_at: m.handled_at.map(|t| t.timestamp_millis()),
            created_at: m.created_at.timestamp_millis(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct AlarmQuery {
    pub camera_id: Option<String>,
    pub status: Option<String>,
    pub start_time: Option<i64>,
    pub end_time: Option<i64>,
    #[serde(default = "default_limit")]
    pub limit: u64,
    #[serde(default)]
    pub offset: u64,
}

#[derive(Debug, Deserialize)]
pub struct UpdateAlarmStatusRequest {
    pub status: AlarmStatus,
}

fn default_limit() -> u64 {
    20
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_alarms))
        .route("/{id}/status", axum::routing::put(update_alarm_status))
}

async fn list_alarms(
    State(state): State<AppState>,
    Query(params): Query<AlarmQuery>,
) -> Result<ApiResponse<Vec<AlarmDto>>, ApiError> {
    let start_utc = params.start_time.and_then(DateTime::from_timestamp_millis);
    let end_utc = params.end_time.and_then(DateTime::from_timestamp_millis);

    let list = AlarmRepo::list_filtered(
        &state.db,
        params.camera_id.as_deref(),
        params.status.as_deref(),
        start_utc,
        end_utc,
        params.limit,
        params.offset,
    )
    .await?;
    let dtos = list.into_iter().map(AlarmDto::from).collect();
    Ok(ApiResponse::success(dtos))
}

async fn update_alarm_status(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<i64>,
    Json(payload): Json<UpdateAlarmStatusRequest>,
) -> Result<ApiResponse<AlarmDto>, ApiError> {
    let updated = AlarmRepo::update_status(&state.db, id, payload.status.as_str()).await?;
    let dto = AlarmDto::from(updated);

    // 广播告警状态变更事件
    let _ = state.event_broadcaster.send(WsBroadcastEvent {
        topic: TOPIC_ALARM_STATUS_CHANGED.to_string(),
        payload: serde_json::json!({
            "id": dto.id,
            "eventId": dto.event_id,
            "status": dto.status.as_str(),
            "handledAt": dto.handled_at,
        }),
        timestamp: chrono::Utc::now().timestamp_millis(),
    });

    Ok(ApiResponse::success(dto))
}
