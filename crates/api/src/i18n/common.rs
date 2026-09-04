use super::{Locale, StaticMessage};

pub const STATIC_MESSAGES: &[StaticMessage] = &[StaticMessage {
    code: 0,
    zh_cn: "success",
    zh_tw: "成功",
    en: "success",
}];

pub fn translate_common(code: u32, original_msg: &str, locale: Locale) -> Option<String> {
    if let Some(entry) = STATIC_MESSAGES.iter().find(|m| m.code == code) {
        return Some(entry.for_locale(locale).to_string());
    }

    match code {
        40001 => match locale {
            Locale::En => {
                if original_msg.contains("用户名不能为空") {
                    Some("Username cannot be empty".to_string())
                } else {
                    Some(format!("Invalid request parameters: {original_msg}"))
                }
            }
            Locale::ZhTw => {
                if original_msg.contains("用户名不能为空") {
                    Some("管理員使用者名稱不能為空".to_string())
                } else {
                    Some(format!("請求參數校驗失敗: {original_msg}"))
                }
            }
            Locale::ZhCn => Some(original_msg.to_string()),
        },
        40401 => match locale {
            Locale::En => Some(format!("Resource not found: {original_msg}")),
            Locale::ZhTw => Some(format!("資源未找到: {original_msg}")),
            Locale::ZhCn => Some(original_msg.to_string()),
        },
        50000..=59999 => match locale {
            Locale::En => Some("Internal server error".to_string()),
            Locale::ZhTw => Some("內部伺服器錯誤".to_string()),
            Locale::ZhCn => Some(original_msg.to_string()),
        },
        _ => None,
    }
}
