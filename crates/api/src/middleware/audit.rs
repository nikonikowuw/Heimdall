use std::{
    future::Future,
    net::SocketAddr,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::Instant,
};

use axum::{
    body::Body,
    extract::OriginalUri,
    http::{header, Extensions, HeaderMap, Method, Request},
    response::Response,
};
use bytes::Bytes;
use db::DatabaseConnection;
use tokio_stream::StreamExt;
use tower::{Layer, Service};

const AUDIT_BODY_CHAR_LIMIT: usize = 2048;
const AUDIT_BODY_CAPTURE_BYTES: usize = AUDIT_BODY_CHAR_LIMIT * 4;

/// 请求 extensions 中注入的用户名标识（由 `require_auth` 中间件写入）。
#[derive(Clone, Debug)]
pub struct AuditUser(pub String);

/// 审计日志 Layer。默认只记录 POST、PUT、PATCH 和 DELETE 请求。
#[derive(Clone, Debug)]
pub struct AuditLogLayer {
    db: DatabaseConnection,
    log_writes_only: bool,
}

impl AuditLogLayer {
    /// 创建只记录写操作的审计 Layer。
    pub fn new(db: DatabaseConnection) -> Self {
        Self {
            db,
            log_writes_only: true,
        }
    }

    /// 设置是否同时记录 GET、HEAD 和 OPTIONS 等读请求。
    #[must_use]
    pub fn log_reads(mut self, enabled: bool) -> Self {
        self.log_writes_only = !enabled;
        self
    }
}

impl<S> Layer<S> for AuditLogLayer {
    type Service = AuditLogService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AuditLogService {
            inner,
            db: self.db.clone(),
            log_writes_only: self.log_writes_only,
        }
    }
}

/// 审计日志 Service，在下游 handler 完成后异步写入数据库。
#[derive(Clone, Debug)]
pub struct AuditLogService<S> {
    inner: S,
    db: DatabaseConnection,
    log_writes_only: bool,
}

impl<S> Service<Request<Body>> for AuditLogService<S>
where
    S: Service<Request<Body>, Response = Response> + Send + 'static,
    S::Future: Send + 'static,
    S::Error: Send + 'static,
{
    type Response = Response;
    type Error = S::Error;
    type Future =
        Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send + 'static>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request<Body>) -> Self::Future {
        let method = request.method().clone();
        let (path, query_string) = request_path_and_query(&request);
        let should_record = !self.log_writes_only || is_write_method(&method);
        let client_ip = extract_client_ip(request.headers(), request.extensions());
        let user_agent = extract_user_agent(request.headers());
        let username = request
            .extensions()
            .get::<AuditUser>()
            .map(|user| user.0.clone())
            .unwrap_or_default();
        let start = Instant::now();

        let captured_body = Arc::new(Mutex::new(Vec::with_capacity(AUDIT_BODY_CAPTURE_BYTES)));
        let request = if should_record
            && is_write_method(&method)
            && should_capture_body(request.headers())
        {
            let (parts, body) = request.into_parts();
            let body = tee_body(body, captured_body.clone());
            Request::from_parts(parts, body)
        } else {
            request
        };

        let future = self.inner.call(request);
        let db = self.db.clone();

        Box::pin(async move {
            let response = future.await?;

            if should_record {
                let duration_ms = start.elapsed().as_millis() as i64;
                let status_code = response.status().as_u16() as i32;
                let method_string = method.to_string();
                let (module, action) = infer_module_action(&path, &method);
                let body = body_to_string(&captured_body);

                tokio::spawn(async move {
                    if let Err(error) = db::OplogRepo::record(
                        &db,
                        &username,
                        &module,
                        &action,
                        &method_string,
                        &path,
                        &query_string,
                        &body,
                        status_code,
                        duration_ms,
                        &client_ip,
                        &user_agent,
                    )
                    .await
                    {
                        tracing::debug!(error = ?error, module = %module, action = %action, "审计日志写入失败");
                    }
                });
            }

            Ok(response)
        })
    }
}

fn tee_body(body: Body, captured: Arc<Mutex<Vec<u8>>>) -> Body {
    let mut stream = body.into_data_stream();
    let stream = async_stream::stream! {
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(chunk) => {
                    capture_body_bytes(&captured, &chunk);
                    yield Ok::<Bytes, std::io::Error>(chunk);
                }
                Err(error) => {
                    yield Err::<Bytes, std::io::Error>(std::io::Error::other(error.to_string()));
                }
            }
        }
    };
    Body::from_stream(stream)
}

fn capture_body_bytes(captured: &Mutex<Vec<u8>>, chunk: &Bytes) {
    let Ok(mut stored) = captured.lock() else {
        return;
    };

    let remaining = AUDIT_BODY_CAPTURE_BYTES.saturating_sub(stored.len());
    if remaining > 0 {
        stored.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
    }
}

fn body_to_string(captured: &Mutex<Vec<u8>>) -> String {
    let bytes = match captured.lock() {
        Ok(stored) => stored.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    };

    String::from_utf8_lossy(&bytes)
        .chars()
        .take(AUDIT_BODY_CHAR_LIMIT)
        .collect()
}

fn should_capture_body(headers: &HeaderMap) -> bool {
    let Some(content_type) = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
    else {
        return true;
    };

    let content_type = content_type.to_ascii_lowercase();
    !content_type.starts_with("multipart/") && !content_type.starts_with("application/octet-stream")
}

fn is_write_method(method: &Method) -> bool {
    matches!(
        *method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    )
}

fn request_path_and_query(request: &Request<Body>) -> (String, String) {
    let uri = request
        .extensions()
        .get::<OriginalUri>()
        .map(|original| &original.0)
        .unwrap_or_else(|| request.uri());
    (
        uri.path().to_string(),
        uri.query().unwrap_or_default().to_string(),
    )
}

/// 从请求 headers + extensions 中解析客户端真实 IP。
///
/// 优先级：X-Forwarded-For（第一个） → X-Real-IP → ConnectInfo → "unknown"。
/// 代理头由部署侧反向代理负责写入，保留第一个地址以兼容多级代理链。
pub fn extract_client_ip(headers: &HeaderMap, extensions: &Extensions) -> String {
    if let Some(value) = headers.get("x-forwarded-for") {
        if let Ok(value) = value.to_str() {
            if let Some(first) = value.split(',').map(str::trim).find(|ip| !ip.is_empty()) {
                return first.to_string();
            }
        }
    }

    if let Some(value) = headers.get("x-real-ip") {
        if let Ok(value) = value.to_str() {
            let value = value.trim();
            if !value.is_empty() {
                return value.to_string();
            }
        }
    }

    extensions
        .get::<axum::extract::connect_info::ConnectInfo<SocketAddr>>()
        .map(|connect_info| connect_info.0.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// 从请求 headers 中提取 User-Agent。
pub fn extract_user_agent(headers: &HeaderMap) -> String {
    headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string()
}

fn method_action(method: &Method) -> &'static str {
    match *method {
        Method::POST => "create",
        Method::PUT | Method::PATCH => "update",
        Method::DELETE => "delete",
        _ => "unknown",
    }
}

/// 从 path + method 自动推断 module 和 action。
fn infer_module_action(path: &str, method: &Method) -> (String, String) {
    let path = path.strip_prefix("/api/v1").unwrap_or(path);
    let segments: Vec<&str> = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let generic_action = method_action(method);

    let (module, action) = match segments.as_slice() {
        ["cameras", "deduce-substream"] => ("camera", "deduce_substream"),
        ["cameras", _, "probe"] => ("camera", "probe"),
        ["cameras", ..] => ("camera", generic_action),
        ["tasks", "instances", ..] => {
            let action = if segments.last() == Some(&"enabled") {
                "update_enabled"
            } else {
                generic_action
            };
            ("task_instance", action)
        }
        ["tasks", ..] if matches!(*method, Method::PUT) => ("task", "update_rules"),
        ["tasks", ..] => ("task", generic_action),
        ["algorithms", "upload"] => ("algorithm", "upload"),
        ["algorithms", _, "versions", _, "activate"] => ("algorithm", "activate"),
        ["algorithms", _, "versions", _] if matches!(*method, Method::DELETE) => {
            ("algorithm", "uninstall")
        }
        ["algorithms", ..] => ("algorithm", generic_action),
        ["alarms", _, "status"] => ("alarm", "update_status"),
        ["evidence", ..] => ("evidence", generic_action),
        ["system", "network", "changes", _, "confirm"] => ("system", "confirm_network_change"),
        ["system", "network", "changes", _, "cancel"] => ("system", "cancel_network_change"),
        ["system", "storage", "cleanup"] => ("system", "cleanup_storage"),
        ["system", "time", "sync"] => ("system", "sync_time"),
        ["system", "time", "set"] => ("system", "set_time"),
        ["system", ..] => ("system", generic_action),
        [module, ..] => (*module, generic_action),
        [] => ("unknown", generic_action),
    };

    (module.to_string(), action.to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        routing::post,
        Extension, Router,
    };
    use tower::ServiceExt;

    #[test]
    fn client_ip_uses_proxy_headers_before_connect_info() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-forwarded-for",
            "198.51.100.10, 10.0.0.1".parse().unwrap(),
        );
        let mut extensions = Extensions::new();
        extensions.insert(axum::extract::connect_info::ConnectInfo(SocketAddr::from(
            ([127, 0, 0, 1], 8080),
        )));

        assert_eq!(extract_client_ip(&headers, &extensions), "198.51.100.10");

        headers.remove("x-forwarded-for");
        headers.insert("x-real-ip", "203.0.113.20".parse().unwrap());
        assert_eq!(extract_client_ip(&headers, &extensions), "203.0.113.20");

        headers.remove("x-real-ip");
        assert_eq!(extract_client_ip(&headers, &extensions), "127.0.0.1");
    }

    #[test]
    fn audit_prefers_original_uri_for_nested_routes() {
        let mut request = Request::new(Body::empty());
        request.extensions_mut().insert(OriginalUri(
            "/api/v1/cameras/cam-1?source=test".parse().unwrap(),
        ));

        assert_eq!(
            request_path_and_query(&request),
            (
                "/api/v1/cameras/cam-1".to_string(),
                "source=test".to_string()
            )
        );
    }

    #[test]
    fn module_and_action_mapping_handles_protected_write_routes() {
        assert_eq!(
            infer_module_action("/api/v1/cameras/cam-1", &Method::PUT),
            ("camera".to_string(), "update".to_string())
        );
        assert_eq!(
            infer_module_action("/api/v1/tasks/cam-1", &Method::PUT),
            ("task".to_string(), "update_rules".to_string())
        );
        assert_eq!(
            infer_module_action(
                "/api/v1/algorithms/demo/versions/1.0.0/activate",
                &Method::PUT
            ),
            ("algorithm".to_string(), "activate".to_string())
        );
        assert_eq!(
            infer_module_action("/api/v1/alarms/7/status", &Method::PUT),
            ("alarm".to_string(), "update_status".to_string())
        );
    }

    #[tokio::test]
    async fn layer_records_request_metadata_and_capped_body() {
        let db = db::init_test_db().await.unwrap();
        let protected = Router::new()
            .route(
                "/cameras",
                post(|body: Bytes| async move { (StatusCode::CREATED, body) }),
            )
            .route_layer(AuditLogLayer::new(db.clone()))
            .layer(Extension(AuditUser("tester".to_string())));
        let app = Router::new().nest("/api/v1", protected);
        let payload = "x".repeat(AUDIT_BODY_CAPTURE_BYTES + 100);
        let req = Request::builder()
            .method(Method::POST)
            .uri("/api/v1/cameras?source=test")
            .header("x-forwarded-for", "203.0.113.8")
            .header("user-agent", "audit-test")
            .header("content-type", "text/plain")
            .body(Body::from(payload.clone()))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let response_body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(response_body.len(), payload.len());

        for _ in 0..100 {
            let logs = db::OplogRepo::list_recent(&db, Some("camera"), 10, 0)
                .await
                .unwrap();
            if let Some(log) = logs.first() {
                assert_eq!(log.username, "tester");
                assert_eq!(log.path, "/api/v1/cameras");
                assert_eq!(log.query, "source=test");
                assert_eq!(log.status_code, 201);
                assert_eq!(log.ip, "203.0.113.8");
                assert_eq!(log.user_agent, "audit-test");
                assert_eq!(log.body.chars().count(), AUDIT_BODY_CHAR_LIMIT);
                assert!(log.duration_ms >= 0);
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }

        panic!("审计日志未在限定时间内写入");
    }
}
