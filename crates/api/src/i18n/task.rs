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
    StaticMessage {
        code: 30004,
        zh_cn: "摄像头分析管线已存在",
        zh_tw: "攝影機分析管線已存在",
        en: "Camera analysis pipeline already exists",
    },
    StaticMessage {
        code: 30005,
        zh_cn: "领域类型校验失败",
        zh_tw: "領域類型校驗失敗",
        en: "Domain type validation failed",
    },
    StaticMessage {
        code: 30011,
        zh_cn: "AI 模型文件加载失败",
        zh_tw: "AI 模型檔案加載失敗",
        en: "Failed to load AI model file",
    },
    StaticMessage {
        code: 30012,
        zh_cn: "算法包动态库加载失败",
        zh_tw: "演算法包動態庫加載失敗",
        en: "Failed to load algorithm dynamic library",
    },
    StaticMessage {
        code: 30013,
        zh_cn: "算法导出符号寻址失败",
        zh_tw: "演算法導出符號定址失敗",
        en: "Failed to lookup algorithm exported symbol",
    },
    StaticMessage {
        code: 30014,
        zh_cn: "算法 C ABI 虚表不匹配或无效",
        zh_tw: "演算法 C ABI 虛表不相容或無效",
        en: "Incompatible or invalid algorithm C ABI interface",
    },
    StaticMessage {
        code: 30015,
        zh_cn: "算法底层运行时执行异常",
        zh_tw: "演算法底層運行時執行異常",
        en: "Algorithm runtime execution error",
    },
    StaticMessage {
        code: 30016,
        zh_cn: "算法包沙箱安全校验失败",
        zh_tw: "演算法包沙箱安全校驗失敗",
        en: "Algorithm sandbox security validation failed",
    },
    StaticMessage {
        code: 30017,
        zh_cn: "算法元数据解析失败",
        zh_tw: "演算法元數據解析失敗",
        en: "Failed to parse algorithm metadata",
    },
    StaticMessage {
        code: 30018,
        zh_cn: "输入张量形状不匹配",
        zh_tw: "輸入張量形狀不匹配",
        en: "Input tensor shape mismatch",
    },
    StaticMessage {
        code: 30019,
        zh_cn: "当前平台推理后端不可用",
        zh_tw: "當前平台推理後端不可用",
        en: "Inference backend unavailable on current platform",
    },
    StaticMessage {
        code: 30020,
        zh_cn: "算法硬件推理超时",
        zh_tw: "演算法硬體推理超時",
        en: "Algorithm hardware inference timed out",
    },
    StaticMessage {
        code: 30021,
        zh_cn: "算法前向推理执行失败",
        zh_tw: "演算法前向推理執行失敗",
        en: "Algorithm forward inference execution failed",
    },
    StaticMessage {
        code: 30022,
        zh_cn: "底层视频帧句柄错误",
        zh_tw: "底層視訊幀句柄錯誤",
        en: "Underlying video frame handle error",
    },
];

pub fn translate_task(code: u32, _original_msg: &str, locale: Locale) -> Option<String> {
    if let Some(entry) = STATIC_MESSAGES.iter().find(|m| m.code == code) {
        return Some(entry.for_locale(locale).to_string());
    }
    None
}
