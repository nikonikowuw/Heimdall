use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use thiserror::Error;

/// API 层错误枚举
#[derive(Debug, Error)]
pub enum ApiError {
    #[error("请求参数校验失败: {0}")]
    BadRequest(String),

    #[error("未经授权的访问或 Token 无效")]
    Unauthorized,

    #[error("资源未找到: {0}")]
    NotFound(String),

    #[error("数据库操作错误: {0}")]
    Db(#[from] db::DbError),

    #[error("分析管线错误: {0}")]
    Pipeline(#[from] pipeline::PipelineError),

    #[error("内部服务器错误: {0}")]
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, msg) = match &self {
            Self::BadRequest(m) => (StatusCode::BAD_REQUEST, 40001, m.clone()),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, 40101, self.to_string()),
            Self::NotFound(m) => (StatusCode::NOT_FOUND, 40401, m.clone()),
            Self::Db(e) => (StatusCode::INTERNAL_SERVER_ERROR, 50001, e.to_string()),
            Self::Pipeline(e) => (StatusCode::INTERNAL_SERVER_ERROR, 50002, e.to_string()),
            Self::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, 50000, m.clone()),
        };

        let body = Json(json!({
            "code": code,
            "message": msg,
            "data": null,
            "timestamp": chrono::Utc::now().timestamp_millis(),
        }));

        (status, body).into_response()
    }
}
