use std::sync::atomic::Ordering;

use axum::extract::{FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::middleware::Next;
use axum::response::Response;

use crate::crypto::verify_jwt;
use crate::error::ApiError;
use crate::state::AppState;

/// 已通过认证的管理员用户上下文（Axum Extractor）
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub username: String,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let auth_header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|val| val.to_str().ok());

        let token = if let Some(header) = auth_header {
            if let Some(bearer) = header.strip_prefix("Bearer ") {
                bearer.trim()
            } else {
                return Err(ApiError::Unauthorized);
            }
        } else if let Some(query) = parts.uri.query() {
            let query_token = query.split('&').find_map(|pair| {
                let mut kv = pair.split('=');
                if kv.next() == Some("token") {
                    kv.next()
                } else {
                    None
                }
            });
            match query_token {
                Some(tok) if !tok.is_empty() => tok,
                _ => return Err(ApiError::AuthRequired),
            }
        } else {
            return Err(ApiError::AuthRequired);
        };

        let invalid_before = state.token_invalid_before.load(Ordering::Relaxed);
        let secret = state.get_jwt_secret();
        let claims = verify_jwt(token, &secret, invalid_before)?;

        Ok(AuthUser {
            username: claims.sub,
        })
    }
}

/// 路由级 JWT 鉴权中间件
pub async fn require_auth(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let (mut parts, body) = req.into_parts();
    let auth_user = AuthUser::from_request_parts(&mut parts, &state).await?;

    // 将解析出的 AuthUser 存入 request extensions，便于下游直接获取
    let mut req = Request::from_parts(parts, body);
    req.extensions_mut().insert(auth_user);

    Ok(next.run(req).await)
}
