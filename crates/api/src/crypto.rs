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

const PBKDF2_ITERATIONS: u32 = 10_000;

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
    let salt_uuid = uuid::Uuid::new_v4();
    let salt = salt_uuid.as_bytes();
    let derived = pbkdf2_hmac_sha256(password.as_bytes(), salt, PBKDF2_ITERATIONS);

    let salt_b64 = URL_SAFE_NO_PAD.encode(salt);
    let hash_b64 = URL_SAFE_NO_PAD.encode(derived);

    format!("$pbkdf2-sha256$i={PBKDF2_ITERATIONS}${salt_b64}${hash_b64}")
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
    let parts: Vec<&str> = encoded.split('$').collect();
    if parts.len() != 5 || parts[1] != "pbkdf2-sha256" {
        return false;
    }

    let iterations = match parts[2].strip_prefix("i=") {
        Some(iter_str) => match iter_str.parse::<u32>() {
            Ok(val) => val,
            Err(_) => return false,
        },
        None => return false,
    };

    let salt = match URL_SAFE_NO_PAD.decode(parts[3]) {
        Ok(s) => s,
        Err(_) => return false,
    };

    let expected_hash = match URL_SAFE_NO_PAD.decode(parts[4]) {
        Ok(h) => h,
        Err(_) => return false,
    };

    let derived = pbkdf2_hmac_sha256(password.as_bytes(), &salt, iterations);
    constant_time_eq(&derived, &expected_hash)
}

/// 异步非阻塞密码校验，避免在 Tokio 主调度器上同步运行高频迭代
pub async fn verify_password_async(password: String, encoded: String) -> bool {
    tokio::task::spawn_blocking(move || verify_password(&password, &encoded))
        .await
        .unwrap_or(false)
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

        assert!(hash.starts_with("$pbkdf2-sha256$i=10000$"));
        assert!(verify_password(password, &hash));
        assert!(!verify_password("wrong-password", &hash));
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
