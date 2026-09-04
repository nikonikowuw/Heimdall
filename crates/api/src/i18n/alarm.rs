use super::{Locale, StaticMessage};

pub const STATIC_MESSAGES: &[StaticMessage] = &[
    StaticMessage {
        code: 40002,
        zh_cn: "告警事件记录未找到",
        zh_tw: "告警事件記錄未找到",
        en: "Alarm record not found",
    },
    StaticMessage {
        code: 40003,
        zh_cn: "告警抓拍快照证据未找到",
        zh_tw: "告警抓拍快照證據未找到",
        en: "Alarm snapshot evidence not found",
    },
];

pub fn translate_alarm(code: u32, _original_msg: &str, locale: Locale) -> Option<String> {
    if let Some(entry) = STATIC_MESSAGES.iter().find(|m| m.code == code) {
        return Some(entry.for_locale(locale).to_string());
    }
    None
}
