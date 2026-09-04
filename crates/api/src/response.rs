use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

/// 根信封标准成功响应体
#[derive(Debug, Serialize)]
pub struct ApiResponse<T: Serialize> {
    pub code: i32,
    pub message: String,
    pub data: T,
    pub timestamp: i64,
}

impl<T: Serialize> ApiResponse<T> {
    pub fn success(data: T) -> Self {
        Self {
            code: 0,
            message: "success".to_string(),
            data,
            timestamp: chrono::Utc::now().timestamp_millis(),
        }
    }
}

impl<T: Serialize> IntoResponse for ApiResponse<T> {
    fn into_response(self) -> Response {
        Json(self).into_response()
    }
}
