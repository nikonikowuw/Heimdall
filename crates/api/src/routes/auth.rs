use std::sync::atomic::Ordering;

use axum::extract::{Json, State};
use axum::http::{Extensions, HeaderMap};
use axum::routing::{get, post, put};
use axum::Router;

use crate::crypto::{
    generate_jwt, hash_password_async, mask_sensitive_json, needs_rehash,
    verify_dummy_password_async, verify_password_async,
};
use crate::error::ApiError;
use crate::middleware::audit::{extract_client_ip, extract_user_agent};
use crate::middleware::AuthUser;
use crate::response::ApiResponse;
use crate::state::AppState;
use types::{
    AdminUserDto, AuthClaims, ChangePasswordRequest, InitStatusResponse, InitializeRequest,
    LoginRequest, LoginResponse,
};

/// 登录与初始化接口共用的 JSON 请求体上限（仅用户名与密码两个短字段）
const AUTH_BODY_LIMIT_BYTES: usize = 4096;

/// 最短密码长度，前端与后端同源
const MIN_PASSWORD_LENGTH: usize = 6;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/init-status", get(get_init_status))
        .route("/initialize", post(initialize))
        .route("/login", post(login))
        .route("/password", put(change_password))
        .route("/me", get(get_me))
        .route("/logout", post(logout))
        // 登录/初始化只接受两个短字符串。显式收紧 body 上限，避免匿名请求
        // 在解析阶段就占满默认配额；该层只覆盖本 router 内的路由。
        .layer(axum::extract::DefaultBodyLimit::max(AUTH_BODY_LIMIT_BYTES))
}

/// 启动时从数据库同步初始化状态、失效时间戳以及持久化 JWT Secret
pub async fn sync_auth_state(state: &AppState) {
    // 同步并持久化 JWT Secret（如果未通过环境变量注入）
    if std::env::var("HEIMDALL_JWT_SECRET")
        .map(|s| s.trim().is_empty())
        .unwrap_or(true)
    {
        if let Ok(persisted_secret) =
            db::SystemConfigRepo::get_or_set_with(&state.db, "jwt_secret", || {
                let u1 = uuid::Uuid::new_v4();
                let u2 = uuid::Uuid::new_v4();
                format!("{u1}{u2}")
            })
            .await
        {
            state.set_jwt_secret(persisted_secret.into_bytes());
        }
    }

    if let Ok(count) = db::AdminUserRepo::count(&state.db).await {
        let initialized = count > 0;
        state.is_initialized.store(initialized, Ordering::Relaxed);
        if initialized {
            if let Ok(Some(first_admin)) = db::AdminUserRepo::get_first_admin(&state.db).await {
                state
                    .token_invalid_before
                    .store(first_admin.token_invalid_before, Ordering::Relaxed);
            }
        }
    }
}

/// 统一签发 HS256 JWT Token 辅助函数
fn issue_token(username: &str, secret: &[u8], ttl_ms: i64) -> Result<(String, i64), ApiError> {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let expires_at = now_ms.saturating_add(ttl_ms);

    let claims = AuthClaims {
        sub: username.to_string(),
        iat: now_ms,
        exp: expires_at,
    };

    let access_token = generate_jwt(&claims, secret)?;
    Ok((access_token, expires_at))
}

/// 取签名密钥；密钥不可用时返回 500 而不是继续用空密钥签发。
fn require_jwt_secret(state: &AppState) -> Result<std::sync::Arc<[u8]>, ApiError> {
    state
        .get_jwt_secret()
        .ok_or_else(|| ApiError::Internal("凭据签名密钥不可用".to_string()))
}

fn request_metadata(headers: &HeaderMap, extensions: &Extensions) -> (String, String) {
    (
        extract_client_ip(headers, extensions),
        extract_user_agent(headers),
    )
}

/// 查询系统初始化状态
async fn get_init_status(
    State(state): State<AppState>,
) -> Result<ApiResponse<InitStatusResponse>, ApiError> {
    let initialized = state.is_initialized.load(Ordering::Relaxed);
    Ok(ApiResponse::success(InitStatusResponse { initialized }))
}

/// 开箱首次初始化管理员账号与密码
async fn initialize(
    State(state): State<AppState>,
    headers: HeaderMap,
    extensions: Extensions,
    Json(req): Json<InitializeRequest>,
) -> Result<ApiResponse<LoginResponse>, ApiError> {
    let start = std::time::Instant::now();
    if state.is_initialized.load(Ordering::Relaxed)
        || db::AdminUserRepo::is_initialized(&state.db).await?
    {
        return Err(ApiError::AlreadyInitialized);
    }

    let username = req.username.trim();
    if username.is_empty() {
        return Err(ApiError::BadRequest("管理员用户名不能为空".to_string()));
    }

    if req.password.chars().count() < MIN_PASSWORD_LENGTH {
        return Err(ApiError::WeakPassword(
            "初始密码长度不能少于 6 位".to_string(),
        ));
    }

    let password_hash = hash_password_async(req.password.clone()).await;
    db::AdminUserRepo::create_admin(&state.db, username, &password_hash).await?;

    state.is_initialized.store(true, Ordering::Relaxed);

    let secret = require_jwt_secret(&state)?;
    let (access_token, expires_at) = issue_token(username, &secret, state.token_ttl_ms)?;

    // 记录开箱初始化审计日志（密码脱敏）
    let mut body_json = serde_json::to_value(&req).unwrap_or_default();
    mask_sensitive_json(&mut body_json);
    let (client_ip, user_agent) = request_metadata(&headers, &extensions);
    let _ = db::OplogRepo::record(
        &state.db,
        username,
        "auth",
        "initialize",
        "POST",
        "/api/v1/auth/initialize",
        "",
        &body_json.to_string(),
        200,
        start.elapsed().as_millis() as i64,
        &client_ip,
        &user_agent,
    )
    .await;

    Ok(ApiResponse::success(LoginResponse {
        access_token,
        username: username.to_string(),
        expires_at,
    }))
}

/// 管理员登录
async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    extensions: Extensions,
    Json(req): Json<LoginRequest>,
) -> Result<ApiResponse<LoginResponse>, ApiError> {
    let start = std::time::Instant::now();
    let username = req.username.trim();
    let (client_ip, user_agent) = request_metadata(&headers, &extensions);

    if !state.is_initialized.load(Ordering::Relaxed) {
        return Err(ApiError::InvalidCredentials);
    }

    // 限流必须先于一切昂贵计算：它的价值就在于别让攻击者消耗到 PBKDF2。
    // 账号键统一小写，防止大小写变体把单个账号的额度摊成多份。
    let limiter_key = username.to_lowercase();
    if let Some(retry_after) = state.login_limiter.check(&limiter_key, &client_ip) {
        // 此分支刻意不写审计表：一旦进入退避，后续请求都会被拦在这里，
        // 逐次落库等于给攻击者提供了「零成本磁盘放大」入口。
        tracing::warn!(
            ip = %client_ip,
            retry_after_ms = retry_after.as_millis() as i64,
            "登录失败次数超限，已拒绝本次尝试"
        );
        return Err(ApiError::TooManyAttempts {
            retry_after_ms: retry_after.as_millis().max(1000) as i64,
        });
    }

    let user = db::AdminUserRepo::find_by_username(&state.db, username).await?;

    // 用户不存在时仍需跑一次等价耗时的哈希校验，否则响应耗时差会暴露该用户名是否存在。
    let verified = match &user {
        Some(user) => verify_password_async(req.password.clone(), user.password_hash.clone()).await,
        None => {
            verify_dummy_password_async(req.password.clone()).await;
            false
        }
    };

    if !verified {
        let outcome = state.login_limiter.record_failure(&limiter_key, &client_ip);
        if let crate::login_limiter::FailureOutcome::Locked {
            failures,
            retry_after,
        } = outcome
        {
            tracing::warn!(
                ip = %client_ip,
                failures,
                retry_after_ms = retry_after.as_millis() as i64,
                "登录连续失败达到阈值，已启动退避"
            );
        }

        // 失败审计条数受限流阈值封顶，不会无界增长；密码同样经脱敏。
        let mut body_json = serde_json::to_value(&req).unwrap_or_default();
        mask_sensitive_json(&mut body_json);
        let _ = db::OplogRepo::record(
            &state.db,
            username,
            "auth",
            "login_failed",
            "POST",
            "/api/v1/auth/login",
            "",
            &body_json.to_string(),
            400,
            start.elapsed().as_millis() as i64,
            &client_ip,
            &user_agent,
        )
        .await;

        return Err(ApiError::InvalidCredentials);
    }

    let user = user.expect("verified 为真时用户必然存在");
    state.login_limiter.record_success(&limiter_key);

    // 密码校验成功后才拿到明文，因此迭代数升级只能在登录成功后被动完成：
    // 历史哈希仍可登录，不会被强制重置，但会在首次登录后自动抿平到当前强度。
    if needs_rehash(&user.password_hash) {
        let upgraded = hash_password_async(req.password.clone()).await;
        match db::AdminUserRepo::update_password_hash(&state.db, &user.username, &upgraded).await {
            Ok(()) => tracing::info!(
                username = %user.username,
                "已按当前迭代数升级密码哈希"
            ),
            // 哈希升级失败不能阻断登录：用户凭据本身是有效的。
            Err(error) => tracing::warn!(error = %error, "密码哈希强度升级失败，保留原哈希"),
        }
    }

    let secret = require_jwt_secret(&state)?;
    let (access_token, expires_at) = issue_token(&user.username, &secret, state.token_ttl_ms)?;

    // 记录登录审计日志（密码脱敏）
    let mut body_json = serde_json::to_value(&req).unwrap_or_default();
    mask_sensitive_json(&mut body_json);
    let _ = db::OplogRepo::record(
        &state.db,
        &user.username,
        "auth",
        "login",
        "POST",
        "/api/v1/auth/login",
        "",
        &body_json.to_string(),
        200,
        start.elapsed().as_millis() as i64,
        &client_ip,
        &user_agent,
    )
    .await;

    Ok(ApiResponse::success(LoginResponse {
        access_token,
        username: user.username,
        expires_at,
    }))
}

/// 修改管理员密码
async fn change_password(
    auth: AuthUser,
    State(state): State<AppState>,
    headers: HeaderMap,
    extensions: Extensions,
    Json(req): Json<ChangePasswordRequest>,
) -> Result<ApiResponse<AdminUserDto>, ApiError> {
    let start = std::time::Instant::now();
    if req.new_password.chars().count() < MIN_PASSWORD_LENGTH {
        return Err(ApiError::WeakPassword(
            "新密码长度不能少于 6 位".to_string(),
        ));
    }

    let user = db::AdminUserRepo::find_by_username(&state.db, &auth.username)
        .await?
        .ok_or(ApiError::Unauthorized)?;

    if !verify_password_async(req.old_password.clone(), user.password_hash.clone()).await {
        return Err(ApiError::InvalidCredentials);
    }

    let new_hash = hash_password_async(req.new_password.clone()).await;
    let now_ms = chrono::Utc::now().timestamp_millis();

    db::AdminUserRepo::update_password(&state.db, &auth.username, &new_hash, now_ms).await?;
    state.token_invalid_before.store(now_ms, Ordering::Relaxed);

    // 记录改密审计日志（新旧密码均必须严格脱敏为 ******）
    let mut body_json = serde_json::to_value(&req).unwrap_or_default();
    mask_sensitive_json(&mut body_json);
    let (client_ip, user_agent) = request_metadata(&headers, &extensions);
    let _ = db::OplogRepo::record(
        &state.db,
        &auth.username,
        "auth",
        "change_password",
        "PUT",
        "/api/v1/auth/password",
        "",
        &body_json.to_string(),
        200,
        start.elapsed().as_millis() as i64,
        &client_ip,
        &user_agent,
    )
    .await;

    Ok(ApiResponse::success(AdminUserDto {
        username: auth.username,
        created_at: user.created_at.timestamp_millis(),
        updated_at: now_ms,
    }))
}

/// 获取当前登录管理员信息
async fn get_me(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<ApiResponse<AdminUserDto>, ApiError> {
    let user = db::AdminUserRepo::find_by_username(&state.db, &auth.username)
        .await?
        .ok_or(ApiError::Unauthorized)?;

    Ok(ApiResponse::success(AdminUserDto {
        username: user.username,
        created_at: user.created_at.timestamp_millis(),
        updated_at: user.updated_at.timestamp_millis(),
    }))
}

/// 管理员登出
async fn logout(
    auth: AuthUser,
    State(state): State<AppState>,
    headers: HeaderMap,
    extensions: Extensions,
) -> Result<ApiResponse<()>, ApiError> {
    let start = std::time::Instant::now();
    let now_ms = chrono::Utc::now().timestamp_millis();
    db::AdminUserRepo::invalidate_tokens(&state.db, &auth.username, now_ms).await?;
    state.token_invalid_before.store(now_ms, Ordering::Relaxed);

    // 记录登出审计日志
    let (client_ip, user_agent) = request_metadata(&headers, &extensions);
    let _ = db::OplogRepo::record(
        &state.db,
        &auth.username,
        "auth",
        "logout",
        "POST",
        "/api/v1/auth/logout",
        "",
        "",
        200,
        start.elapsed().as_millis() as i64,
        &client_ip,
        &user_agent,
    )
    .await;

    Ok(ApiResponse::success(()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    async fn setup_test_app() -> (axum::Router, AppState) {
        let db = db::init_test_db().await.unwrap();
        let pipeline = std::sync::Arc::new(pipeline::PipelineManager::new());
        let state = AppState::new(db, pipeline);
        crate::sync_auth_state(&state).await;
        let app = crate::create_app(state.clone());
        (app, state)
    }

    /// 走完开箱初始化并返回响应体中的 accessToken。
    async fn initialize_and_get_token(app: &axum::Router) -> String {
        let req = Request::builder()
            .uri("/api/v1/auth/initialize")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&InitializeRequest {
                    username: "admin".to_string(),
                    password: "myStrongPassword2026".to_string(),
                })
                .unwrap(),
            ))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        body["data"]["accessToken"].as_str().unwrap().to_string()
    }

    /// 从 JWT 载荷里解出 `exp`，与响应体的 `expiresAt` 交叉验证。
    fn decode_jwt_exp(token: &str) -> i64 {
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use base64::Engine;

        let payload = token.split('.').nth(1).unwrap();
        let decoded = URL_SAFE_NO_PAD.decode(payload).unwrap();
        let claims: types::AuthClaims = serde_json::from_slice(&decoded).unwrap();
        claims.exp
    }

    #[tokio::test]
    async fn test_token_ttl_is_taken_from_app_state() {
        let db = db::init_test_db().await.unwrap();
        let pipeline = std::sync::Arc::new(pipeline::PipelineManager::new());
        // 刻意用非默认值：默认就是 24h，只有换个值才能证明它真的可配。
        let ttl_ms: i64 = 90 * 60 * 1000;
        let state = AppState::new(db, pipeline).with_token_ttl_ms(ttl_ms);
        crate::sync_auth_state(&state).await;
        let app = crate::create_app(state.clone());

        let token = initialize_and_get_token(&app).await;
        let exp = decode_jwt_exp(&token);
        let issued_at = exp - ttl_ms;

        // exp 必须落在「签发时刻 + ttl」附近（初始化本身耗时可忽略，留 30s 余量）。
        let drift_ms = (chrono::Utc::now().timestamp_millis() - issued_at).abs();
        assert!(
            drift_ms < 30_000,
            "exp 与配置的 TTL 不符：漂移 {drift_ms}ms，期望约等于 {ttl_ms}ms"
        );

        // 签名与撤销时间戳校验也要接受该凭据，证明 TTL 未破坏校验链路。
        let secret = state.get_jwt_secret().unwrap();
        let invalid_before = state.token_invalid_before.load(Ordering::Relaxed);
        let claims = crate::crypto::verify_jwt(&token, &secret, invalid_before).unwrap();
        assert_eq!(claims.exp, exp);
    }

    #[tokio::test]
    async fn test_login_response_expires_at_matches_configured_ttl() {
        let db = db::init_test_db().await.unwrap();
        let pipeline = std::sync::Arc::new(pipeline::PipelineManager::new());
        let ttl_ms: i64 = 45 * 60 * 1000;
        let state = AppState::new(db, pipeline).with_token_ttl_ms(ttl_ms);
        crate::sync_auth_state(&state).await;
        let app = crate::create_app(state.clone());

        initialize_and_get_token(&app).await;

        let req = Request::builder()
            .uri("/api/v1/auth/login")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&LoginRequest {
                    username: "admin".to_string(),
                    password: "myStrongPassword2026".to_string(),
                })
                .unwrap(),
            ))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        let expires_at = body["data"]["expiresAt"].as_i64().unwrap();
        let token = body["data"]["accessToken"].as_str().unwrap();

        // 响应体的 expiresAt 必须等于 Token 载荷里的 exp，且等于签发时刻 + TTL。
        assert_eq!(expires_at, decode_jwt_exp(token));
        let drift_ms = (chrono::Utc::now().timestamp_millis() - (expires_at - ttl_ms)).abs();
        assert!(
            drift_ms < 30_000,
            "expiresAt 与配置的 TTL 不符：漂移 {drift_ms}ms，期望约等于 {ttl_ms}ms"
        );
    }

    #[tokio::test]
    async fn test_full_auth_lifecycle() {
        let (app, state) = setup_test_app().await;

        // 1. 检查未初始化状态
        let req = Request::builder()
            .uri("/api/v1/auth/init-status")
            .method("GET")
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(val["code"], 0);
        assert_eq!(val["data"]["initialized"], false);

        // 2. 尝试登录（未初始化直接拒绝）
        let req = Request::builder()
            .uri("/api/v1/auth/login")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&LoginRequest {
                    username: "admin".to_string(),
                    password: "password123".to_string(),
                })
                .unwrap(),
            ))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);

        // 3. 执行首次开箱初始化
        let req = Request::builder()
            .uri("/api/v1/auth/initialize")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&InitializeRequest {
                    username: "root_admin".to_string(),
                    password: "myStrongPassword2026".to_string(),
                })
                .unwrap(),
            ))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let init_res: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(init_res["code"], 0);
        assert_eq!(init_res["data"]["username"], "root_admin");
        let initial_token = init_res["data"]["accessToken"]
            .as_str()
            .unwrap()
            .to_string();

        // 4. 重复初始化必须被拒绝 (403)
        let req = Request::builder()
            .uri("/api/v1/auth/initialize")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&InitializeRequest {
                    username: "hacker".to_string(),
                    password: "hackerpassword".to_string(),
                })
                .unwrap(),
            ))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN);

        // 5. 访问 /api/v1/auth/me (无 Token 返回 401)
        let req = Request::builder()
            .uri("/api/v1/auth/me")
            .method("GET")
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

        // 6. 访问 /api/v1/auth/me (带有效 Token 返回 200)
        let req = Request::builder()
            .uri("/api/v1/auth/me")
            .method("GET")
            .header("authorization", format!("Bearer {initial_token}"))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let me_res: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(me_res["data"]["username"], "root_admin");

        // 7. 测试受保护的业务接口 /api/v1/cameras (无 Token 自动被中间件拦截 401)
        let req = Request::builder()
            .uri("/api/v1/cameras")
            .method("GET")
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

        // 8. 测试登录接口 (错误密码返回 400, 正确密码返回 200)
        let req = Request::builder()
            .uri("/api/v1/auth/login")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&LoginRequest {
                    username: "root_admin".to_string(),
                    password: "wrong_pwd".to_string(),
                })
                .unwrap(),
            ))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);

        let req = Request::builder()
            .uri("/api/v1/auth/login")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&LoginRequest {
                    username: "root_admin".to_string(),
                    password: "myStrongPassword2026".to_string(),
                })
                .unwrap(),
            ))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let login_res: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let token2 = login_res["data"]["accessToken"]
            .as_str()
            .unwrap()
            .to_string();

        // 9. 修改密码
        // 确保时间戳发生递增
        tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
        let req = Request::builder()
            .uri("/api/v1/auth/password")
            .method("PUT")
            .header("authorization", format!("Bearer {token2}"))
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&ChangePasswordRequest {
                    old_password: "myStrongPassword2026".to_string(),
                    new_password: "brandNewPassword2027!".to_string(),
                })
                .unwrap(),
            ))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        // 10. 改密后，旧 Token (initial_token 和 token2) 访问接口应全部返回 401 撤销
        let req = Request::builder()
            .uri("/api/v1/auth/me")
            .method("GET")
            .header("authorization", format!("Bearer {token2}"))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

        // 11. 用新密码登录并签发新 Token
        let req = Request::builder()
            .uri("/api/v1/auth/login")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&LoginRequest {
                    username: "root_admin".to_string(),
                    password: "brandNewPassword2027!".to_string(),
                })
                .unwrap(),
            ))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let new_login_res: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let token3 = new_login_res["data"]["accessToken"]
            .as_str()
            .unwrap()
            .to_string();

        // 12. 执行登出
        tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
        let req = Request::builder()
            .uri("/api/v1/auth/logout")
            .method("POST")
            .header("authorization", format!("Bearer {token3}"))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        // 13. 登出后 token3 立即失效
        let req = Request::builder()
            .uri("/api/v1/auth/me")
            .method("GET")
            .header("authorization", format!("Bearer {token3}"))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

        // 14. 验证 Accept-Language 响应国际化支持 (English & 繁體中文)
        let req_en = Request::builder()
            .uri("/api/v1/auth/login")
            .method("POST")
            .header("content-type", "application/json")
            .header("accept-language", "en-US,en;q=0.9")
            .body(Body::from(
                serde_json::to_vec(&LoginRequest {
                    username: "root_admin".to_string(),
                    password: "wrong_password".to_string(),
                })
                .unwrap(),
            ))
            .unwrap();
        let res_en = app.clone().oneshot(req_en).await.unwrap();
        assert_eq!(res_en.status(), StatusCode::BAD_REQUEST);
        assert_eq!(res_en.headers().get("content-language").unwrap(), "en");
        let bytes = axum::body::to_bytes(res_en.into_body(), usize::MAX)
            .await
            .unwrap();
        let val_en: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(val_en["code"], 10007);
        assert_eq!(val_en["message"], "Invalid username or password");

        let req_tw = Request::builder()
            .uri("/api/v1/auth/login")
            .method("POST")
            .header("content-type", "application/json")
            .header("accept-language", "zh-TW,zh;q=0.9")
            .body(Body::from(
                serde_json::to_vec(&LoginRequest {
                    username: "root_admin".to_string(),
                    password: "wrong_password".to_string(),
                })
                .unwrap(),
            ))
            .unwrap();
        let res_tw = app.clone().oneshot(req_tw).await.unwrap();
        assert_eq!(res_tw.status(), StatusCode::BAD_REQUEST);
        assert_eq!(res_tw.headers().get("content-language").unwrap(), "zh-TW");
        let bytes = axum::body::to_bytes(res_tw.into_body(), usize::MAX)
            .await
            .unwrap();
        let val_tw: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(val_tw["code"], 10007);
        assert_eq!(val_tw["message"], "使用者名稱或密碼錯誤");

        // 15. 验证操作审计日志落库且敏感密码已被完全脱敏
        let logs = db::OplogRepo::list_recent(
            &state.db,
            &db::repository::oplog::ListParams {
                module: Some("auth"),
                limit: 50,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(!logs.is_empty());
        for log in logs {
            assert!(!log.body.contains("myStrongPassword2026"));
            assert!(!log.body.contains("brandNewPassword2027!"));
            if !log.body.is_empty() {
                assert!(log.body.contains("******"));
            }
        }

        // 16. 验证 WebSocket /ws/events 必须受鉴权保护
        let req_ws_no_token = Request::builder()
            .uri("/api/v1/ws/events")
            .method("GET")
            .body(Body::empty())
            .unwrap();
        let res_ws = app.clone().oneshot(req_ws_no_token).await.unwrap();
        assert_eq!(res_ws.status(), StatusCode::UNAUTHORIZED);

        // 17. 验证持久化密钥保持一致性
        let secret1 = state.get_jwt_secret();
        let state2 = AppState::new(state.db.clone(), state.pipeline.clone());
        crate::sync_auth_state(&state2).await;
        let secret2 = state2.get_jwt_secret();
        assert_eq!(secret1, secret2);
    }

    /// 登录失败达到阈值后必须返回 429 并附带 Retry-After，
    /// 且限流必须发生在密码哈希之前（否则退避期间仍会消耗 PBKDF2）。
    #[tokio::test]
    async fn test_login_rate_limit_returns_429_with_retry_after() {
        let (app, state) = setup_test_app().await;

        let hash = crate::crypto::hash_password_async("correct-horse-battery".to_string()).await;
        db::AdminUserRepo::create_admin(&state.db, "admin", &hash)
            .await
            .unwrap();
        state.is_initialized.store(true, Ordering::Relaxed);

        // 阈值前：统一以 10007 拒绝，不泄露用户是否存在
        for _ in 0..4 {
            let (status, body, _) = post_login(&app, "admin", "wrong-password").await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(body["code"], 10007);
        }

        // 第 5 次失败触发锁定，本次仍按凭据错误返回
        let (status, _, _) = post_login(&app, "admin", "wrong-password").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // 此后进入退避：状态码与业务码都必须变，便于前端区分「密码错」与「被限流」
        let (status, body, _) = post_login(&app, "admin", "wrong-password").await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(body["code"], 10009);

        // 退避期间即使密码正确也必须被拦下，且必须带 Retry-After
        let (status, _, headers) = post_login(&app, "admin", "correct-horse-battery").await;
        assert_eq!(
            status,
            StatusCode::TOO_MANY_REQUESTS,
            "退避期间即使密码正确也必须被拦下"
        );
        assert!(
            headers.get("retry-after").is_some(),
            "限流响应必须带 Retry-After"
        );
    }

    /// 大小写变体不得把单个账号的额度摊成多份。
    #[tokio::test]
    async fn test_rate_limit_buckets_username_case_insensitively() {
        let (app, state) = setup_test_app().await;

        let hash = crate::crypto::hash_password_async("correct-horse-battery".to_string()).await;
        db::AdminUserRepo::create_admin(&state.db, "admin", &hash)
            .await
            .unwrap();
        state.is_initialized.store(true, Ordering::Relaxed);

        // 阈值（5）之内的变体：全部按凭据错误拒绝。这些变体在 trim + 小写后
        // 落到同一个键（"admin"），因此计数是累加的而非各自一份。
        let within_threshold = ["admin", "ADMIN", "Admin", "aDmIn", "admin "];
        for variant in within_threshold {
            let (status, body, _) = post_login(&app, variant, "wrong-password").await;
            assert_eq!(
                status,
                StatusCode::BAD_REQUEST,
                "变体 {variant:?} 未按凭据错误拒绝"
            );
            assert_eq!(body["code"], 10007);
        }

        // 第 6 个变体是阈值后的第一次尝试：若每个变体各算一份额度，
        // 这里仍会返回 400；实际返回 429 才能证明它们共用同一账号桶。
        let (status, body, _) = post_login(&app, "  ADMIN  ", "wrong-password").await;
        assert_eq!(
            status,
            StatusCode::TOO_MANY_REQUESTS,
            "大小写与空白变体应共享同一账号桶，实际 {:?}",
            body
        );
    }

    /// 登录成功后账号桶清零，正常用户不会因早期误输而被锁定。
    #[tokio::test]
    async fn test_successful_login_resets_username_bucket() {
        let (app, state) = setup_test_app().await;

        let hash = crate::crypto::hash_password_async("correct-horse-battery".to_string()).await;
        db::AdminUserRepo::create_admin(&state.db, "admin", &hash)
            .await
            .unwrap();
        state.is_initialized.store(true, Ordering::Relaxed);

        for _ in 0..4 {
            let _ = post_login(&app, "admin", "wrong-password").await;
        }
        let (status, _, _) = post_login(&app, "admin", "correct-horse-battery").await;
        assert_eq!(status, StatusCode::OK);

        // 桶已清零：再错一次只应回到计数 1，而不是直接锁定
        let (status, body, _) = post_login(&app, "admin", "wrong-password").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], 10007);
    }

    /// 时频侧信道防护：用户不存在与密码错误必须返回完全相同的响应。
    ///
    /// 断言响应报文与状态码一致；耗时对等由 `verify_dummy_password_async` 保证，
    /// 单测不做墙钟比较（CI 抖动会让基于时间的断言变成 flaky）。
    #[tokio::test]
    async fn test_unknown_user_and_wrong_password_are_indistinguishable() {
        let (app, state) = setup_test_app().await;

        let hash = crate::crypto::hash_password_async("correct-horse-battery".to_string()).await;
        db::AdminUserRepo::create_admin(&state.db, "admin", &hash)
            .await
            .unwrap();
        state.is_initialized.store(true, Ordering::Relaxed);

        let (status_known, body_known, _) = post_login(&app, "admin", "wrong-password").await;
        let (status_unknown, body_unknown, _) =
            post_login(&app, "no-such-operator", "wrong-password").await;

        assert_eq!(status_known, status_unknown);
        assert_eq!(body_known["code"], body_unknown["code"]);
        assert_eq!(body_known["message"], body_unknown["message"]);
    }

    /// 占位哈希必须真的被计算：被限流的账号不能靠退避绕过时频抹平。
    ///
    /// 该用例同时钉住「占位哈希与真实哈希同迭代数」这一前提，
    /// 否则两条分支的耗时差会重新暴露用户名是否存在。
    #[test]
    fn test_dummy_hash_matches_current_iterations() {
        assert!(!crate::crypto::needs_rehash(
            crate::crypto::dummy_password_hash()
        ));
    }

    /// 低迭代数历史哈希在登录成功后必须被自动升级到当前强度，
    /// 且升级不得作废既有 Token（不是改密）。
    #[tokio::test]
    async fn test_login_upgrades_legacy_hash_without_invalidating_tokens() {
        let (app, state) = setup_test_app().await;

        let password = "legacy-password-2026";
        let legacy_hash = crate::crypto::hash_password_with_iterations(password, 1_000);
        db::AdminUserRepo::create_admin(&state.db, "admin", &legacy_hash)
            .await
            .unwrap();
        state.is_initialized.store(true, Ordering::Relaxed);

        let (status, body, _) = post_login(&app, "admin", password).await;
        assert_eq!(status, StatusCode::OK);
        let token = body["data"]["accessToken"].as_str().unwrap().to_string();

        // 库里已是当前强度的新哈希
        let stored = db::AdminUserRepo::find_by_username(&state.db, "admin")
            .await
            .unwrap()
            .unwrap();
        assert!(!crate::crypto::needs_rehash(&stored.password_hash));
        assert!(crate::crypto::verify_password(
            password,
            &stored.password_hash
        ));

        // 升级哈希不是改密：既有 Token 必须继续可用
        assert_eq!(stored.token_invalid_before, 0);
        let req = Request::builder()
            .uri("/api/v1/auth/me")
            .method("GET")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(
            res.status(),
            StatusCode::OK,
            "哈希升级不得把用户从会话中踢出"
        );
    }

    /// 登录失败必须留审计痕迹，且密码仍然脱敏。
    #[tokio::test]
    async fn test_failed_login_is_audited_with_masked_password() {
        let (app, state) = setup_test_app().await;

        let hash = crate::crypto::hash_password_async("correct-horse-battery".to_string()).await;
        db::AdminUserRepo::create_admin(&state.db, "admin", &hash)
            .await
            .unwrap();
        state.is_initialized.store(true, Ordering::Relaxed);

        let _ = post_login(&app, "admin", "leaked-plaintext-attempt").await;

        let logs = db::OplogRepo::list_recent(
            &state.db,
            &db::repository::oplog::ListParams {
                module: Some("auth"),
                limit: 50,
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let failed: Vec<_> = logs
            .iter()
            .filter(|log| log.action == "login_failed")
            .collect();
        assert_eq!(failed.len(), 1, "失败登录应恰好留下一条审计");
        assert!(!failed[0].body.contains("leaked-plaintext-attempt"));
        assert!(failed[0].body.contains("******"));
    }

    /// 超长 body 必须在解析前被独立上限拒绝：登录接口只接受两个短字段，
    /// 若它跟随其它路由的放宽上限，匿名请求就能在反序列化阶段占满内存。
    #[tokio::test]
    async fn test_login_body_limit_is_tight_and_independent() {
        let (app, state) = setup_test_app().await;
        let _ = state;

        let (status, _, _) = post_login(&app, "admin", &"a".repeat(900_000)).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    }

    /// 服务端与前端必须共享同一条密码长度规则。
    #[tokio::test]
    async fn test_initialize_rejects_short_password_by_character_count() {
        let (app, _state) = setup_test_app().await;

        // 5 个字符：拒绝
        let req = Request::builder()
            .uri("/api/v1/auth/initialize")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&InitializeRequest {
                    username: "admin".to_string(),
                    password: "密码五个字".to_string(),
                })
                .unwrap(),
            ))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);

        // 6 个字符：接受（若长度按字节数判定，上面那条会因 15 字节而通过）
        let req = Request::builder()
            .uri("/api/v1/auth/initialize")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&InitializeRequest {
                    username: "admin".to_string(),
                    password: "密码六个字符".to_string(),
                })
                .unwrap(),
            ))
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
    }

    /// 提交登录请求，返回 (状态码, 响应 JSON, 响应头)。
    async fn post_login(
        app: &axum::Router,
        username: &str,
        password: &str,
    ) -> (StatusCode, serde_json::Value, axum::http::HeaderMap) {
        let req = Request::builder()
            .uri("/api/v1/auth/login")
            .method("POST")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&LoginRequest {
                    username: username.to_string(),
                    password: password.to_string(),
                })
                .unwrap(),
            ))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let headers = res.headers().clone();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
        (status, body, headers)
    }
}
