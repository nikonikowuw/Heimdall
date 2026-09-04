use super::{Locale, StaticMessage};

pub const STATIC_MESSAGES: &[StaticMessage] = &[
    StaticMessage {
        code: 30001,
        zh_cn: "分析任务未找到",
        zh_tw: "分析任務未找到",
        en: "Analysis task not found",
    },
    StaticMessage {
        code: 30002,
        zh_cn: "布防规则配置无效",
        zh_tw: "布防規則配置無效",
        en: "Invalid detection rule configuration",
    },
    StaticMessage {
        code: 30003,
        zh_cn: "AI 模型加载或推理失败",
        zh_tw: "AI 模型加載或推理失敗",
        en: "AI model loading or inference failed",
    },
];

pub fn translate_task(code: u32, _original_msg: &str, locale: Locale) -> Option<String> {
    if let Some(entry) = STATIC_MESSAGES.iter().find(|m| m.code == code) {
        return Some(entry.for_locale(locale).to_string());
    }
    None
}
