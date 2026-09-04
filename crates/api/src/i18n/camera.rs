use super::{Locale, StaticMessage};

pub const STATIC_MESSAGES: &[StaticMessage] = &[
    StaticMessage {
        code: 20001,
        zh_cn: "摄像头 RTSP 连接失败",
        zh_tw: "攝影機 RTSP 連線失敗",
        en: "Camera RTSP connection failed",
    },
    StaticMessage {
        code: 20002,
        zh_cn: "RTSP 码流协议交互异常",
        zh_tw: "RTSP 串流協議互動異常",
        en: "RTSP stream protocol error",
    },
    StaticMessage {
        code: 20003,
        zh_cn: "视频 SPS 参数集解析失败",
        zh_tw: "視訊 SPS 參數集解析失敗",
        en: "Failed to parse video SPS parameter set",
    },
    StaticMessage {
        code: 20004,
        zh_cn: "摄像头探活超时",
        zh_tw: "攝影機探活超時",
        en: "Camera probe timed out",
    },
    StaticMessage {
        code: 20005,
        zh_cn: "硬件解码器初始化失败",
        zh_tw: "硬體解碼器初始化失敗",
        en: "Hardware decoder initialization failed",
    },
    StaticMessage {
        code: 20006,
        zh_cn: "视频帧硬件解码失败",
        zh_tw: "視訊幀硬體解碼失敗",
        en: "Video frame hardware decode failed",
    },
    StaticMessage {
        code: 20007,
        zh_cn: "不支持的视频编解码格式",
        zh_tw: "不支援的視訊編解碼格式",
        en: "Unsupported video codec format",
    },
    StaticMessage {
        code: 20008,
        zh_cn: "摄像头流会话未找到",
        zh_tw: "攝影機串流會話未找到",
        en: "Camera stream session not found",
    },
    StaticMessage {
        code: 20009,
        zh_cn: "底层硬件帧句柄错误",
        zh_tw: "底層硬體幀句柄錯誤",
        en: "Underlying hardware frame handle error",
    },
    StaticMessage {
        code: 20010,
        zh_cn: "网络与系统 IO 错误",
        zh_tw: "網路與系統 IO 錯誤",
        en: "Network and system IO error",
    },
];

pub fn translate_camera(code: u32, _original_msg: &str, locale: Locale) -> Option<String> {
    if let Some(entry) = STATIC_MESSAGES.iter().find(|m| m.code == code) {
        return Some(entry.for_locale(locale).to_string());
    }
    None
}
