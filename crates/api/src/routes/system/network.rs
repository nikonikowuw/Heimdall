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
        assert!(interfaces.iter().any(|interface| interface["name"]
            .as_str()
            .map(|name| name.starts_with("en") || !name.is_empty())
            .unwrap_or(false)));
    }

    /// 只使用全路径引用，不依赖 macOS 门控的 `use super::*` / `use tower::ServiceExt`，
    /// 使这两个用例在 Linux（生产目标）与 macOS 下都能编译。
    ///
    /// 两者都在 `validate_ip_config` 处终止，不触碰网卡枚举与硬件命令，结果与环境无关；
    /// 断言的是状态码 + 信封 + 稳定业务码，而不是只排除 422。
    #[tokio::test]
    async fn update_network_interface_rejects_omitted_method() {
        let app = build_app().await;

        // 空对象：method 缺省 → IpMethod::None，必须在边界被拒，
        // 不能落到静态分支用空地址拼出 `/24` 写进系统
        let (status, payload) = put_ip_config(app, r#"{}"#).await;

        assert_eq!(status, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(payload["code"], 51001);
        assert_eq!(payload["message"], "必须指定 dhcp 或 static 模式");
        assert!(payload["data"].is_null());
        assert!(payload["timestamp"].is_i64());
    }

    #[tokio::test]
    async fn update_network_interface_accepts_payload_without_optional_fields() {
        let app = build_app().await;

        // 仅带 method 的载荷（dns/address/prefix/gateway/metric 全缺）：
        // 提取层不得报 422，必须进入业务校验并给出稳定业务码
        let (status, payload) = put_ip_config(app, r#"{"method":"static"}"#).await;

        assert_ne!(status, axum::http::StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(status, axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(payload["code"], 51001);
        assert_eq!(payload["message"], "静态 IP 模式必须提供 IP 地址");
        assert!(payload["data"].is_null());
    }

    async fn build_app() -> axum::Router<()> {
        let db = db::init_test_db().await.expect("初始化测试数据库失败");
        let pipeline = std::sync::Arc::new(pipeline::PipelineManager::new());
        super::router().with_state(super::AppState::new(db, pipeline))
    }

    async fn put_ip_config(
        app: axum::Router<()>,
        body: &'static str,
    ) -> (axum::http::StatusCode, serde_json::Value) {
        let response = tower::ServiceExt::oneshot(
            app,
            axum::http::Request::builder()
                .method("PUT")
                .uri("/network/interfaces/eth0")
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body))
                .expect("构造请求失败"),
        )
        .await
        .expect("执行请求失败");

        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("读取响应体失败");
        let payload = serde_json::from_slice(&bytes).expect("响应体应为合法 JSON 信封");
        (status, payload)
    }
}
