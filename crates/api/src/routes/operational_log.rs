use axum::extract::{Query, State};
use axum::routing::get;
use axum::Router;
use serde::{Deserialize, Serialize};

use crate::error::ApiError;
use crate::response::ApiResponse;
use crate::state::AppState;

const DEFAULT_LIMIT: u64 = 50;
const MAX_LIMIT: u64 = 200;

#[derive(Debug, Deserialize)]
pub struct OperationalLogQuery {
    pub level: Option<String>,
    pub event: Option<String>,
    pub camera_id: Option<String>,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    #[serde(default = "default_limit")]
    pub limit: u64,
    /// 时间戳游标（13 位 UTC 毫秒，返回早于该时间的记录；兼容别名 cursor）
    #[serde(alias = "cursor")]
    pub before: Option<i64>,
}

fn default_limit() -> u64 {
    DEFAULT_LIMIT
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationalLogDto {
    pub id: i64,
    pub ts_ms: i64,
    pub level: String,
    pub event: String,
    pub target: String,
    pub message: String,
    pub camera_id: Option<String>,
    pub extra_json: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationalLogPage {
    pub items: Vec<OperationalLogDto>,
    pub has_more: bool,
    pub next_before: Option<i64>,
}

impl From<db::entity::operational_log::Model> for OperationalLogDto {
    fn from(model: db::entity::operational_log::Model) -> Self {
        Self {
            id: model.id,
            ts_ms: model.ts_ms,
            level: model.level,
            event: model.event,
            target: model.target,
            message: model.message,
            camera_id: model.camera_id,
            extra_json: model.extra_json,
        }
    }
}

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(list_operational_logs))
}

async fn list_operational_logs(
    State(state): State<AppState>,
    Query(params): Query<OperationalLogQuery>,
) -> Result<ApiResponse<OperationalLogPage>, ApiError> {
    let limit = params.limit.clamp(1, MAX_LIMIT);
    // 多查一条用于判断是否有下一页，避免额外 COUNT 查询
    let list = db::OperationalLogRepo::list(
        &state.db,
        &db::repository::operational_log::ListParams {
            level: params.level.as_deref(),
            event: params.event.as_deref(),
            camera_id: params.camera_id.as_deref(),
            from_ms: params.from_ms,
            to_ms: params.to_ms,
            before: params.before,
            limit: limit + 1,
        },
    )
    .await?;
    let has_more = list.len() as u64 > limit;
    let items: Vec<OperationalLogDto> = list
        .into_iter()
        .take(limit as usize)
        .map(OperationalLogDto::from)
        .collect();
    let next_before = if has_more {
        items.last().map(|i| i.ts_ms)
    } else {
        None
    };
    Ok(ApiResponse::success(OperationalLogPage {
        items,
        has_more,
        next_before,
    }))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn operational_log_dto_uses_camel_case() {
        let model = db::entity::operational_log::Model {
            id: 1,
            ts_ms: 1705312320000,
            level: "info".to_string(),
            event: "camera_online".to_string(),
            target: "media".to_string(),
            message: "摄像头 cam-01 连接就绪".to_string(),
            camera_id: Some("cam-01".to_string()),
            extra_json: None,
        };
        let dto = OperationalLogDto::from(model);
        let value = serde_json::to_value(dto).unwrap();

        assert_eq!(value["tsMs"], serde_json::json!(1705312320000_i64));
        assert_eq!(value["cameraId"], "cam-01");
        assert_eq!(value["event"], "camera_online");
        // 确保 snake_case 字段不存在
        assert!(value.get("ts_ms").is_none());
        assert!(value.get("camera_id").is_none());
    }

    #[test]
    fn operational_log_page_has_more_and_next_before_field() {
        let page = OperationalLogPage {
            items: vec![],
            has_more: true,
            next_before: Some(1705312000000),
        };
        let value = serde_json::to_value(page).unwrap();
        assert_eq!(value["hasMore"], serde_json::json!(true));
        assert_eq!(value["nextBefore"], serde_json::json!(1705312000000_i64));
        assert!(value.get("has_more").is_none());
        assert!(value.get("next_before").is_none());
    }

    #[test]
    fn operational_log_query_alias_cursor_to_before() {
        let json = r#"{"cursor": 1705312000000, "limit": 20}"#;
        let query: OperationalLogQuery = serde_json::from_str(json).unwrap();
        assert_eq!(query.before, Some(1705312000000));
        assert_eq!(query.limit, 20);

        let json2 = r#"{"before": 1705312999999}"#;
        let query2: OperationalLogQuery = serde_json::from_str(json2).unwrap();
        assert_eq!(query2.before, Some(1705312999999));
    }
}
