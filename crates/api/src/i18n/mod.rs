pub mod alarm;
pub mod auth;
pub mod camera;
pub mod common;
pub mod system;
pub mod task;

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

#[derive(Debug)]
pub struct StaticMessage {
    pub code: u32,
    pub zh_cn: &'static str,
    pub zh_tw: &'static str,
    pub en: &'static str,
}

impl StaticMessage {
    pub fn for_locale(&self, locale: Locale) -> &'static str {
        match locale {
            Locale::ZhCn => self.zh_cn,
            Locale::ZhTw => self.zh_tw,
            Locale::En => self.en,
        }
    }
}

/// 根据错误码段路由至对应领域模块，生成目标语言的本地化文本
pub fn localize_api_message(code: u32, original_msg: &str, locale: Locale) -> String {
    // 1. 按业务错误码段路由到各领域模块解析
    let localized = match code {
        0 => common::translate_common(code, original_msg, locale),
        10000..=19999 => auth::translate_auth(code, original_msg, locale),
        20000..=29999 => camera::translate_camera(code, original_msg, locale),
        30000..=39999 => task::translate_task(code, original_msg, locale),
        40002..=49999 => alarm::translate_alarm(code, original_msg, locale),
        51000..=51999 => system::translate_system(code, original_msg, locale),
        _ => common::translate_common(code, original_msg, locale),
    };

    localized.unwrap_or_else(|| original_msg.to_string())
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
        // Auth 10000 段
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

        // Camera 20000 段
        assert_eq!(
            localize_api_message(20001, "原消息", Locale::En),
            "Camera RTSP connection failed"
        );
        assert_eq!(
            localize_api_message(20003, "原消息", Locale::ZhTw),
            "視訊 SPS 參數集解析失敗"
        );
        assert_eq!(
            localize_api_message(20004, "原消息", Locale::ZhCn),
            "摄像头探活超时"
        );
        assert_eq!(
            localize_api_message(20011, "原消息", Locale::ZhCn),
            "媒体流传输静默超时"
        );
        assert_eq!(
            localize_api_message(20011, "原消息", Locale::En),
            "Media stream inactivity timeout"
        );

        // Task & Infer 30000 段
        assert_eq!(
            localize_api_message(30001, "原消息", Locale::En),
            "Analysis task not found"
        );
        assert_eq!(
            localize_api_message(30004, "原消息", Locale::ZhTw),
            "攝影機分析管線已存在"
        );
        assert_eq!(
            localize_api_message(30011, "原消息", Locale::En),
            "Failed to load AI model file"
        );
        assert_eq!(
            localize_api_message(30012, "原消息", Locale::ZhTw),
            "演算法包動態庫加載失敗"
        );
        assert_eq!(
            localize_api_message(30014, "原消息", Locale::ZhCn),
            "算法 C ABI 虚表不匹配或无效"
        );
        assert_eq!(
            localize_api_message(30016, "原消息", Locale::En),
            "Algorithm sandbox security validation failed"
        );
        assert_eq!(
            localize_api_message(30016, "原消息", Locale::ZhTw),
            "演算法包沙箱安全校驗失敗"
        );
        assert_eq!(
            localize_api_message(30016, "原消息", Locale::ZhCn),
            "算法包沙箱安全校验失败"
        );
        assert_eq!(
            localize_api_message(30020, "原消息", Locale::En),
            "Algorithm hardware inference timed out"
        );

        // Common & Db 50000 段
        assert_eq!(
            localize_api_message(50001, "原消息", Locale::En),
            "Database operation error"
        );
        assert_eq!(
            localize_api_message(50001, "原消息", Locale::ZhTw),
            "資料庫操作異常"
        );
        assert_eq!(
            localize_api_message(50001, "原消息", Locale::ZhCn),
            "数据库操作异常"
        );

        // Alarm 40000 段
        assert_eq!(
            localize_api_message(40002, "原消息", Locale::En),
            "Alarm record not found"
        );

        // System 51000 段
        assert_eq!(
            localize_api_message(51011, "原消息: 192.168.1.100", Locale::En),
            "Static IP conflict detected: 192.168.1.100"
        );
        assert_eq!(
            localize_api_message(51011, "原消息", Locale::ZhTw),
            "檢測到靜態 IP 衝突"
        );
        assert_eq!(
            localize_api_message(51006, "原消息", Locale::ZhCn),
            "已有进行中的网络变更操作"
        );
    }
}
