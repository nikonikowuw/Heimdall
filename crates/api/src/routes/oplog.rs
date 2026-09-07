use axum::extract::{Query, State};
use axum::routing::get;
use axum::Router;
use serde::{Deserialize, Serialize};

use db::OplogRepo;

use crate::error::ApiError;
use crate::response::ApiResponse;
use crate::state::AppState;

const DEFAULT_LIMIT: u64 = 20;
const MAX_LIMIT: u64 = 100;

#[derive(Debug, Deserialize)]
pub struct OplogQuery {
    pub module: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: u64,
    #[serde(default)]
    pub offset: u64,
}

fn default_limit() -> u64 {
    DEFAULT_LIMIT
}

/// 面向 HTTP 客户端的操作日志 DTO，隔离 SeaORM entity 和时间/字段命名细节。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationLogDto {
    pub id: i64,
    pub username: String,
    pub module: String,
    pub action: String,
    pub method: String,
    pub path: String,
    pub query: String,
    pub body: String,
    pub status_code: i32,
    pub duration_ms: i64,
    pub ip: String,
    pub user_agent: String,
    pub created_at: i64,
}

impl From<db::entity::oplog::Model> for OperationLogDto {
    fn from(model: db::entity::oplog::Model) -> Self {
        Self {
            id: model.id,
            username: model.username,
            module: model.module,
            action: model.action,
            method: model.method,
            path: model.path,
            query: model.query,
            body: model.body,
            status_code: model.status_code,
            duration_ms: model.duration_ms,
            ip: model.ip,
            user_agent: model.user_agent,
            created_at: model.created_at.timestamp_millis(),
        }
    }
}

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(list_oplogs))
}

async fn list_oplogs(
    State(state): State<AppState>,
    Query(params): Query<OplogQuery>,
) -> Result<ApiResponse<Vec<OperationLogDto>>, ApiError> {
    let limit = params.limit.clamp(1, MAX_LIMIT);
    let list =
        OplogRepo::list_recent(&state.db, params.module.as_deref(), limit, params.offset).await?;
    let logs = list.into_iter().map(OperationLogDto::from).collect();
    Ok(ApiResponse::success(logs))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn operation_log_dto_uses_camel_case_and_unix_milliseconds() {
        let now = chrono::Utc::now();
        let dto = OperationLogDto::from(db::entity::oplog::Model {
            id: 1,
            username: "admin".to_string(),
            module: "camera".to_string(),
            action: "create".to_string(),
            method: "POST".to_string(),
            path: "/api/v1/cameras".to_string(),
            query: "source=test".to_string(),
            body: "{}".to_string(),
            status_code: 201,
            duration_ms: 4,
            ip: "127.0.0.1".to_string(),
            user_agent: "test-agent".to_string(),
            created_at: now,
        });
        let value = serde_json::to_value(dto).unwrap();

        assert_eq!(value["statusCode"], 201);
        assert_eq!(value["durationMs"], 4);
        assert_eq!(value["createdAt"], now.timestamp_millis());
        assert!(value.get("status_code").is_none());
        assert!(value.get("created_at").is_none());
    }
}
