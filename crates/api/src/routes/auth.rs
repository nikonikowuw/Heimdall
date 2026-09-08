use std::sync::atomic::Ordering;

use axum::extract::{Json, State};
use axum::http::{Extensions, HeaderMap};
use axum::routing::{get, post, put};
use axum::Router;

use crate::crypto::{
    generate_jwt, hash_password_async, mask_sensitive_json, verify_password_async,
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

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/init-status", get(get_init_status))
        .route("/initialize", post(initialize))
        .route("/login", post(login))
        .route("/password", put(change_password))
        .route("/me", get(get_me))
        .route("/logout", post(logout))
}

/// 启动时从数据库同步初始化状态、失效时间戳以及持久化 JWT Secret
pub async fn sync_auth_state(state: &AppState) {
    // 同步并持久化 JWT Secret（如果未通过环境变量注入）
    if std::env::var("ARGUS_JWT_SECRET")
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
            if let Ok(mut guard) = state.jwt_secret.write() {
                *guard = persisted_secret.into_bytes();
            }
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
fn issue_token(username: &str, secret: &[u8]) -> Result<(String, i64), ApiError> {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let expires_at = now_ms + 24 * 3600 * 1000; // 24小时有效

    let claims = AuthClaims {
        sub: username.to_string(),
        iat: now_ms,
        exp: expires_at,
    };

    let access_token = generate_jwt(&claims, secret)?;
    Ok((access_token, expires_at))
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

    if req.password.len() < 6 {
        return Err(ApiError::WeakPassword(
            "初始密码长度不能少于 6 位".to_string(),
        ));
    }

    let password_hash = hash_password_async(req.password.clone()).await;
    db::AdminUserRepo::create_admin(&state.db, username, &password_hash).await?;

    state.is_initialized.store(true, Ordering::Relaxed);

    let secret = state.get_jwt_secret();
    let (access_token, expires_at) = issue_token(username, &secret)?;

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
    if !state.is_initialized.load(Ordering::Relaxed) {
        return Err(ApiError::InvalidCredentials);
    }

    let username = req.username.trim();
    let user = db::AdminUserRepo::find_by_username(&state.db, username)
        .await?
        .ok_or(ApiError::InvalidCredentials)?;

    if !verify_password_async(req.password.clone(), user.password_hash.clone()).await {
        return Err(ApiError::InvalidCredentials);
    }

    let secret = state.get_jwt_secret();
    let (access_token, expires_at) = issue_token(&user.username, &secret)?;

    // 记录登录审计日志（密码脱敏）
    let mut body_json = serde_json::to_value(&req).unwrap_or_default();
    mask_sensitive_json(&mut body_json);
    let (client_ip, user_agent) = request_metadata(&headers, &extensions);
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
    if req.new_password.len() < 6 {
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
        let logs = db::OplogRepo::list_recent(&state.db, Some("auth"), 50, 0)
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
}
