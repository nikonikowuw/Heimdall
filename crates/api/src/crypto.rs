use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::error::ApiError;

/// 对 JSON 报文中的敏感字段（如密码、Token 等）执行递归脱敏
pub fn mask_sensitive_json(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                if v.is_object() || v.is_array() {
                    mask_sensitive_json(v);
                } else {
                    let lower = k.to_lowercase();
                    if lower.contains("password")
                        || lower == "token"
                        || lower == "accesstoken"
                        || lower == "access_token"
                        || lower == "secret"
                    {
                        *v = serde_json::Value::String("******".to_string());
                    }
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr.iter_mut() {
                mask_sensitive_json(item);
            }
        }
        _ => {}
    }
}

/// 对 JSON 字符串中的敏感字段执行脱敏，若非合法 JSON 则直接返回原字符串
pub fn mask_sensitive_json_str(body: &str) -> String {
    if let Ok(mut json) = serde_json::from_str::<serde_json::Value>(body) {
        mask_sensitive_json(&mut json);
        json.to_string()
    } else {
        body.to_string()
    }
}

type HmacSha256 = Hmac<Sha256>;

/// 新建密码哈希使用的 PBKDF2 迭代数。
///
/// 取值依据是实测而非照抄 OWASP 基线（600k）：面向 ARM 边缘设备的部署里，
/// 单次校验的开销直接等于登录接口的最坏并发延迟，600k 会让每个登录请求吃掉
/// 百余毫秒的 blocking 线程。210k 在开发机实测约 62ms/次，是「离线爆破成本」
/// 与「边缘设备可承受延迟」的折中。
///
/// 该值只影响**新建**哈希。历史哈希通过内嵌的 `i=` 参数自行声明迭代数，
/// 因此调整此常量不会让既有密码失效；登录成功后按 [`needs_rehash`] 被动升级。
pub(crate) const PBKDF2_ITERATIONS: u32 = 210_000;

/// 用于抹平「用户不存在」与「密码错误」耗时的占位哈希。
///
/// 用户不存在时若直接返回，请求会在跳过 PBKDF2 的情况下提前结束，
/// 响应耗时差异可被远程测量，从而枚举出合法管理员用户名。
/// 该哈希在首次使用时按当前迭代数惰性生成，保证两条分支的 CPU 开销同量级。
static DUMMY_PASSWORD_HASH: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// 返回占位哈希，供用户不存在分支执行一次等价耗时的校验。
pub fn dummy_password_hash() -> &'static str {
    DUMMY_PASSWORD_HASH.get_or_init(|| {
        // 占位哈希的明文不可猜，避免攻击者用一个已知密码命中占位条目。
        hash_password("heimdall-timing-equalizer-placeholder")
    })
}

/// 判断既有哈希是否需要用当前迭代数重新派生。
pub fn needs_rehash(encoded: &str) -> bool {
    parse_iterations(encoded).is_some_and(|iterations| iterations < PBKDF2_ITERATIONS)
}

/// 常数时间字节切片比对，抵御时序侧信道攻击
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut res = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        res |= x ^ y;
    }
    res == 0
}

/// 标准 PBKDF2-HMAC-SHA256 单块密钥派生（克隆预设密钥上下文提升计算吞吐）
fn pbkdf2_hmac_sha256(password: &[u8], salt: &[u8], iterations: u32) -> [u8; 32] {
    let base_hmac = HmacSha256::new_from_slice(password).expect("HMAC can accept any key length");

    let mut hmac = base_hmac.clone();
    hmac.update(salt);
    hmac.update(&1u32.to_be_bytes());
    let mut u = hmac.finalize().into_bytes();
    let mut out = u;

    for _ in 1..iterations {
        let mut hmac = base_hmac.clone();
        hmac.update(&u);
        u = hmac.finalize().into_bytes();
        for (o, byte) in out.iter_mut().zip(u.iter()) {
            *o ^= byte;
        }
    }

    out.into()
}

/// 使用加盐 PBKDF2-HMAC-SHA256 对明文密码进行哈希
pub fn hash_password(password: &str) -> String {
    hash_password_with_iterations(password, PBKDF2_ITERATIONS)
}

/// 以指定迭代数派生哈希。
///
/// 仅供测试构造「历史低强度哈希」以验证平滑升级路径；生产路径一律走 [`hash_password`]，
/// 不给调用方留下降低强度的后门。
pub(crate) fn hash_password_with_iterations(password: &str, iterations: u32) -> String {
    let salt_uuid = uuid::Uuid::new_v4();
    let salt = salt_uuid.as_bytes();
    let derived = pbkdf2_hmac_sha256(password.as_bytes(), salt, iterations);

    let salt_b64 = URL_SAFE_NO_PAD.encode(salt);
    let hash_b64 = URL_SAFE_NO_PAD.encode(derived);

    format!("$pbkdf2-sha256$i={iterations}${salt_b64}${hash_b64}")
}

/// 异步非阻塞密码哈希计算，将 CPU 密集运算委派给 blocking 线程池，杜绝阻塞 Tokio Worker
pub async fn hash_password_async(password: String) -> String {
    let pwd = password.clone();
    tokio::task::spawn_blocking(move || hash_password(&pwd))
        .await
        .unwrap_or_else(|_| hash_password(&password))
}

/// 校验明文密码是否与加盐哈希匹配
pub fn verify_password(password: &str, encoded: &str) -> bool {
    let (iterations, salt, expected_hash) = match parse_encoded(encoded) {
        Some(parsed) => parsed,
        None => return false,
    };

    let derived = pbkdf2_hmac_sha256(password.as_bytes(), &salt, iterations);
    constant_time_eq(&derived, &expected_hash)
}

/// 从 `$pbkdf2-sha256$i={n}${salt}${hash}` 提取迭代数，避免额外解码 base64 缓冲区。
fn parse_iterations(encoded: &str) -> Option<u32> {
    let parts: Vec<&str> = encoded.split('$').collect();
    if parts.len() != 5 || parts[1] != "pbkdf2-sha256" {
        return None;
    }
    parts[2].strip_prefix("i=")?.parse::<u32>().ok()
}

/// 解析 `$pbkdf2-sha256$i={n}${salt}${hash}` 编码，返回迭代数与原始字节。
fn parse_encoded(encoded: &str) -> Option<(u32, Vec<u8>, Vec<u8>)> {
    let parts: Vec<&str> = encoded.split('$').collect();
    if parts.len() != 5 || parts[1] != "pbkdf2-sha256" {
        return None;
    }

    let iterations = parts[2].strip_prefix("i=")?.parse::<u32>().ok()?;
    let salt = URL_SAFE_NO_PAD.decode(parts[3]).ok()?;
    let hash = URL_SAFE_NO_PAD.decode(parts[4]).ok()?;

    Some((iterations, salt, hash))
}

/// 异步非阻塞密码校验，避免在 Tokio 主调度器上同步运行高频迭代
pub async fn verify_password_async(password: String, encoded: String) -> bool {
    tokio::task::spawn_blocking(move || verify_password(&password, &encoded))
        .await
        .unwrap_or(false)
}

/// 对不存在的用户执行一次等价耗时的校验。
///
/// 调用方必须忽略返回值：它唯一的职责是消耗掉与真实校验同量级的 CPU 时间，
/// 使「用户不存在」与「密码错误」两条分支在响应耗时上不可区分。
pub async fn verify_dummy_password_async(password: String) {
    let _ = verify_password_async(password, dummy_password_hash().to_string()).await;
}

/// 签发 HS256 标准 JWT 令牌
pub fn generate_jwt(claims: &types::AuthClaims, secret: &[u8]) -> Result<String, ApiError> {
    let header_json = serde_json::json!({
        "alg": "HS256",
        "typ": "JWT"
    });
    let header_b64 = URL_SAFE_NO_PAD.encode(header_json.to_string().as_bytes());

    let payload_json =
        serde_json::to_string(claims).map_err(|e| ApiError::Internal(e.to_string()))?;
    let payload_b64 = URL_SAFE_NO_PAD.encode(payload_json.as_bytes());

    let signing_input = format!("{header_b64}.{payload_b64}");

    let mut mac =
        HmacSha256::new_from_slice(secret).map_err(|e| ApiError::Internal(e.to_string()))?;
    mac.update(signing_input.as_bytes());
    let signature_bytes = mac.finalize().into_bytes();
    let signature_b64 = URL_SAFE_NO_PAD.encode(signature_bytes);

    Ok(format!("{signing_input}.{signature_b64}"))
}

/// 校验 JWT 签名、有效期并核对撤销时间戳
pub fn verify_jwt(
    token: &str,
    secret: &[u8],
    token_invalid_before: i64,
) -> Result<types::AuthClaims, ApiError> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(ApiError::TokenRevoked);
    }

    let signing_input = format!("{}.{}", parts[0], parts[1]);
    let signature = URL_SAFE_NO_PAD
        .decode(parts[2])
        .map_err(|_| ApiError::TokenRevoked)?;

    let mut mac =
        HmacSha256::new_from_slice(secret).map_err(|e| ApiError::Internal(e.to_string()))?;
    mac.update(signing_input.as_bytes());
    let expected_sig = mac.finalize().into_bytes();

    if !constant_time_eq(&signature, &expected_sig) {
        return Err(ApiError::TokenRevoked);
    }

    let payload_bytes = URL_SAFE_NO_PAD
        .decode(parts[1])
        .map_err(|_| ApiError::TokenRevoked)?;
    let claims: types::AuthClaims =
        serde_json::from_slice(&payload_bytes).map_err(|_| ApiError::TokenRevoked)?;

    let now_ms = chrono::Utc::now().timestamp_millis();

    // 检查是否过期
    if claims.exp < now_ms {
        return Err(ApiError::TokenExpired);
    }

    // 检查是否在撤销时间点之前签发
    if claims.iat < token_invalid_before {
        return Err(ApiError::TokenRevoked);
    }

    Ok(claims)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_password_hash_and_verify() {
        let password = "SuperSecretPassword2026!";
        let hash = hash_password(password);

        // 前缀断言绑定当前迭代数：调整 PBKDF2_ITERATIONS 时此测试必须同步更新，
        // 避免「改了强度但没人发现」的静默漂移。
        assert!(hash.starts_with(&format!("$pbkdf2-sha256$i={PBKDF2_ITERATIONS}$")));
        assert!(verify_password(password, &hash));
        assert!(!verify_password("wrong-password", &hash));
    }

    #[test]
    fn test_legacy_hash_still_verifies_after_iteration_bump() {
        // 模拟升级前生成的低迭代数哈希：携带 i=10000 的旧格式必须继续可用，
        // 否则调整 PBKDF2_ITERATIONS 会把所有既有部署锁在门外。
        let password = "legacy-password-2026";
        let legacy = hash_password_with_iterations(password, 10_000);

        assert!(legacy.starts_with("$pbkdf2-sha256$i=10000$"));
        assert!(verify_password(password, &legacy));
        assert!(!verify_password("another-password", &legacy));
        assert!(needs_rehash(&legacy), "低迭代数哈希应被标记为需要升级");
    }

    #[test]
    fn test_needs_rehash_ignores_current_and_malformed() {
        assert!(!needs_rehash(&hash_password("whatever")));
        assert!(!needs_rehash("not-a-hash"));
        assert!(!needs_rehash("$pbkdf2-sha256$i=abc$salt$hash"));
    }

    #[test]
    fn test_dummy_hash_is_verifiable_and_stable() {
        // 占位哈希必须在同一次进程内保持同一实例：否则每次调用都重新派生 210k 次
        let first = dummy_password_hash();
        let second = dummy_password_hash();
        assert!(std::ptr::eq(first, second));
        assert!(!verify_password("anything", first));
        assert!(!needs_rehash(first));
    }

    #[test]
    fn test_jwt_flow() {
        let secret = b"unit-test-secret-key-at-least-32-bytes-long!";
        let now = chrono::Utc::now().timestamp_millis();
        let claims = types::AuthClaims {
            sub: "admin".to_string(),
            iat: now,
            exp: now + 3_600_000,
        };

        // 正常签发与校验
        let token = generate_jwt(&claims, secret).unwrap();
        let verified = verify_jwt(&token, secret, 0).unwrap();
        assert_eq!(verified.sub, "admin");

        // 密钥错误
        let err = verify_jwt(&token, b"different-secret-key-different-secret!", 0);
        assert!(err.is_err());

        // Token 签发早于失效时间戳
        let revoked_err = verify_jwt(&token, secret, now + 1000);
        assert!(matches!(revoked_err, Err(ApiError::TokenRevoked)));

        // 过期 Token
        let expired_claims = types::AuthClaims {
            sub: "admin".to_string(),
            iat: now - 5000,
            exp: now - 1000,
        };
        let expired_token = generate_jwt(&expired_claims, secret).unwrap();
        let expired_err = verify_jwt(&expired_token, secret, 0);
        assert!(matches!(expired_err, Err(ApiError::TokenExpired)));
    }

    #[test]
    fn test_mask_sensitive_json() {
        let mut json = serde_json::json!({
            "username": "admin",
            "password": "myPlainPassword",
            "nested": {
                "oldPassword": "old",
                "new_password": "new",
                "normalField": 123
            },
            "tokens": [
                {"accessToken": "secret-token", "info": "ok"}
            ]
        });

        mask_sensitive_json(&mut json);

        assert_eq!(json["password"], "******");
        assert_eq!(json["nested"]["oldPassword"], "******");
        assert_eq!(json["nested"]["new_password"], "******");
        assert_eq!(json["nested"]["normalField"], 123);
        assert_eq!(json["tokens"][0]["accessToken"], "******");
        assert_eq!(json["tokens"][0]["info"], "ok");
    }
}
