use serde::Serialize;
use uuid::Uuid;

use algo_sdk::emitter::ResultEmitter;
use algo_sdk::error::AlgoError;

use crate::quality::FaceQuality;

#[derive(Debug, Clone, Serialize)]
pub struct FaceDetection {
    pub bbox: [f32; 4],
    pub landmarks: [[f32; 2]; 5],
    pub detection_score: f32,
    pub quality: FaceQuality,
}

#[derive(Debug, Serialize)]
struct FaceRecognitionEnvelope<'a> {
    event_id: Uuid,
    faces: &'a [FaceDetection],
}

/// 序列化并发射 `AV_RESULT_RECOGNITION` 结果。
pub fn emit_face_detections(
    emitter: &mut ResultEmitter<'_>,
    faces: &[FaceDetection],
) -> Result<(), AlgoError> {
    if faces.is_empty() {
        return Ok(());
    }
    let envelope = FaceRecognitionEnvelope {
        event_id: Uuid::new_v4(),
        faces,
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
    fn recognition_payload_contains_landmarks_and_quality() {
        let mut captured = String::new();
        // SAFETY: 回调和 user_data 在 emitter 生命周期内保持有效。
        let mut emitter = unsafe {
            ResultEmitter::from_raw(
                1,
                Some(capture_result),
                (&mut captured as *mut String).cast::<c_void>(),
            )
        };
        let face = FaceDetection {
            bbox: [0.1, 0.2, 0.3, 0.4],
            landmarks: [[0.2, 0.3]; 5],
            detection_score: 0.95,
            quality: FaceQuality {
                score: 0.8,
                yaw: 1.0,
                pitch: -2.0,
                blur: 0.1,
                face_size: 96,
            },
        };
        emit_face_detections(&mut emitter, &[face]).expect("结果应发射成功");
        let value: serde_json::Value = serde_json::from_str(&captured).expect("JSON 应合法");
        assert_eq!(value["faces"][0]["quality"]["face_size"], 96);
        assert_eq!(
            value["faces"][0]["landmarks"].as_array().map(Vec::len),
            Some(5)
        );
    }

    #[test]
    fn empty_faces_do_not_emit() {
        let mut called = false;
        unsafe extern "C" fn callback(_result: *const AvAlgoResult, user_data: *mut c_void) {
            // SAFETY: 测试 user_data 指向有效 bool。
            unsafe { *(user_data as *mut bool) = true };
        }
        // SAFETY: 回调和 user_data 在 emitter 生命周期内保持有效。
        let mut emitter = unsafe {
            ResultEmitter::from_raw(
                1,
                Some(callback),
                (&mut called as *mut bool).cast::<c_void>(),
            )
        };
        emit_face_detections(&mut emitter, &[]).expect("空结果应是成功 no-op");
        assert!(!called);
    }
}
