use axum::http::{header, HeaderMap, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../web/dist/"]
struct Assets;

/// 静态资源长期缓存（文件名含 content hash）
const CACHE_IMMUTABLE: &str = "public, max-age=31536000, immutable";
/// SPA 入口不缓存，确保每次拿到最新版本
const NO_CACHE: &str = "no-cache";

fn build_headers(path: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let content_type = match HeaderValue::from_str(mime.as_ref()) {
        Ok(value) => value,
        Err(_) => HeaderValue::from_static("application/octet-stream"),
    };
    headers.insert(header::CONTENT_TYPE, content_type);

    // assets/ 下的哈希文件长期缓存，其余（index.html 等）不缓存
    let cache = if path.starts_with("assets/") {
        CACHE_IMMUTABLE
    } else {
        NO_CACHE
    };
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    headers
}

/// 前端 SPA 静态资源处理器
pub async fn static_handler(uri: Uri) -> Response {
    let mut path = uri.path().trim_start_matches('/').to_string();

    if path.is_empty() {
        path = "index.html".to_string();
    }

    match Assets::get(&path) {
        Some(content) => (StatusCode::OK, build_headers(&path), content.data).into_response(),
        None => {
            // SPA 模式：未命中直接回退到 index.html
            match Assets::get("index.html") {
                Some(content) => {
                    (StatusCode::OK, build_headers("index.html"), content.data).into_response()
                }
                None => (StatusCode::NOT_FOUND, "404 Not Found").into_response(),
            }
        }
    }
}
