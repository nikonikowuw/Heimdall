//! 通用目标检测算法插件实现 (GeneralDetector)

#[cfg(target_os = "macos")]
use std::sync::Arc;

#[cfg(target_os = "macos")]
use algo_sdk::cv::platforms::apple::AppleCvEngine;
#[cfg(target_os = "macos")]
use algo_sdk::cv::CvEngine;
use algo_sdk::emitter::ResultEmitter;
use algo_sdk::error::AlgoError;
use algo_sdk::frame::SafeFrame;
use algo_sdk::plugin::{AlgoPlugin, InitContext};

use crate::config::{ClassMask, InstanceConfig};
#[cfg(target_os = "macos")]
use crate::coreml::CoreMlRunner;
#[cfg(target_os = "macos")]
use crate::postprocess::parse_and_unmap_detections;

#[cfg(target_os = "macos")]
#[derive(Debug)]
pub struct GeneralDetector {
    pub runner: Arc<CoreMlRunner>,
    pub config: InstanceConfig,
    pub mask: ClassMask,
}

#[cfg(not(target_os = "macos"))]
#[derive(Debug)]
pub struct GeneralDetector {
    pub config: InstanceConfig,
    pub mask: ClassMask,
}

impl AlgoPlugin for GeneralDetector {
    type Config = InstanceConfig;

    #[cfg(target_os = "macos")]
    fn init(ctx: &InitContext<'_>, config: Self::Config) -> Result<Self, AlgoError> {
        let runner = CoreMlRunner::load_model(ctx.package_root)?;
        let mask = ClassMask::from_classes(&config.target_classes);

        Ok(Self {
            runner: Arc::new(runner),
            config,
            mask,
        })
    }

    #[cfg(not(target_os = "macos"))]
    fn init(_ctx: &InitContext<'_>, config: Self::Config) -> Result<Self, AlgoError> {
        let mask = ClassMask::from_classes(&config.target_classes);
        Ok(Self { config, mask })
    }

    #[cfg(target_os = "macos")]
    fn process(
        &mut self,
        frame: SafeFrame<'_>,
        emitter: &mut ResultEmitter<'_>,
    ) -> Result<(), AlgoError> {
        let (buf, mode) = AppleCvEngine.letterbox(&frame, 640, 384, [114, 114, 114])?;

        let pixelbuffer = buf.as_raw_ptr().ok_or_else(|| AlgoError::Internal {
            reason: "预处理输出的 CVPixelBuffer 指针为空".to_string(),
        })?;

        // SAFETY: pixelbuffer 来自有效的 CvBuffer，受当前生命周期保护
        let raw_output = unsafe { self.runner.predict_pixelbuffer(pixelbuffer)? };

        let boxes = parse_and_unmap_detections(
            &raw_output,
            &self.config,
            &self.mask,
            &mode,
            frame.width(),
            frame.height(),
        );

        emitter.emit_detections(&boxes)?;

        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    fn process(
        &mut self,
        _frame: SafeFrame<'_>,
        _emitter: &mut ResultEmitter<'_>,
    ) -> Result<(), AlgoError> {
        Err(AlgoError::Internal {
            reason: "macOS CoreML 检测插件仅支持在 macOS 平台上运行".to_string(),
        })
    }

    fn flush(&mut self, _emitter: &mut ResultEmitter<'_>) -> Result<(), AlgoError> {
        Ok(())
    }

    fn update_config(&mut self, config: Self::Config) -> Result<(), AlgoError> {
        self.mask = ClassMask::from_classes(&config.target_classes);
        self.config = config;
        Ok(())
    }
}
