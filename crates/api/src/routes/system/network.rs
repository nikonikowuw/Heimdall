use axum::extract::State;
use axum::Json;

use crate::error::ApiError;
use crate::response::ApiResponse;
use crate::state::AppState;

/// `GET /api/v1/system/network/interfaces`
pub async fn get_network_interfaces(
    State(_state): State<AppState>,
) -> Result<ApiResponse<types::NetworkInterfacesResponse>, ApiError> {
    let response = crate::network_service::NetworkService::list_with_pending().await?;
    Ok(ApiResponse::success(response))
}

/// `GET /api/v1/system/network/interfaces/:name`
pub async fn get_network_interface(
    State(_state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Result<ApiResponse<types::NetworkInterface>, ApiError> {
    let iface = crate::network_service::NetworkService::get_interface(&name).await?;
    Ok(ApiResponse::success(iface))
}

/// `PUT /api/v1/system/network/interfaces/:name`
pub async fn update_network_interface(
    State(_state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
    Json(config): Json<types::IpConfig>,
) -> Result<ApiResponse<types::NetworkUpdateResult>, ApiError> {
    let result = crate::network_service::NetworkService::update_interface(&name, &config).await?;
    Ok(ApiResponse::success(result))
}

/// `GET /api/v1/system/network/changes/pending`
pub async fn get_pending_network_change(
    State(_state): State<AppState>,
) -> Result<ApiResponse<Option<types::NetworkChangeOperation>>, ApiError> {
    let op = crate::network_service::NetworkService::get_pending_operation().await?;
    Ok(ApiResponse::success(op))
}

/// `POST /api/v1/system/network/changes/:id/confirm`
pub async fn confirm_network_change(
    State(_state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<ApiResponse<types::OperationConfirmResult>, ApiError> {
    let result = crate::network_service::NetworkService::confirm_operation(&id).await?;
    Ok(ApiResponse::success(result))
}

/// `POST /api/v1/system/network/changes/:id/cancel`
pub async fn cancel_network_change(
    State(_state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<ApiResponse<types::OperationConfirmResult>, ApiError> {
    let result = crate::network_service::NetworkService::cancel_operation(&id).await?;
    Ok(ApiResponse::success(result))
}

/// `POST /api/v1/system/network/diagnose`
pub async fn diagnose_network(
    State(_state): State<AppState>,
    axum::Json(req): axum::Json<types::NetworkDiagnosticRequest>,
) -> Result<ApiResponse<types::NetworkDiagnosticResult>, ApiError> {
    let result = crate::network_service::NetworkService::diagnose(&req).await?;
    Ok(ApiResponse::success(result))
}

pub fn router() -> axum::Router<AppState> {
    axum::Router::new()
        .route(
            "/network/interfaces",
            axum::routing::get(get_network_interfaces),
        )
        .route(
            "/network/interfaces/{name}",
            axum::routing::get(get_network_interface).put(update_network_interface),
        )
        .route(
            "/network/changes/pending",
            axum::routing::get(get_pending_network_change),
        )
        .route(
            "/network/changes/{id}/confirm",
            axum::routing::post(confirm_network_change),
        )
        .route(
            "/network/changes/{id}/cancel",
            axum::routing::post(cancel_network_change),
        )
        .route("/network/diagnose", axum::routing::post(diagnose_network))
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "macos")]
    use super::*;
    #[cfg(target_os = "macos")]
    use axum::body::Body;
    #[cfg(target_os = "macos")]
    use axum::http::{Request, StatusCode};
    #[cfg(target_os = "macos")]
    use std::sync::Arc;
    #[cfg(target_os = "macos")]
    use tower::ServiceExt;

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn network_interfaces_returns_macos_interfaces() {
        let db = db::init_test_db().await.expect("初始化测试数据库失败");
        let pipeline = Arc::new(pipeline::PipelineManager::new());
        let app = router().with_state(AppState::new(db, pipeline));

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/network/interfaces")
                    .body(Body::empty())
                    .expect("构造请求失败"),
            )
            .await
            .expect("执行请求失败");

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("读取响应体失败");
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("解析网卡响应失败");
        assert_eq!(payload["code"], 0);
        let interfaces = payload["data"]["interfaces"]
            .as_array()
            .expect("interfaces 应为数组");
        assert!(!interfaces.is_empty());
        assert!(interfaces
            .iter()
            .any(|interface| interface["name"] == "lo0"));
    }
}
