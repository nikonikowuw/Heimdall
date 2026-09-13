//! 人脸检测插件的标准 `AlgoPlugin` 适配层。

use algo_sdk::emitter::ResultEmitter;
use algo_sdk::error::AlgoError;
use algo_sdk::frame::SafeFrame;
use algo_sdk::plugin::{AlgoPlugin, InitContext};

use crate::config::InstanceConfig;

#[cfg(target_os = "macos")]
use std::sync::Arc;

#[cfg(target_os = "macos")]
use crate::align::face_alignment_matrix;
#[cfg(target_os = "macos")]
use crate::coreml::CoreMlFaceModels;
#[cfg(target_os = "macos")]
use crate::detect::{decode_face_detections, nms, unmap_letterbox};
#[cfg(target_os = "macos")]
use crate::normalize_embedding;
#[cfg(target_os = "macos")]
use crate::quality::compute_quality;
#[cfg(target_os = "macos")]
use algo_sdk::cv::platforms::apple::AppleCvEngine;
#[cfg(target_os = "macos")]
use algo_sdk::cv::CvEngine;

#[cfg(target_os = "macos")]
#[derive(Debug)]
pub struct FaceRecognizer {
    pub models: Arc<CoreMlFaceModels>,
    pub config: InstanceConfig,
    /// 仅用于当前算法实例内部的 best-shot 去重，不向 C ABI/宿主输出 trackId。
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
    fn init(ctx: &InitContext<'_>, mut config: Self::Config) -> Result<Self, AlgoError> {
        let env = ctx.load_env();
        config.apply_env(&env);
        config
            .validate()
            .map_err(|reason| AlgoError::ConfigParse { reason })?;
        let models = crate::shared_models(ctx.package_root)?;
        Ok(Self {
            models,
            config,
            tracker: crate::bytetrack::ByteTracker::new(
                crate::bytetrack::ByteTrackConfig::default(),
            ),
            best_shots: crate::best_shot::BestShotManager::new(),
        })
    }

    #[cfg(not(target_os = "macos"))]
    fn init(ctx: &InitContext<'_>, mut config: Self::Config) -> Result<Self, AlgoError> {
        let env = ctx.load_env();
        config.apply_env(&env);
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
        let source_pixelbuffer = match frame.handle_view() {
            algo_sdk::frame::FrameHandleView::ApplePixelBuffer { ptr } => ptr,
            _ => {
                tracing::warn!(
                    "best-shot EdgeFace 需要原生 Apple CVPixelBuffer，当前帧跳过 embedding"
                );
                std::ptr::null_mut()
            }
        };
        let (buffer, mode) = AppleCvEngine.letterbox(&frame, 640, 384, [114, 114, 114])?;
        let pixelbuffer = buffer.as_raw_ptr().ok_or_else(|| AlgoError::Preprocess {
            reason: "CoreML 检测输入 CVPixelBuffer 指针为空".to_string(),
        })?;

        // yolo26n 和人脸模型共享同一个平台侧预处理缓冲区。两次推理都在
        // 当前 process 调用内完成，pixelbuffer 不会逃逸到异步任务或插件状态。
        // SAFETY: pixelbuffer 由当前 CvBuffer 持有，直到两次同步预测返回前有效。
        let raw_person = unsafe { self.models.predict_person_detector(pixelbuffer)? };
        // SAFETY: pixelbuffer 仍由当前 CvBuffer 持有，且 predict_detector 同步执行。
        let raw_face_output = unsafe { self.models.predict_detector(pixelbuffer)? };

        let mut persons = crate::detect::decode_person_detections(&raw_person, 0.40);
        crate::detect::nms_persons(&mut persons, 0.45);
        crate::detect::unmap_persons_letterbox(&mut persons, &mode, frame.width(), frame.height());

        let mut raw_faces =
            decode_face_detections(&raw_face_output, self.config.detection_confidence_threshold);
        nms(&mut raw_faces, 0.45);
        unmap_letterbox(&mut raw_faces, &mode, frame.width(), frame.height());

        // 关联是当前帧的空间操作。宿主仍然拥有对外 TrackDto 使用的 trackId；
        // 这里的私有 ByteTrack 只为 best-shot 质量状态提供稳定键，不进入结果 JSON。
        let associated = crate::association::associate_persons_and_faces(&persons, &raw_faces);
        let track_dets: Vec<crate::bytetrack::TrackDetection> = associated
            .iter()
            .map(|candidate| crate::bytetrack::TrackDetection {
                bbox: candidate.person_bbox,
                score: candidate.person_score,
                class_id: 0,
            })
            .collect();
        let active_tracks = self.tracker.update(&track_dets);
        let matched_assocs =
            crate::association::match_tracks_to_associated(&active_tracks, &associated, 0.30);
        let active_track_ids: Vec<u64> = active_tracks.iter().map(|track| track.track_id).collect();
        let mut objects = Vec::with_capacity(active_tracks.len());

        for (track, best_assoc) in active_tracks.iter().zip(matched_assocs) {
            let Some(candidate) = best_assoc else {
                continue;
            };

            let face_detail = if let Some(face) = candidate.attached_face {
                let quality = compute_quality(
                    &face.landmarks,
                    &face.landmark_scores,
                    face.bbox[2] * frame.width() as f32,
                    &self.config.quality_thresholds,
                );
                if quality.accepted(&self.config.quality_thresholds, self.config.min_face_size) {
                    // best-shot 仍由质量门控控制频率，但提取在当前算法 worker 内同步完成：
                    // CVPixelBuffer -> Core Image affine warp -> CVPixelBuffer -> CoreML/ANE。
                    // 不执行整帧 D2H readback、CPU RGB 重排或 JPEG 往返。
                    let embedding = if !source_pixelbuffer.is_null()
                        && self.best_shots.should_update_best_shot(
                            track.track_id,
                            &quality,
                            frame.frame_id() as usize,
                        ) {
                        match face_alignment_matrix(frame.width(), frame.height(), &face.landmarks)
                            .map_err(|error| error.to_string())
                            .and_then(|matrix| {
                                // SAFETY: source_pixelbuffer 来自当前 SafeFrame，且本次调用同步完成；
                                // EdgeFace 不会保存该裸指针或把它交给异步任务。
                                unsafe {
                                    self.models
                                        .predict_embedding_from_pixelbuffer(
                                            source_pixelbuffer,
                                            frame.width(),
                                            frame.height(),
                                            matrix,
                                        )
                                        .map_err(|error| error.to_string())
                                }
                            })
                            .and_then(|values| {
                                normalize_embedding(&values).map_err(|error| error.to_string())
                            }) {
                            Ok(normalized) => {
                                let fused = self.best_shots.update_with_fusion(
                                    track.track_id,
                                    face.bbox,
                                    face.landmarks,
                                    face.score,
                                    quality,
                                    &normalized,
                                    frame.frame_id() as usize,
                                );
                                Some(fused)
                            }
                            Err(error) => {
                                self.best_shots.record_attempt_without_embedding(
                                    track.track_id,
                                    face.bbox,
                                    face.landmarks,
                                    face.score,
                                    quality,
                                    frame.frame_id() as usize,
                                );
                                tracing::warn!(
                                    error = %error,
                                    "best-shot CoreML 设备侧 EdgeFace 提取失败，保留检测结果并允许后续重试"
                                );
                                None
                            }
                        }
                    } else {
                        None
                    };

                    let embedding_str = embedding
                        .as_ref()
                        .map(|value| crate::postprocess::encode_embedding(value.as_slice()))
                        .transpose()?;

                    Some(crate::postprocess::FaceDetailObject {
                        bbox: crate::postprocess::normalized_xywh_to_xyxy(face.bbox),
                        confidence: face.score.clamp(0.0, 1.0),
                        quality_score: Some(quality.score.clamp(0.0, 1.0)),
                        embedding: embedding_str,
                    })
                } else {
                    None
                }
            } else {
                None
            };

            objects.push(crate::postprocess::DetectionObject {
                class_id: 0,
                label: "person".to_string(),
                confidence: candidate.person_score.clamp(0.0, 1.0),
                bbox: crate::postprocess::normalized_xywh_to_xyxy(candidate.person_bbox),
                face: face_detail,
            });
        }

        self.best_shots.retain_active_tracks(&active_track_ids);
        crate::postprocess::emit_detection_objects(emitter, &objects)
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
