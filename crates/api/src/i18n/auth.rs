use super::{Locale, StaticMessage};

pub const STATIC_MESSAGES: &[StaticMessage] = &[
    StaticMessage {
        code: 10001,
        zh_cn: "未登录（缺少凭据）",
        zh_tw: "未登入（缺少憑證）",
        en: "Authentication required (missing credentials)",
    },
    StaticMessage {
        code: 10002,
        zh_cn: "凭据已过期",
        zh_tw: "憑據已過期",
        en: "Credentials have expired",
    },
    StaticMessage {
        code: 10003,
        zh_cn: "凭据无效或已被撤销",
        zh_tw: "憑據無效或已被撤銷",
        en: "Credentials invalid or revoked",
    },
    StaticMessage {
        code: 10004,
        zh_cn: "无权限访问该资源",
        zh_tw: "無權限存取該資源",
        en: "Access denied: unauthorized resource",
    },
    StaticMessage {
        code: 10006,
        zh_cn: "系统已初始化，禁止重复初始化",
        zh_tw: "系統已完成初始化，禁止重複調用",
        en: "System already initialized, duplicate initialization forbidden",
    },
    StaticMessage {
        code: 10007,
        zh_cn: "用户名或密码错误",
        zh_tw: "使用者名稱或密碼錯誤",
        en: "Invalid username or password",
    },
];

pub fn translate_auth(code: u32, original_msg: &str, locale: Locale) -> Option<String> {
    if let Some(entry) = STATIC_MESSAGES.iter().find(|m| m.code == code) {
        return Some(entry.for_locale(locale).to_string());
    }

    match code {
        10008 => match locale {
            Locale::En => {
                if original_msg.contains('6') {
                    Some("Password must be at least 6 characters".to_string())
                } else {
                    Some("Password does not meet strength requirements".to_string())
                }
            }
            Locale::ZhTw => {
                if original_msg.contains('6') {
                    Some("密碼長度不能少於 6 位".to_string())
                } else {
                    Some("密碼強度不符合要求".to_string())
                }
            }
            Locale::ZhCn => {
                if original_msg.contains('6') {
                    Some("新密码长度不能少于 6 位".to_string())
                } else {
                    Some("新密码强度不符合要求".to_string())
                }
            }
        },
        _ => None,
    }
}
