use axum::extract::{Query, State};
use axum::routing::get;
use axum::Router;
use serde::{Deserialize, Serialize};

use db::repository::oplog::{ListParams, StatusClass};
use db::OplogRepo;

use crate::error::ApiError;
use crate::response::ApiResponse;
use crate::state::AppState;

const DEFAULT_LIMIT: u64 = 20;
const MAX_LIMIT: u64 = 100;
/// 关键字长度上限：LIKE 模式不能由客户端无限拉长
const MAX_KEYWORD_CHARS: usize = 64;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OplogQuery {
    pub module: Option<String>,
    /// `success` = 2xx，`failed` = 4xx/5xx；其余取值报 400
    pub status: Option<String>,
    /// 关键字，字面量匹配操作人、模块、动作、请求路径与客户端 IP
    pub q: Option<String>,
    #[serde(alias = "from_ms")]
    pub from_ms: Option<i64>,
    #[serde(alias = "to_ms")]
    pub to_ms: Option<i64>,
    #[serde(default = "default_limit")]
    pub limit: u64,
    #[serde(default)]
    pub offset: u64,
}

fn default_limit() -> u64 {
    DEFAULT_LIMIT
}

/// 解析 `status` 查询参数，空串视为未过滤，未知取值直接拒绝而不静默忽略
fn parse_status(value: Option<&str>) -> Result<Option<StatusClass>, ApiError> {
    match value.map(str::trim).filter(|raw| !raw.is_empty()) {
        None => Ok(None),
        Some("success") => Ok(Some(StatusClass::Success)),
        Some("failed") => Ok(Some(StatusClass::Failed)),
        Some(other) => Err(ApiError::BadRequest(format!(
            "status 只支持 success 或 failed，收到 {other}"
        ))),
    }
}

/// 解析 `q` 查询参数，空串视为未过滤，超长直接拒绝
fn parse_keyword(value: Option<&str>) -> Result<Option<&str>, ApiError> {
    let Some(keyword) = value.map(str::trim).filter(|raw| !raw.is_empty()) else {
        return Ok(None);
    };
    if keyword.chars().count() > MAX_KEYWORD_CHARS {
        return Err(ApiError::BadRequest(format!(
            "q 长度不能超过 {MAX_KEYWORD_CHARS} 个字符"
        )));
    }
    Ok(Some(keyword))
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
    let status = parse_status(params.status.as_deref())?;
    let keyword = parse_keyword(params.q.as_deref())?;

    let list = OplogRepo::list_recent(
        &state.db,
        &ListParams {
            module: params.module.as_deref(),
            status,
            keyword,
            from_ms: params.from_ms,
            to_ms: params.to_ms,
            limit,
            offset: params.offset,
        },
    )
    .await?;
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

    #[test]
    fn oplog_query_deserializes_camel_and_snake_case_time_range() {
        let json = r#"{"module":"camera","fromMs":1000,"toMs":2000,"limit":10,"offset":0}"#;
        let query: OplogQuery = serde_json::from_str(json).unwrap();
        assert_eq!(query.module, Some("camera".to_string()));
        assert_eq!(query.from_ms, Some(1000));
        assert_eq!(query.to_ms, Some(2000));
        assert_eq!(query.limit, 10);
        assert_eq!(query.offset, 0);

        let json_snake = r#"{"from_ms":3000,"to_ms":4000}"#;
        let query_snake: OplogQuery = serde_json::from_str(json_snake).unwrap();
        assert_eq!(query_snake.from_ms, Some(3000));
        assert_eq!(query_snake.to_ms, Some(4000));
    }

    #[test]
    fn oplog_query_deserializes_status_and_keyword() {
        let json = r#"{"status":"failed","q":"cameras"}"#;
        let query: OplogQuery = serde_json::from_str(json).unwrap();
        assert_eq!(query.status.as_deref(), Some("failed"));
        assert_eq!(query.q.as_deref(), Some("cameras"));
    }

    #[test]
    fn parse_status_accepts_only_the_documented_vocabulary() {
        assert!(parse_status(None).unwrap().is_none());
        assert!(parse_status(Some("")).unwrap().is_none());
        assert!(parse_status(Some("  ")).unwrap().is_none());
        assert_eq!(
            parse_status(Some("success")).unwrap(),
            Some(StatusClass::Success)
        );
        assert_eq!(
            parse_status(Some(" failed ")).unwrap(),
            Some(StatusClass::Failed)
        );

        let rejected = parse_status(Some("2xx")).unwrap_err();
        assert!(rejected.to_string().contains("status"));
    }

    #[test]
    fn parse_keyword_trims_and_bounds_the_pattern() {
        assert!(parse_keyword(None).unwrap().is_none());
        assert!(parse_keyword(Some("   ")).unwrap().is_none());
        assert_eq!(parse_keyword(Some("  cameras  ")).unwrap(), Some("cameras"));

        let at_limit = "a".repeat(MAX_KEYWORD_CHARS);
        assert_eq!(
            parse_keyword(Some(&at_limit)).unwrap(),
            Some(at_limit.as_str())
        );

        let over_limit = "a".repeat(MAX_KEYWORD_CHARS + 1);
        let rejected = parse_keyword(Some(&over_limit)).unwrap_err();
        assert!(rejected.to_string().contains("q"));
    }
}
