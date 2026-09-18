use serde::Serialize;

use algo_sdk::emitter::ResultEmitter;
use algo_sdk::error::AlgoError;
use algo_sdk::math::{box_xywh_to_xyxy, encode_embedding_base64_le};

/// 挂载在主体人员目标上的精细人脸详情对象。
#[derive(Debug, Clone, Serialize)]
pub struct FaceDetailObject {
    pub bbox: [f32; 4],
    pub confidence: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality_score: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding: Option<String>,
    /// 当前模板参与融合的非冗余帧数。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fused_count: Option<u32>,
    /// 当前模板参与融合帧的质量加权均值。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_quality: Option<f32>,
    /// 仅在模板首次成熟的帧上发射 `true`，作为宿主结算握手。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_mature: Option<bool>,
    /// 承载人脸的人体框是由人脸几何推导的虚拟躯干（画面中未检出对应人体）时为 `true`。
    ///
    /// 合成躯干底部常被钉在画面下沿，宿主若直接拿它做空间规则判定（ROI 侵入、
    /// 折线越界）会产生与真实人体无关的告警，因此需要把这个事实透出到 ABI JSON。
    /// 只在该帧置位（`Some(true)`），普通人体框不上报，避免无意义字节。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pseudo_body: Option<bool>,
}

/// 宿主检测解析器消费的稳定目标对象。
///
/// `bbox` 为人体主体框，`face` 为挂载的人脸检测详情。
/// 均使用归一化 `[x1, y1, x2, y2]`，而检测阶段的内部人体框仍使用
/// `[x, y, width, height]`。转换集中在本模块，避免把坐标约定泄漏到调用方。
#[derive(Debug, Clone, Serialize)]
pub struct DetectionObject {
    pub class_id: usize,
    pub label: String,
    pub confidence: f32,
    pub bbox: [f32; 4],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub face: Option<FaceDetailObject>,
}

#[derive(Debug, Serialize)]
struct RecognitionEnvelope<'a> {
    schema_version: u32,
    objects: &'a [DetectionObject],
}

pub const RECOGNITION_SCHEMA_VERSION: u32 = 1;

/// 将归一化 `xywh` 转为宿主契约要求的归一化 `xyxy`。
#[inline]
pub fn normalized_xywh_to_xyxy(bbox: [f32; 4]) -> [f32; 4] {
    box_xywh_to_xyxy(bbox)
}

pub fn encode_embedding(embedding: &[f32]) -> Result<String, AlgoError> {
    if embedding.len() != 512 {
        return Err(AlgoError::Inference {
            reason: format!("embedding 维度非法: 期望 512，实际 {}", embedding.len()),
        });
    }

    encode_embedding_base64_le(embedding).map_err(|e| AlgoError::Inference {
        reason: format!("特征向量编码失败: {e}"),
    })
}

/// 序列化并发射符合女娲标准规范的人脸识别结果。
///
/// 空对象列表也是有效结果，代表当前帧没有通过人脸质量门控的目标。
/// 该函数不携带插件内部航迹、事件 ID 或非必要的特征向量。
pub fn emit_detection_objects(
    emitter: &mut ResultEmitter<'_>,
    objects: &[DetectionObject],
) -> Result<(), AlgoError> {
    let envelope = RecognitionEnvelope {
        schema_version: RECOGNITION_SCHEMA_VERSION,
        objects,
    };
    let json = serde_json::to_vec(&envelope).map_err(|error| AlgoError::Internal {
        reason: format!("序列化人脸识别结果失败: {error}"),
    })?;
    emitter.emit_recognition_json(&json)
}

#[cfg(test)]
mod tests {
    use std::ffi::{c_void, CStr};

    use algo_sdk::c_abi::{AvAlgoResult, AV_RESULT_RECOGNITION};
    use algo_sdk::emitter::ResultEmitter;

    use super::*;

    unsafe extern "C" fn capture_result(result: *const AvAlgoResult, user_data: *mut c_void) {
        // SAFETY: 测试回调只在 emitter 同步调用期间读取有效结果指针。
        let result = unsafe { &*result };
        assert_eq!(result.kind, AV_RESULT_RECOGNITION);
        // SAFETY: ResultEmitter 保证 JSON 在回调期间以 NUL 结尾且有效。
        let json = unsafe { CStr::from_ptr(result.json) };
        // SAFETY: user_data 指向测试中持有的 String。
        let captured = unsafe { &mut *(user_data as *mut String) };
        *captured = json.to_string_lossy().into_owned();
    }

    #[test]
    fn converts_normalized_xywh_to_xyxy() {
        assert_eq!(
            normalized_xywh_to_xyxy([0.1, 0.2, 0.3, 0.4]),
            [0.1, 0.2, 0.4, 0.6]
        );
        assert_eq!(
            normalized_xywh_to_xyxy([-0.2, 0.1, 1.5, 1.2]),
            [0.0, 0.1, 1.0, 1.0]
        );
    }

    #[test]
    fn emits_only_standard_detection_objects() {
        let mut captured = String::new();
        // SAFETY: 回调和 user_data 在 emitter 生命周期内保持有效。
        let mut emitter = unsafe {
            ResultEmitter::from_raw(
                1,
                Some(capture_result),
                (&mut captured as *mut String).cast::<c_void>(),
            )
        };

        let objects = vec![DetectionObject {
            class_id: 0,
            label: "person".to_string(),
            confidence: 0.95,
            bbox: normalized_xywh_to_xyxy([0.1, 0.2, 0.3, 0.4]),
            face: Some(FaceDetailObject {
                bbox: [0.15, 0.22, 0.25, 0.35],
                confidence: 0.92,
                quality_score: Some(0.88),
                embedding: None,
                fused_count: None,
                template_quality: None,
                template_mature: None,
                pseudo_body: None,
            }),
        }];

        emit_detection_objects(&mut emitter, &objects).expect("发射应成功");
        assert!(captured.contains("\"schema_version\":1"));
        assert!(captured.contains("\"objects\""));
        assert!(captured.contains("\"label\":\"person\""));
        assert!(captured.contains("\"bbox\":[0.1,0.2,0.4,0.6]"));
        assert!(captured.contains("\"face\":{"));
        assert!(captured.contains("\"quality_score\":0.88"));
        assert!(captured.contains("\"bbox\":[0.15,0.22,0.25,0.35]"));
        assert!(!captured.contains("tracks"));
        assert!(!captured.contains("track_id"));
        assert!(!captured.contains("is_pseudo_body"));
        // 普通人体框不得携带 pseudo_body 标记（只在该帧置位，避免噪声字节）。
        assert!(!captured.contains("pseudo_body"));
        assert!(!captured.contains("embedding"));
    }

    #[test]
    fn encodes_embedding_only_when_present() {
        let encoded = encode_embedding(&[0.25; 512]).expect("embedding 编码应成功");
        assert_eq!(encoded.len(), 2732);

        let object = DetectionObject {
            class_id: 0,
            label: "person".to_string(),
            confidence: 0.9,
            bbox: [0.1, 0.2, 0.4, 0.6],
            face: Some(FaceDetailObject {
                bbox: [0.15, 0.22, 0.25, 0.35],
                confidence: 0.92,
                quality_score: Some(0.9),
                embedding: Some(encoded),
                fused_count: Some(2),
                template_quality: Some(0.86),
                template_mature: Some(true),
                pseudo_body: Some(true),
            }),
        };
        let json = serde_json::to_string(&object).expect("目标序列化应成功");
        assert!(json.contains("face"));
        assert!(json.contains("embedding"));
        assert!(json.contains("\"fused_count\":2"));
        assert!(json.contains("\"template_quality\":0.86"));
        assert!(json.contains("\"template_mature\":true"));
        assert!(json.contains("\"pseudo_body\":true"));
    }

    #[test]
    fn emits_empty_successful_result() {
        let mut captured = String::new();
        // SAFETY: 回调和 user_data 在 emitter 生命周期内保持有效。
        let mut emitter = unsafe {
            ResultEmitter::from_raw(
                1,
                Some(capture_result),
                (&mut captured as *mut String).cast::<c_void>(),
            )
        };

        emit_detection_objects(&mut emitter, &[]).expect("空结果也应发射");
        assert_eq!(captured, r#"{"schema_version":1,"objects":[]}"#);
    }
}
