//! 人脸检测插件的标准 `AlgoPlugin` 适配层。

use algo_sdk::emitter::ResultEmitter;
use algo_sdk::error::AlgoError;
use algo_sdk::frame::SafeFrame;
use algo_sdk::plugin::{AlgoPlugin, InitContext};

use crate::config::InstanceConfig;
use crate::detect::{decode_face_detections, nms, unmap_letterbox};
use crate::quality::compute_quality;

#[cfg(target_os = "macos")]
use std::sync::Arc;

#[cfg(target_os = "macos")]
use crate::coreml::CoreMlFaceModels;
#[cfg(target_os = "macos")]
use algo_sdk::cv::platforms::apple::AppleCvEngine;
#[cfg(target_os = "macos")]
use algo_sdk::cv::CvEngine;

#[cfg(target_os = "macos")]
#[derive(Debug)]
pub struct FaceRecognizer {
    pub models: Arc<CoreMlFaceModels>,
    pub config: InstanceConfig,
    pub tracker: crate::bytetrack::ByteTracker,
    pub best_shots: crate::best_shot::BestShotManager,
}

#[cfg(not(target_os = "macos"))]
#[derive(Debug)]
pub struct FaceRecognizer {
    pub config: InstanceConfig,
}

impl AlgoPlugin for FaceRecognizer {
    type Config = InstanceConfig;

    #[cfg(target_os = "macos")]
    fn init(ctx: &InitContext<'_>, config: Self::Config) -> Result<Self, AlgoError> {
        config
            .validate()
            .map_err(|reason| AlgoError::ConfigParse { reason })?;
        let models = crate::shared_models(ctx.package_root)?;
        let tracker =
            crate::bytetrack::ByteTracker::new(crate::bytetrack::ByteTrackConfig::default());
        let best_shots = crate::best_shot::BestShotManager::new();
        Ok(Self {
            models,
            config,
            tracker,
            best_shots,
        })
    }

    #[cfg(not(target_os = "macos"))]
    fn init(_ctx: &InitContext<'_>, config: Self::Config) -> Result<Self, AlgoError> {
        config
            .validate()
            .map_err(|reason| AlgoError::ConfigParse { reason })?;
        Ok(Self { config })
    }

    #[cfg(target_os = "macos")]
    fn process(
        &mut self,
        frame: SafeFrame<'_>,
        emitter: &mut ResultEmitter<'_>,
    ) -> Result<(), AlgoError> {
        let (buffer, mode) = AppleCvEngine.letterbox(&frame, 640, 384, [114, 114, 114])?;
        let pixelbuffer = buffer.as_raw_ptr().ok_or_else(|| AlgoError::Preprocess {
            reason: "CoreML 检测输入 CVPixelBuffer 指针为空".to_string(),
        })?;

        // 1. 人体检测 (yolo26n / 640x384)
        // SAFETY: pixelbuffer 由当前 CvBuffer 持有，直到预测返回前不会释放。
        let raw_person = unsafe { self.models.predict_person_detector(pixelbuffer)? };
        let mut persons = crate::detect::decode_person_detections(&raw_person, 0.40);
        crate::detect::nms_persons(&mut persons, 0.45);
        crate::detect::unmap_persons_letterbox(&mut persons, &mode, frame.width(), frame.height());

        // 2. 人脸检测 (yolov8-face / 640x384)
        // SAFETY: pixelbuffer 由当前 CvBuffer 持有，直到预测返回前不会释放。
        let raw_face_output = unsafe { self.models.predict_detector(pixelbuffer)? };
        let mut raw_faces =
            decode_face_detections(&raw_face_output, self.config.detection_confidence_threshold);
        nms(&mut raw_faces, 0.45);
        unmap_letterbox(&mut raw_faces, &mode, frame.width(), frame.height());

        // 3. 人体与人脸二分图空间几何挂载
        let associated = crate::association::associate_persons_and_faces(&persons, &raw_faces);

        // 4. ByteTrack 多目标航迹更新 (以人体为主航迹主体)
        let track_dets: Vec<crate::bytetrack::TrackDetection> = associated
            .iter()
            .map(|a| crate::bytetrack::TrackDetection {
                bbox: a.person_bbox,
                score: a.person_score,
                class_id: 0,
            })
            .collect();
        let active_tracks = self.tracker.update(&track_dets);

        // 5. 将活跃航迹映射回关联的人脸，并执行质量门控与动态择优抓拍
        let mut tracked_outputs = Vec::with_capacity(active_tracks.len());
        let mut active_track_ids = Vec::with_capacity(active_tracks.len());

        for track in &active_tracks {
            active_track_ids.push(track.track_id);

            // 在当前关联对中寻找重合度最高的匹配
            let best_assoc = associated
                .iter()
                .find(|a| crate::bytetrack::box_iou(&track.bbox, &a.person_bbox) >= 0.35);

            let attached_face = best_assoc.and_then(|a| a.attached_face);
            let is_pseudo = best_assoc.map(|a| a.is_pseudo_body).unwrap_or(false);

            let mut face_detail = None;
            if let Some(face) = attached_face {
                let quality = compute_quality(
                    &face.landmarks,
                    &face.landmark_scores,
                    face.bbox[2] * frame.width() as f32,
                    &self.config.quality_thresholds,
                );

                if quality.accepted(&self.config.quality_thresholds, self.config.min_face_size) {
                    let should_extract = self
                        .best_shots
                        .should_update_best_shot(track.track_id, &quality);
                    let mut embedding = None;
                    let is_best_shot = should_extract;

                    if should_extract {
                        // 记录最优抓拍状态（在流式感知阶段轻量标记，不执行重型特征提取）
                        self.best_shots.update(
                            track.track_id,
                            face.bbox,
                            face.landmarks,
                            face.score,
                            quality,
                            Vec::new(),
                            frame.frame_id() as usize,
                        );
                    } else if let Some(rec) = self.best_shots.get(track.track_id) {
                        embedding = rec.embedding_opt().map(|s| s.to_vec());
                    }

                    face_detail = Some(crate::postprocess::FaceDetail {
                        bbox: face.bbox,
                        landmarks: face.landmarks,
                        detection_score: face.score,
                        quality,
                        embedding,
                        is_best_shot,
                    });
                }
            }

            tracked_outputs.push(crate::postprocess::TrackedPersonOutput {
                track_id: track.track_id,
                person_bbox: track.bbox,
                person_score: track.score,
                is_pseudo_body: is_pseudo,
                face: face_detail,
            });
        }

        // 级联清理失活航迹
        self.best_shots.retain_active_tracks(&active_track_ids);

        crate::postprocess::emit_tracked_results(emitter, &tracked_outputs)
    }

    #[cfg(not(target_os = "macos"))]
    fn process(
        &mut self,
        _frame: SafeFrame<'_>,
        _emitter: &mut ResultEmitter<'_>,
    ) -> Result<(), AlgoError> {
        Err(AlgoError::Internal {
            reason: "macOS CoreML 人脸识别插件仅支持在 macOS 平台运行".to_string(),
        })
    }

    fn update_config(&mut self, config: Self::Config) -> Result<(), AlgoError> {
        config
            .validate()
            .map_err(|reason| AlgoError::ConfigParse { reason })?;
        self.config = config;
        Ok(())
    }

    fn flush(&mut self, _emitter: &mut ResultEmitter<'_>) -> Result<(), AlgoError> {
        #[cfg(target_os = "macos")]
        {
            self.tracker.reset();
            self.best_shots.clear();
        }
        Ok(())
    }
}
