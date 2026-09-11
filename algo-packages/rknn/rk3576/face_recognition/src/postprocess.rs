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

/// 挂载在航迹上的人脸详情
#[derive(Debug, Clone, Serialize)]
pub struct FaceDetail {
    pub bbox: [f32; 4],
    pub landmarks: [[f32; 2]; 5],
    pub detection_score: f32,
    pub quality: FaceQuality,
    pub embedding: Option<Vec<f32>>,
    pub is_best_shot: bool,
}

/// ByteTrack 输出的带挂载人脸的人体航迹
#[derive(Debug, Clone, Serialize)]
pub struct TrackedPersonOutput {
    pub track_id: u64,
    pub person_bbox: [f32; 4],
    pub person_score: f32,
    pub is_pseudo_body: bool,
    pub face: Option<FaceDetail>,
}

/// 兼容 Engine 规则引擎的目标对象
#[derive(Debug, Clone, Serialize)]
pub struct CompatibleObject {
    pub track_id: u64,
    pub class_id: usize,
    pub label: String,
    pub confidence: f32,
    pub bbox: [f32; 4],
}

#[derive(Debug, Serialize)]
struct FaceRecognitionEnvelope<'a> {
    event_id: Uuid,
    tracks: &'a [TrackedPersonOutput],
    faces: Vec<FaceDetection>,
    objects: Vec<CompatibleObject>,
}

/// 序列化并发射完整的人体航迹与人脸识别结果。
pub fn emit_tracked_results(
    emitter: &mut ResultEmitter<'_>,
    tracks: &[TrackedPersonOutput],
) -> Result<(), AlgoError> {
    if tracks.is_empty() {
        return Ok(());
    }

    let mut faces = Vec::new();
    let mut objects = Vec::with_capacity(tracks.len());

    for t in tracks {
        objects.push(CompatibleObject {
            track_id: t.track_id,
            class_id: 0,
            label: "face".to_string(),
            confidence: t.person_score,
            bbox: t.person_bbox,
        });

        if let Some(ref face) = t.face {
            faces.push(FaceDetection {
                bbox: face.bbox,
                landmarks: face.landmarks,
                detection_score: face.detection_score,
                quality: face.quality,
            });
        }
    }

    let envelope = FaceRecognitionEnvelope {
        event_id: Uuid::new_v4(),
        tracks,
        faces,
        objects,
    };
    let json = serde_json::to_vec(&envelope).map_err(|error| AlgoError::Internal {
        reason: format!("序列化人脸识别结果失败: {error}"),
    })?;
    emitter.emit_recognition_json(&json)
}

/// 兼容性序列化并发射原纯人脸格式 `AV_RESULT_RECOGNITION` 结果。
pub fn emit_face_detections(
    emitter: &mut ResultEmitter<'_>,
    faces: &[FaceDetection],
) -> Result<(), AlgoError> {
    if faces.is_empty() {
        return Ok(());
    }
    let tracks: Vec<TrackedPersonOutput> = faces
        .iter()
        .enumerate()
        .map(|(idx, f)| TrackedPersonOutput {
            track_id: (idx + 1) as u64,
            person_bbox: f.bbox,
            person_score: f.detection_score,
            is_pseudo_body: true,
            face: Some(FaceDetail {
                bbox: f.bbox,
                landmarks: f.landmarks,
                detection_score: f.detection_score,
                quality: f.quality,
                embedding: None,
                // 常驻检测路径不做 readback/embedding；抓拍接口单独返回 best shot。
                is_best_shot: false,
            }),
        })
        .collect();

    emit_tracked_results(emitter, &tracks)
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

        let faces = vec![FaceDetection {
            bbox: [0.1, 0.2, 0.4, 0.6],
            landmarks: [[0.15, 0.25]; 5],
            detection_score: 0.95,
            quality: FaceQuality {
                score: 0.88,
                blur: 0.12,
                yaw: 1.5,
                pitch: -2.0,
                face_size: 160,
            },
        }];

        emit_face_detections(&mut emitter, &faces).expect("发射应成功");
        assert!(captured.contains("\"faces\""));
        assert!(captured.contains("\"tracks\""));
        assert!(captured.contains("\"objects\""));
        assert!(captured.contains("\"track_id\":1"));
    }
}
