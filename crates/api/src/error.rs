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

    #[error("未登录（缺少凭据）")]
    AuthRequired,

    #[error("凭据已过期")]
    TokenExpired,

    #[error("凭据无效或已被撤销")]
    TokenRevoked,

    #[error("系统已初始化，禁止重复初始化")]
    AlreadyInitialized,

    #[error("用户名或密码错误")]
    InvalidCredentials,

    #[error("新密码强度不符合要求: {0}")]
    WeakPassword(String),

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
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, 10001, self.to_string()),
            Self::AuthRequired => (StatusCode::UNAUTHORIZED, 10001, self.to_string()),
            Self::TokenExpired => (StatusCode::UNAUTHORIZED, 10002, self.to_string()),
            Self::TokenRevoked => (StatusCode::UNAUTHORIZED, 10003, self.to_string()),
            Self::AlreadyInitialized => (StatusCode::FORBIDDEN, 10006, self.to_string()),
            Self::InvalidCredentials => (StatusCode::BAD_REQUEST, 10007, self.to_string()),
            Self::WeakPassword(m) => (StatusCode::BAD_REQUEST, 10008, m.clone()),
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
