use super::{Locale, StaticMessage};

pub const STATIC_MESSAGES: &[StaticMessage] = &[
    StaticMessage {
        code: 50301,
        zh_cn: "人脸识别算法包未就绪，无法提取特征，请先部署/激活人脸算法",
        zh_tw: "人臉識別演算法包未就緒，無法擷取特徵，請先部署/啟用人臉演算法",
        en: "Face recognition algorithm package is not ready. Please deploy or activate a face algorithm package first.",
    },
    StaticMessage {
        code: 40902,
        zh_cn: "当前有全量底库特征重新提取任务正在后台执行中，请稍候再试",
        zh_tw: "當前有全量底庫特徵重新擷取任務正在背景執行中，請稍候再試",
        en: "A gallery face feature re-extraction task is currently running in the background, please try again later.",
    },
];

pub fn translate_personnel(code: u32, original_msg: &str, locale: Locale) -> Option<String> {
    if let Some(entry) = STATIC_MESSAGES.iter().find(|m| m.code == code) {
        return Some(entry.for_locale(locale).to_string());
    }

    match code {
        40001 => {
            if original_msg.contains("人员姓名不能为空") || original_msg.contains("姓名不能为空")
            {
                return match locale {
                    Locale::ZhCn => Some("姓名不能为空".to_string()),
                    Locale::ZhTw => Some("姓名不能為空".to_string()),
                    Locale::En => Some("Person name cannot be empty".to_string()),
                };
            }
            if original_msg.contains("人员编号不能为空") {
                return match locale {
                    Locale::ZhCn => Some("人员编号不能为空".to_string()),
                    Locale::ZhTw => Some("人員編號不能為空".to_string()),
                    Locale::En => Some("Subject ID cannot be empty".to_string()),
                };
            }
            if original_msg.contains("必须提供至少一张人脸照片") {
                return match locale {
                    Locale::ZhCn => Some("必须提供至少一张人脸照片".to_string()),
                    Locale::ZhTw => Some("必須提供至少一張人臉照片".to_string()),
                    Locale::En => Some("At least one face photo must be provided".to_string()),
                };
            }
            if original_msg.contains("人员最多关联 5 张人脸照片") {
                return match locale {
                    Locale::ZhCn => Some(original_msg.to_string()),
                    Locale::ZhTw => Some(
                        original_msg
                            .replace("人员最多关联 5 张人脸照片", "人員最多關聯 5 張人臉照片")
                            .replace("当前已有", "當前已有")
                            .replace("张", "張")
                            .replace("本次尝试追加", "本次嘗試追加"),
                    ),
                    Locale::En => {
                        Some("A person can have at most 5 face photos in total.".to_string())
                    }
                };
            }
            if original_msg.contains("人员至少保留 1 张人脸样本") {
                return match locale {
                    Locale::ZhCn => Some("人员至少保留 1 张人脸样本，无法删除最后一张照片".to_string()),
                    Locale::ZhTw => Some("人員至少保留 1 張人臉樣本，無法刪除最後一張照片".to_string()),
                    Locale::En => Some("Each person must keep at least one face photo. The last photo cannot be deleted.".to_string()),
                };
            }
            if original_msg.contains("照片样本不属于该人员") {
                return match locale {
                    Locale::ZhCn => Some("照片样本不属于该人员".to_string()),
                    Locale::ZhTw => Some("照片樣本不屬於該人員".to_string()),
                    Locale::En => {
                        Some("The face sample does not belong to this person".to_string())
                    }
                };
            }
            if original_msg.contains("未选择要追加的照片") {
                return match locale {
                    Locale::ZhCn => Some("未选择要追加的照片".to_string()),
                    Locale::ZhTw => Some("未選擇要追加的照片".to_string()),
                    Locale::En => Some("No face photos selected for upload".to_string()),
                };
            }
            if original_msg.contains("人员编号已存在") {
                let id = original_msg
                    .split(':')
                    .nth(1)
                    .map(|s| s.trim())
                    .unwrap_or("");
                return match locale {
                    Locale::ZhCn => Some(format!("人员编号已存在: {id}")),
                    Locale::ZhTw => Some(format!("人員編號已存在: {id}")),
                    Locale::En => Some(format!("Subject ID already exists: {id}")),
                };
            }
            if original_msg.contains("不支持的图片格式") {
                return match locale {
                    Locale::ZhCn => Some(original_msg.to_string()),
                    Locale::ZhTw => Some(
                        original_msg
                            .replace("不支持的图片格式或文件损坏", "不支援的圖片格式或檔案損毀"),
                    ),
                    Locale::En => Some("Unsupported image format or corrupted file".to_string()),
                };
            }
            None
        }
        40002 => {
            if original_msg.contains("未在") && original_msg.contains("检测到有效人脸") {
                return match locale {
                    Locale::ZhCn => Some("未在上传照片中检测到有效人脸，请上传正面清晰免冠照".to_string()),
                    Locale::ZhTw => Some("未在上傳照片中檢測到有效人臉，請上傳正面清晰免冠照".to_string()),
                    Locale::En => Some("No valid face detected in the photo. Please upload a clear frontal photo without hat or glasses.".to_string()),
                };
            }
            if original_msg.contains("人脸质量评分过低") {
                return match locale {
                    Locale::ZhCn => Some(original_msg.to_string()),
                    Locale::ZhTw => Some(
                        original_msg
                            .replace("人脸质量评分过低", "人臉質量評分過低")
                            .replace("未满足门禁要求", "未滿足門禁要求")
                            .replace("请上传光线充足的正面照片", "請上傳光線充足的正面照片")
                            .replace("未达到门禁标准", "未達到門禁標準"),
                    ),
                    Locale::En => Some("Face quality score is too low (< 0.50). Please upload a well-lit, clear frontal photo.".to_string()),
                };
            }
            if original_msg.contains("人脸特征提取失败") {
                return match locale {
                    Locale::ZhCn => Some(original_msg.to_string()),
                    Locale::ZhTw => {
                        Some(original_msg.replace("人脸特征提取失败", "人臉特徵擷取失敗"))
                    }
                    Locale::En => Some(format!(
                        "Face feature extraction failed: {}",
                        original_msg.replace("人脸特征提取失败: ", "")
                    )),
                };
            }
            None
        }
        40401 => {
            if original_msg.contains("人员不存在") {
                let id = original_msg
                    .split(':')
                    .nth(1)
                    .map(|s| s.trim())
                    .unwrap_or("");
                return match locale {
                    Locale::ZhCn => Some(format!("人员不存在: {id}")),
                    Locale::ZhTw => Some(format!("人員不存在: {id}")),
                    Locale::En => Some(format!("Person not found: {id}")),
                };
            }
            if original_msg.contains("照片样本不存在") || original_msg.contains("人脸样本不存在")
            {
                let id = original_msg
                    .split(':')
                    .nth(1)
                    .map(|s| s.trim())
                    .unwrap_or("");
                return match locale {
                    Locale::ZhCn => Some(format!("照片样本不存在: {id}")),
                    Locale::ZhTw => Some(format!("照片樣本不存在: {id}")),
                    Locale::En => Some(format!("Face photo not found: {id}")),
                };
            }
            None
        }
        _ => None,
    }
}
