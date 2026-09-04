use serde::{Deserialize, Serialize};

/// 管理员账户实体（领域层）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AdminUser {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
    pub token_invalid_before: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

/// JWT 载荷声明
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthClaims {
    /// 令牌主题（当前管理员用户名）
    pub sub: String,
    /// 签发时间戳（13 位 UTC 毫秒）
    pub iat: i64,
    /// 过期时间戳（13 位 UTC 毫秒）
    pub exp: i64,
}

/// 系统初始化状态响应体
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InitStatusResponse {
    pub initialized: bool,
}

/// 开箱首次初始化请求体
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InitializeRequest {
    pub username: String,
    pub password: String,
}

/// 登录请求体
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// 登录成功响应体
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LoginResponse {
    pub access_token: String,
    pub username: String,
    pub expires_at: i64,
}

/// 修改密码请求体
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChangePasswordRequest {
    pub old_password: String,
    pub new_password: String,
}

/// 管理员个人信息响应 DTO
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AdminUserDto {
    pub username: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<AdminUser> for AdminUserDto {
    fn from(user: AdminUser) -> Self {
        Self {
            username: user.username,
            created_at: user.created_at,
            updated_at: user.updated_at,
        }
    }
}
