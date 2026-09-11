//! 人脸检测插件的标准 `AlgoPlugin` 适配层

use std::sync::Arc;

use algo_sdk::cv::{self, PreprocessMode};
use algo_sdk::emitter::ResultEmitter;
use algo_sdk::error::AlgoError;
use algo_sdk::frame::SafeFrame;
use algo_sdk::plugin::{AlgoPlugin, InitContext};

use crate::config::InstanceConfig;
use crate::postprocess::{emit_face_detections, FaceDetection};
use crate::quality::compute_quality;
use crate::SharedModels;

pub struct FaceRecognizer {
    pub models: Arc<SharedModels>,
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
        Ok(Self { models, config })
    }

    fn process(
        &mut self,
        frame: SafeFrame<'_>,
        emitter: &mut ResultEmitter<'_>,
    ) -> Result<(), AlgoError> {
        // 宏在当前线程作用域注入宿主的 CvEngine；不要绕过 image_ops 重新创建平台引擎。
        let (buf, mode) = cv::letterbox(
            &frame,
            self.models.detector_width,
            self.models.detector_height,
            [114, 114, 114],
        )?;

        let PreprocessMode::Letterbox(layout) = mode else {
            return Err(AlgoError::Preprocess {
                reason: "人脸检测需要 Letterbox 预处理模式".to_string(),
            });
        };

        let min_score = self.config.detection_confidence_threshold;
        let faces = if buf.as_dma_buf_layout().is_some() {
            self.models.worker.detect_dma_buf(buf, layout, min_score)?
        } else if let Some(host_bytes) = buf.as_host_bytes() {
            self.models
                .worker
                .detect_host(host_bytes.to_vec(), layout, min_score)?
        } else {
            return Err(AlgoError::Preprocess {
                reason: "预处理输出既无有效 DMA-BUF 布局，也无 Host 内存视图".to_string(),
            });
        };

        let orig_w = frame.width();
        let orig_h = frame.height();

        // 质量门控过滤
        let detections: Vec<FaceDetection> = faces
            .into_iter()
            .filter_map(|face| {
                let face_width_pixels = face.width() * orig_w as f32;
                let face_height_pixels = face.height() * orig_h as f32;
                let quality = compute_quality(
                    &face.landmarks,
                    &face.landmark_scores,
                    face_width_pixels.min(face_height_pixels),
                    &self.config.quality_thresholds,
                );
                quality
                    .accepted(&self.config.quality_thresholds, self.config.min_face_size)
                    .then_some(FaceDetection {
                        bbox: face.bbox,
                        landmarks: face.landmarks,
                        detection_score: face.score,
                        quality,
                    })
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
