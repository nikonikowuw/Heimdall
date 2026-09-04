use axum::body::Body;
use axum::extract::Request;
use axum::http::header::{CONTENT_LANGUAGE, CONTENT_TYPE};
use axum::http::HeaderValue;
use axum::middleware::Next;
use axum::response::Response;

use crate::i18n::{localize_api_message, Locale};

/// 全局响应国际化拦截中间件
pub async fn i18n_response_middleware(req: Request, next: Next) -> Response {
    let accept_lang = req
        .headers()
        .get(axum::http::header::ACCEPT_LANGUAGE)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string());
    let locale = Locale::from_accept_language(accept_lang.as_deref());

    let res = next.run(req).await;

    // 仅针对 JSON 格式响应处理
    let is_json = res
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|ct| ct.to_str().ok())
        .map(|ct| ct.contains("application/json"))
        .unwrap_or(false);

    if !is_json {
        return res;
    }

    let (mut parts, body) = res.into_parts();
    // 限制最大拦截缓冲为 2MB，防止恶意请求导致无界内存耗尽
    const MAX_I18N_BODY_SIZE: usize = 2 * 1024 * 1024;
    let bytes = match axum::body::to_bytes(body, MAX_I18N_BODY_SIZE).await {
        Ok(b) => b,
        Err(_) => return Response::from_parts(parts, Body::empty()),
    };

    if let Ok(mut json) = serde_json::from_slice::<serde_json::Value>(&bytes) {
        if let (Some(code_val), Some(msg_val)) = (json.get("code"), json.get("message")) {
            if let (Some(code), Some(msg)) = (code_val.as_u64(), msg_val.as_str()) {
                let localized = localize_api_message(code as u32, msg, locale);
                json["message"] = serde_json::Value::String(localized);

                let new_bytes = serde_json::to_vec(&json).unwrap_or_else(|_| bytes.to_vec());
                parts
                    .headers
                    .insert(CONTENT_LANGUAGE, HeaderValue::from_static(locale.as_str()));
                // 更新 Content-Length 保证报文长度严格一致，避免客户端读取截断
                parts.headers.insert(
                    axum::http::header::CONTENT_LENGTH,
                    HeaderValue::from(new_bytes.len()),
                );
                return Response::from_parts(parts, Body::from(new_bytes));
            }
        }
    }

    Response::from_parts(parts, Body::from(bytes))
}
