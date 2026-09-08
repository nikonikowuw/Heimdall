//! 人脸检测插件的标准 `AlgoPlugin` 适配层

use std::sync::Arc;

use algo_sdk::cv::platforms::rockchip::RgaCvEngine;
use algo_sdk::cv::{CvEngine, PreprocessMode};
use algo_sdk::emitter::ResultEmitter;
use algo_sdk::error::AlgoError;
use algo_sdk::frame::SafeFrame;
use algo_sdk::plugin::{AlgoPlugin, InitContext};

use crate::config::InstanceConfig;
use crate::detect::decode_yolov8_face;
use crate::postprocess::{emit_face_detections, FaceDetection};
use crate::quality::compute_quality;
use crate::rknn::RknnInferenceOutput;
use crate::SharedModels;

pub struct FaceRecognizer {
    pub models: Arc<SharedModels>,
    pub cv_engine: RgaCvEngine,
    pub config: InstanceConfig,
}

impl std::fmt::Debug for FaceRecognizer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FaceRecognizer")
            .field("config", &self.config)
            .finish()
    }
}

impl AlgoPlugin for FaceRecognizer {
    type Config = InstanceConfig;

    fn init(ctx: &InitContext<'_>, config: Self::Config) -> Result<Self, AlgoError> {
        config
            .validate()
            .map_err(|reason| AlgoError::ConfigParse { reason })?;
        let models = crate::shared_models(ctx.package_root)?;
        let cv_engine = RgaCvEngine::new();
        Ok(Self {
            models,
            cv_engine,
            config,
        })
    }

    fn process(
        &mut self,
        frame: SafeFrame<'_>,
        emitter: &mut ResultEmitter<'_>,
    ) -> Result<(), AlgoError> {
        // 1. 调用 CV 引擎进行等比缩放与 Letterbox 填充至 640×384
        let (buf, mode) = self
            .cv_engine
            .letterbox(&frame, 640, 384, [114, 114, 114])?;

        let PreprocessMode::Letterbox(layout) = mode else {
            return Err(AlgoError::Preprocess {
                reason: "人脸检测需要 Letterbox 预处理模式".to_string(),
            });
        };

        let Some(host_bytes) = buf.as_host_bytes() else {
            return Err(AlgoError::Preprocess {
                reason: "无法获取 Letterbox 画布的主机内存字节切片".to_string(),
            });
        };

        let orig_w = frame.width();
        let _orig_h = frame.height();

        // 2. 检测推理
        let detector = self
            .models
            .detector
            .lock()
            .map_err(|_| AlgoError::Internal {
                reason: "RKNN 检测会话互斥锁中毒".to_string(),
            })?;

        let attrs: Vec<[u32; 4]> = detector
            .output_attrs
            .iter()
            .map(|a| [a.dims[0], a.dims[1], a.dims[2], a.dims[3]])
            .collect();

        let min_score = self.config.detection_confidence_threshold;

        let faces = detector.infer_with_host_bytes(host_bytes, |output| match output {
            RknnInferenceOutput::Float32(float_views) => {
                let decoded = decode_yolov8_face(float_views, &attrs, &layout, min_score, 0.45);
                Ok(decoded)
            }
        })?;

        // 释放 detector 锁
        drop(detector);

        // 3. 质量门控过滤
        let detections: Vec<FaceDetection> = faces
            .into_iter()
            .filter_map(|face| {
                let face_width_pixels = face.bbox[2] * orig_w as f32;
                let quality = compute_quality(
                    &face.landmarks,
                    &face.landmark_scores,
                    face_width_pixels,
                    &self.config.quality_thresholds,
                );
                if quality.accepted(&self.config.quality_thresholds, self.config.min_face_size) {
                    Some(FaceDetection {
                        bbox: face.bbox,
                        landmarks: face.landmarks,
                        detection_score: face.score,
                        quality,
                    })
                } else {
                    None
                }
            })
            .collect();

        // 4. 结果发射
        emit_face_detections(emitter, &detections)
    }

    fn update_config(&mut self, config: Self::Config) -> Result<(), AlgoError> {
        config
            .validate()
            .map_err(|reason| AlgoError::ConfigParse { reason })?;
        self.config = config;
        Ok(())
    }
}
