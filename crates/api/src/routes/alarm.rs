use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use types::{AlarmSeverity, AlarmStatus, TOPIC_ALARM_STATUS_CHANGED};

use db::{AlarmFilter, AlarmRepo};

use crate::error::ApiError;
use crate::response::ApiResponse;
use crate::routes::query_params::parse_keyword;
use crate::state::{AppState, WsBroadcastEvent};

const DEFAULT_LIMIT: u64 = 20;
const MAX_LIMIT: u64 = 100;

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
    pub target_label: Option<String>,
    pub rule_type: Option<String>,
    pub severity: Option<String>,
    /// 关键字匹配事件 ID、类别、规则、通道 ID 或通道名称。
    pub q: Option<String>,
    pub start_time: Option<i64>,
    pub end_time: Option<i64>,
    #[serde(default = "default_limit")]
    pub limit: u64,
    #[serde(default)]
    pub offset: u64,
}

#[derive(Debug, Deserialize)]
pub struct AlarmCountQuery {
    pub camera_id: Option<String>,
    pub status: Option<String>,
    pub target_label: Option<String>,
    pub rule_type: Option<String>,
    pub severity: Option<String>,
    /// 关键字匹配事件 ID、类别、规则、通道 ID 或通道名称。
    pub q: Option<String>,
    pub start_time: Option<i64>,
    pub end_time: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlarmCountDto {
    pub total: u64,
}

#[derive(Debug, Deserialize)]
pub struct UpdateAlarmStatusRequest {
    pub status: AlarmStatus,
}

#[derive(Debug, Deserialize)]
pub struct BatchUpdateAlarmStatusRequest {
    pub ids: Vec<i64>,
    pub status: AlarmStatus,
}

fn default_limit() -> u64 {
    DEFAULT_LIMIT
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_alarms))
        .route("/count", get(count_alarms))
        .route(
            "/batch-status",
            axum::routing::post(batch_update_alarm_status),
        )
        .route("/{id}/status", axum::routing::put(update_alarm_status))
}

async fn list_alarms(
    State(state): State<AppState>,
    Query(params): Query<AlarmQuery>,
) -> Result<ApiResponse<Vec<AlarmDto>>, ApiError> {
    let limit = params.limit.clamp(1, MAX_LIMIT);
    let start_utc = params.start_time.and_then(DateTime::from_timestamp_millis);
    let end_utc = params.end_time.and_then(DateTime::from_timestamp_millis);
    let keyword = parse_keyword(params.q.as_deref())?;

    let list = AlarmRepo::list_filtered(
        &state.db,
        AlarmFilter {
            camera_id: params.camera_id.as_deref(),
            status: params.status.as_deref(),
            target_label: params.target_label.as_deref(),
            rule_type: params.rule_type.as_deref(),
            severity: params.severity.as_deref(),
            keyword,
            start_time: start_utc,
            end_time: end_utc,
        },
        limit,
        params.offset,
    )
    .await?;
    let dtos = list.into_iter().map(AlarmDto::from).collect();
    Ok(ApiResponse::success(dtos))
}

async fn count_alarms(
    State(state): State<AppState>,
    Query(params): Query<AlarmCountQuery>,
) -> Result<ApiResponse<AlarmCountDto>, ApiError> {
    let start_utc = params.start_time.and_then(DateTime::from_timestamp_millis);
    let end_utc = params.end_time.and_then(DateTime::from_timestamp_millis);
    let keyword = parse_keyword(params.q.as_deref())?;

    let total = AlarmRepo::count_filtered(
        &state.db,
        AlarmFilter {
            camera_id: params.camera_id.as_deref(),
            status: params.status.as_deref(),
            target_label: params.target_label.as_deref(),
            rule_type: params.rule_type.as_deref(),
            severity: params.severity.as_deref(),
            keyword,
            start_time: start_utc,
            end_time: end_utc,
        },
    )
    .await?;

    Ok(ApiResponse::success(AlarmCountDto { total }))
}

fn broadcast_alarm_status_change(state: &AppState, dto: &AlarmDto) {
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
}

async fn batch_update_alarm_status(
    State(state): State<AppState>,
    Json(payload): Json<BatchUpdateAlarmStatusRequest>,
) -> Result<ApiResponse<Vec<AlarmDto>>, ApiError> {
    let updated =
        AlarmRepo::update_status_by_ids(&state.db, &payload.ids, payload.status.as_str()).await?;
    let dtos: Vec<AlarmDto> = updated.into_iter().map(AlarmDto::from).collect();

    // 逐个广播告警状态变更事件
    for dto in &dtos {
        broadcast_alarm_status_change(&state, dto);
    }

    Ok(ApiResponse::success(dtos))
}

async fn update_alarm_status(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(payload): Json<UpdateAlarmStatusRequest>,
) -> Result<ApiResponse<AlarmDto>, ApiError> {
    let updated = AlarmRepo::update_status(&state.db, id, payload.status.as_str()).await?;
    let dto = AlarmDto::from(updated);

    // 广播告警状态变更事件
    broadcast_alarm_status_change(&state, &dto);

    Ok(ApiResponse::success(dto))
}
