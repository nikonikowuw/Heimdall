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

    #[error("流媒体接入与解码错误: {0}")]
    Media(#[from] media::MediaError),

    #[error("算法推理与沙箱错误: {0}")]
    Infer(#[from] infer::InferError),

    #[error("分析管线错误: {0}")]
    Pipeline(#[from] pipeline::PipelineError),

    #[error("分析运行时协调错误: {0}")]
    Coordinator(#[from] pipeline::CoordinatorError),

    #[error("内部服务器错误: {0}")]
    Internal(String),

    // ─── 系统设置 51xxx ───
    #[error("网卡不存在: {0}")]
    NetworkInterfaceNotFound(String),

    #[error("网卡不支持修改: {0}")]
    NetworkInterfaceReadOnly(String),

    #[error("已有进行中的网络操作")]
    NetworkPendingOperation,

    #[error("网络操作超时: {0}")]
    NetworkOperationTimeout(String),

    #[error("网络操作已过期，需重新提交")]
    NetworkOperationExpired,

    #[error("网络配置无效: {0}")]
    NetworkInvalid(String),

    #[error("检测到静态 IP 冲突: {0}")]
    NetworkIpConflict(String),

    #[error("网关不可达或配置冲突: {0}")]
    NetworkGatewayUnreachable(String),

    #[error("网络服务执行失败: {0}")]
    NetworkFailed(String),

    #[error("存储配置校验失败: {0}")]
    StorageConfig(String),

    #[error("时间配置校验失败: {0}")]
    TimeConfig(String),

    #[error("时间差超过一年")]
    TimeDeltaTooLarge,

    #[error("NTP 服务执行失败: {0}")]
    TimeSyncFailed(String),

    #[error("系统信息读取失败: {0}")]
    SystemInfo(String),
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
            Self::Media(e) => (StatusCode::BAD_REQUEST, e.error_code(), e.to_string()),
            Self::Infer(e) => (StatusCode::BAD_REQUEST, e.error_code(), e.to_string()),
            Self::Pipeline(e) => (StatusCode::BAD_REQUEST, e.error_code(), e.to_string()),
            Self::Coordinator(pipeline::CoordinatorError::TaskJoin(e)) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                50000,
                format!("运行时停止任务异常: {e}"),
            ),
            Self::Coordinator(pipeline::CoordinatorError::Validation { reason }) => {
                (StatusCode::BAD_REQUEST, 40001, reason.clone())
            }
            Self::Coordinator(pipeline::CoordinatorError::AlgorithmNotFound { algorithm_id }) => (
                StatusCode::NOT_FOUND,
                40401,
                format!("算法未找到: {algorithm_id}"),
            ),
            Self::Coordinator(e) => (StatusCode::BAD_REQUEST, 40001, e.to_string()),
            Self::Db(db::DbError::BuiltinAlgoProtected(m)) => {
                (StatusCode::FORBIDDEN, 40301, m.clone())
            }
            Self::Db(db::DbError::AlgoInUse(m)) => (StatusCode::CONFLICT, 40901, m.clone()),
            Self::Db(db::DbError::NotFound { entity, key }) => (
                StatusCode::NOT_FOUND,
                40401,
                format!("未找到记录: {entity} (key={key})"),
            ),
            Self::Db(e) => (StatusCode::INTERNAL_SERVER_ERROR, 50001, e.to_string()),
            Self::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, 50000, m.clone()),
            Self::NetworkInterfaceNotFound(m) => (StatusCode::NOT_FOUND, 51007, m.clone()),
            Self::NetworkInterfaceReadOnly(m) => (StatusCode::BAD_REQUEST, 51005, m.clone()),
            Self::NetworkPendingOperation => (StatusCode::CONFLICT, 51006, self.to_string()),
            Self::NetworkOperationTimeout(m) => (StatusCode::REQUEST_TIMEOUT, 51009, m.clone()),
            Self::NetworkOperationExpired => (StatusCode::CONFLICT, 51010, self.to_string()),
            Self::NetworkIpConflict(m) => (StatusCode::CONFLICT, 51011, m.clone()),
            Self::NetworkGatewayUnreachable(m) => (StatusCode::BAD_REQUEST, 51012, m.clone()),
            Self::NetworkInvalid(m) => (StatusCode::BAD_REQUEST, 51001, m.clone()),
            Self::NetworkFailed(m) => (StatusCode::INTERNAL_SERVER_ERROR, 51008, m.clone()),
            Self::StorageConfig(m) => (StatusCode::BAD_REQUEST, 51100, m.clone()),
            Self::TimeConfig(m) => (StatusCode::BAD_REQUEST, 51200, m.clone()),
            Self::TimeDeltaTooLarge => (StatusCode::BAD_REQUEST, 51201, self.to_string()),
            Self::TimeSyncFailed(m) => (StatusCode::INTERNAL_SERVER_ERROR, 51202, m.clone()),
            Self::SystemInfo(m) => (StatusCode::INTERNAL_SERVER_ERROR, 51300, m.clone()),
        };

        if status.is_server_error() {
            tracing::error!(status = %status, code = code, error = %msg, "API 内部处理异常 (5xx)");
        } else if status.is_client_error() && status != StatusCode::UNAUTHORIZED {
            tracing::warn!(status = %status, code = code, error = %msg, "API 客户端请求错误 (4xx)");
        }

        let body = Json(json!({
            "code": code,
            "message": msg,
            "data": null,
            "timestamp": chrono::Utc::now().timestamp_millis(),
        }));

        (status, body).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_error_code_mapping() {
        let infer_err = infer::InferError::SandboxValidation {
            step: "1.路径防穿透".to_string(),
            reason: "非法路径".to_string(),
        };
        assert_eq!(infer_err.error_code(), 30016);
        let api_err = ApiError::from(infer_err);
        let resp = api_err.into_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let pipeline_err = pipeline::PipelineError::PipelineNotFound {
            camera_id: "cam-1".to_string(),
        };
        assert_eq!(pipeline_err.error_code(), 30001);
        let api_err = ApiError::from(pipeline_err);
        let resp = api_err.into_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        let conflict_err = ApiError::NetworkIpConflict("192.168.1.100".to_string());
        let resp = conflict_err.into_response();
        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }
}
