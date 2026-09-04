/// 支持的语言区域枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Locale {
    #[default]
    ZhCn,
    ZhTw,
    En,
}

impl Locale {
    /// 从 Accept-Language 请求头解析语言偏好
    pub fn from_accept_language(header: Option<&str>) -> Self {
        let header = match header {
            Some(h) => h.to_lowercase(),
            None => return Self::ZhCn,
        };

        if header.starts_with("en") || header.contains(",en") || header.contains(";en") {
            Self::En
        } else if header.contains("zh-tw") || header.contains("zh-hk") || header.contains("zh-hant")
        {
            Self::ZhTw
        } else {
            Self::ZhCn
        }
    }

    /// 转换为 BCP-47 规范语言标识
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ZhCn => "zh-CN",
            Self::ZhTw => "zh-TW",
            Self::En => "en",
        }
    }
}

struct StaticMessage {
    code: u32,
    zh_cn: &'static str,
    zh_tw: &'static str,
    en: &'static str,
}

impl StaticMessage {
    fn for_locale(&self, locale: Locale) -> &'static str {
        match locale {
            Locale::ZhCn => self.zh_cn,
            Locale::ZhTw => self.zh_tw,
            Locale::En => self.en,
        }
    }
}

const STATIC_CODE_MESSAGES: &[StaticMessage] = &[
    StaticMessage {
        code: 0,
        zh_cn: "success",
        zh_tw: "成功",
        en: "success",
    },
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

/// 根据错误码及原消息生成对应语言的本地化文本
pub fn localize_api_message(code: u32, original_msg: &str, locale: Locale) -> String {
    // 1. 优先在静态常量表中进行 $O(N)$ 快速映射查找
    if let Some(entry) = STATIC_CODE_MESSAGES.iter().find(|m| m.code == code) {
        return entry.for_locale(locale).to_string();
    }

    // 2. 动态或带有参数上下文的特殊状态码处理
    match code {
        10008 => match locale {
            Locale::En => {
                if original_msg.contains('6') {
                    "Password must be at least 6 characters".to_string()
                } else {
                    "Password does not meet strength requirements".to_string()
                }
            }
            Locale::ZhTw => {
                if original_msg.contains('6') {
                    "密碼長度不能少於 6 位".to_string()
                } else {
                    "密碼強度不符合要求".to_string()
                }
            }
            Locale::ZhCn => {
                if original_msg.contains('6') {
                    "新密码长度不能少于 6 位".to_string()
                } else {
                    "新密码强度不符合要求".to_string()
                }
            }
        },
        40001 => match locale {
            Locale::En => {
                if original_msg.contains("用户名不能为空") {
                    "Username cannot be empty".to_string()
                } else {
                    format!("Invalid request parameters: {original_msg}")
                }
            }
            Locale::ZhTw => {
                if original_msg.contains("用户名不能为空") {
                    "管理員使用者名稱不能為空".to_string()
                } else {
                    format!("請求參數校驗失敗: {original_msg}")
                }
            }
            Locale::ZhCn => original_msg.to_string(),
        },
        40401 => match locale {
            Locale::En => format!("Resource not found: {original_msg}"),
            Locale::ZhTw => format!("資源未找到: {original_msg}"),
            Locale::ZhCn => original_msg.to_string(),
        },
        50000..=59999 => match locale {
            Locale::En => "Internal server error".to_string(),
            Locale::ZhTw => "內部伺服器錯誤".to_string(),
            Locale::ZhCn => original_msg.to_string(),
        },
        _ => original_msg.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_locale_parsing() {
        assert_eq!(Locale::from_accept_language(None), Locale::ZhCn);
        assert_eq!(
            Locale::from_accept_language(Some("en-US,en;q=0.9")),
            Locale::En
        );
        assert_eq!(
            Locale::from_accept_language(Some("zh-TW,zh;q=0.9")),
            Locale::ZhTw
        );
        assert_eq!(
            Locale::from_accept_language(Some("zh-CN,zh;q=0.9")),
            Locale::ZhCn
        );
    }

    #[test]
    fn test_localize_api_message() {
        assert_eq!(
            localize_api_message(10007, "用户名或密码错误", Locale::En),
            "Invalid username or password"
        );
        assert_eq!(
            localize_api_message(10007, "用户名或密码错误", Locale::ZhTw),
            "使用者名稱或密碼錯誤"
        );
        assert_eq!(
            localize_api_message(10001, "未登录（缺少凭据）", Locale::En),
            "Authentication required (missing credentials)"
        );
    }
}
