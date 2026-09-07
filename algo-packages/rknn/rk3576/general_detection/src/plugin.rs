//! 通用目标检测算法插件实现 (RK3576 RKNN GeneralDetector)

use std::path::Path;

use algo_sdk::cv::engine::CvEngine;
use algo_sdk::cv::platforms::rockchip::RgaCvEngine;
use algo_sdk::emitter::ResultEmitter;
use algo_sdk::error::AlgoError;
use algo_sdk::frame::SafeFrame;
use algo_sdk::plugin::{AlgoPlugin, InitContext};

use crate::config::{ClassMask, InstanceConfig};
use crate::postprocess::{parse_and_unmap_output, MODEL_INPUT_HEIGHT, MODEL_INPUT_WIDTH};
use crate::rknn::{RknnRuntime, RknnSession};

/// 寻找算法包内有效的 RKNN 模型文件路径
fn locate_model_file(package_root: &Path) -> Result<std::path::PathBuf, AlgoError> {
    // 1. 优先检测标准模型路径
    let default_model = package_root.join("model/yolov8n-640x384-rk3576.rknn");
    if default_model.is_file() {
        return Ok(default_model);
    }

    // 2. 遍历 model/ 目录查找第一个 .rknn 文件
    let model_dir = package_root.join("model");
    if let Ok(entries) = std::fs::read_dir(&model_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("rknn") {
                return Ok(path);
            }
        }
    }

    Err(AlgoError::Internal {
        reason: format!("在算法包模型目录中未找到 .rknn 模型: {:?}", model_dir),
    })
}

pub struct GeneralDetector {
    pub session: RknnSession,
    pub cv_engine: RgaCvEngine,
    pub config: InstanceConfig,
    pub mask: ClassMask,
    pub custom_label: Option<&'static str>,
}

impl std::fmt::Debug for GeneralDetector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GeneralDetector")
            .field("session", &self.session)
            .field("config", &self.config)
            .finish()
    }
}

impl AlgoPlugin for GeneralDetector {
    type Config = InstanceConfig;

    fn init(ctx: &InitContext<'_>, config: Self::Config) -> Result<Self, AlgoError> {
        let model_path = locate_model_file(ctx.package_root)?;
        let session = match RknnRuntime::load(ctx.package_root) {
            Ok(runtime) => {
                tracing::info!(
                    model = ?model_path,
                    "成功加载物理 RKNN 运行时 (librknnrt.so)，启用常驻硬件推理主路径"
                );
                RknnSession::new(runtime, &model_path)?
            }
            Err(e) => {
                tracing::warn!(
                    reason = ?e,
                    model = ?model_path,
                    "[debug_cpu_fallback_path] 未检测到物理 librknnrt.so，启用开发调试回退推理路径"
                );
                RknnSession::new_fallback(&model_path)?
            }
        };
        let cv_engine = RgaCvEngine::new();
        let mask = ClassMask::from_classes(&config.target_classes);

        // 仅在初始化阶段缓存一次性静态自定义标签，彻底杜绝每帧重复 leak
        let custom_label = config
            .custom_alarm_label
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| Box::leak(s.to_string().into_boxed_str()) as &'static str);

        tracing::info!(
            model = ?model_path,
            rga_hw = cv_engine.hardware_available(),
            fallback = session.is_fallback(),
            "成功初始化 RK3576 通用目标检测算法插件"
        );

        Ok(Self {
            session,
            cv_engine,
            config,
            mask,
            custom_label,
        })
    }

    fn process(
        &mut self,
        frame: SafeFrame<'_>,
        emitter: &mut ResultEmitter<'_>,
    ) -> Result<(), AlgoError> {
        // 1. 调用 RGA 硬件或 CPU 回退引擎进行等比缩放与 Letterbox 填充
        let (buf, mode) = self.cv_engine.letterbox(
            &frame,
            MODEL_INPUT_WIDTH as u32,
            MODEL_INPUT_HEIGHT as u32,
            [114, 114, 114],
        )?;

        let orig_w = frame.width();
        let orig_h = frame.height();

        // 2. 双模自适应分支：优先硬件 DMA-BUF 零拷贝直通，保底走 Host 内存复制
        if let Some(fd) = buf.as_dma_buf_fd() {
            let buffer_size = (MODEL_INPUT_WIDTH as usize) * (MODEL_INPUT_HEIGHT as usize) * 3; // RGB888 3 字节/像素

            let custom_label = self.custom_label;
            self.session
                .infer_with_dma_buf(fd, buffer_size, |net_out| {
                    let boxes = parse_and_unmap_output(
                        net_out,
                        &self.config,
                        &self.mask,
                        custom_label,
                        &mode,
                        orig_w,
                        orig_h,
                    );
                    emitter.emit_detections(&boxes)
                })?;
        } else if let Some(host_bytes) = buf.as_host_bytes() {
            let custom_label = self.custom_label;
            self.session.infer_with_host_bytes(host_bytes, |net_out| {
                let boxes = parse_and_unmap_output(
                    net_out,
                    &self.config,
                    &self.mask,
                    custom_label,
                    &mode,
                    orig_w,
                    orig_h,
                );
                emitter.emit_detections(&boxes)
            })?;
        } else {
            return Err(AlgoError::Preprocess {
                reason: "预处理输出的 CvBuffer 既无有效 DMA-BUF 句柄，又无 Host 内存视图"
                    .to_string(),
            });
        }

        Ok(())
    }

    fn flush(&mut self, _emitter: &mut ResultEmitter<'_>) -> Result<(), AlgoError> {
        Ok(())
    }

    fn update_config(&mut self, config: Self::Config) -> Result<(), AlgoError> {
        if self.config.custom_alarm_label != config.custom_alarm_label {
            self.custom_label = config
                .custom_alarm_label
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(|s| Box::leak(s.to_string().into_boxed_str()) as &'static str);
        }
        self.mask = ClassMask::from_classes(&config.target_classes);
        self.config = config;
        Ok(())
    }
}
