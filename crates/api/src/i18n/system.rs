//! 系统设置模块 i18n 翻译 (51000..=51999)

use super::{Locale, StaticMessage};

const SYSTEM_MESSAGES: &[StaticMessage] = &[
    StaticMessage {
        code: 51001,
        zh_cn: "网络配置无效",
        zh_tw: "網路配置無效",
        en: "Invalid network configuration",
    },
    StaticMessage {
        code: 51005,
        zh_cn: "网卡不支持修改",
        zh_tw: "網路卡不支援修改",
        en: "Network interface is read-only",
    },
    StaticMessage {
        code: 51006,
        zh_cn: "已有进行中的网络变更操作",
        zh_tw: "已有進行中的網路變更操作",
        en: "A network change operation is already pending",
    },
    StaticMessage {
        code: 51007,
        zh_cn: "网卡不存在",
        zh_tw: "網路卡不存在",
        en: "Network interface not found",
    },
    StaticMessage {
        code: 51008,
        zh_cn: "网络服务执行失败",
        zh_tw: "網路服務執行失敗",
        en: "Network service execution failed",
    },
    StaticMessage {
        code: 51009,
        zh_cn: "网络操作超时",
        zh_tw: "網路操作超時",
        en: "Network operation timed out",
    },
    StaticMessage {
        code: 51010,
        zh_cn: "网络操作已过期，需重新提交",
        zh_tw: "網路操作已過期，需重新提交",
        en: "Network operation has expired",
    },
    StaticMessage {
        code: 51011,
        zh_cn: "检测到静态 IP 冲突",
        zh_tw: "檢測到靜態 IP 衝突",
        en: "Static IP conflict detected",
    },
    StaticMessage {
        code: 51012,
        zh_cn: "网关不可达或配置冲突",
        zh_tw: "閘道不可達或配置衝突",
        en: "Gateway unreachable or configuration conflict",
    },
    StaticMessage {
        code: 51100,
        zh_cn: "存储配置校验失败",
        zh_tw: "存儲配置校驗失敗",
        en: "Storage configuration validation failed",
    },
    StaticMessage {
        code: 51200,
        zh_cn: "时间配置校验失败",
        zh_tw: "時間配置校驗失敗",
        en: "Time configuration validation failed",
    },
    StaticMessage {
        code: 51201,
        zh_cn: "时间差超过一年",
        zh_tw: "時間差超過一年",
        en: "Time difference exceeds one year",
    },
    StaticMessage {
        code: 51202,
        zh_cn: "NTP 服务执行失败",
        zh_tw: "NTP 服務執行失敗",
        en: "NTP service execution failed",
    },
    StaticMessage {
        code: 51300,
        zh_cn: "系统信息读取失败",
        zh_tw: "系統資訊讀取失敗",
        en: "Failed to read system information",
    },
];

pub fn translate_system(code: u32, original_msg: &str, locale: Locale) -> Option<String> {
    let msg = SYSTEM_MESSAGES.iter().find(|m| m.code == code)?;
    if let Some((_, detail)) = original_msg.split_once(':') {
        Some(format!("{}: {}", msg.for_locale(locale), detail.trim()))
    } else {
        Some(msg.for_locale(locale).to_string())
    }
}
