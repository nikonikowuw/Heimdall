//! 人脸检测插件的标准 `AlgoPlugin` 适配层。

use algo_sdk::emitter::ResultEmitter;
use algo_sdk::error::AlgoError;
use algo_sdk::frame::SafeFrame;
use algo_sdk::plugin::{AlgoPlugin, InitContext};

use crate::config::InstanceConfig;
use crate::detect::{decode_yolov5_face, nms, unmap_letterbox};
use crate::postprocess::FaceDetection;
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
        Ok(Self { models, config })
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
        // SAFETY: pixelbuffer 由当前 CvBuffer 持有，直到预测返回前不会释放。
        let raw_output = unsafe { self.models.predict_detector(pixelbuffer)? };
        let mut raw_faces =
            decode_yolov5_face(&raw_output, self.config.detection_confidence_threshold);
        nms(&mut raw_faces, 0.45);
        unmap_letterbox(&mut raw_faces, &mode, frame.width(), frame.height());

        let mut faces = Vec::with_capacity(raw_faces.len());
        for face in raw_faces {
            let quality = compute_quality(
                &face.landmarks,
                &face.landmark_scores,
                face.bbox[2] * frame.width() as f32,
                &self.config.quality_thresholds,
            );
            if !quality.accepted(&self.config.quality_thresholds, self.config.min_face_size) {
                continue;
            }
            faces.push(FaceDetection {
                bbox: face.bbox,
                landmarks: face.landmarks,
                detection_score: face.score,
                quality,
            });
        }
        crate::postprocess::emit_face_detections(emitter, &faces)
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
}
