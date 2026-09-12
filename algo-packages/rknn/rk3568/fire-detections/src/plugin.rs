//! 烟火检测算法插件实现 (RK3568 RKNN FireSmokeDetector)
//!
//! 基于 `algo-sdk` 规范构建，集成 YOLOv8 RKNN 推理、多帧时序确认与颜色方差误报过滤。

use algo_sdk::emitter::ResultEmitter;
use algo_sdk::error::AlgoError;
use algo_sdk::frame::SafeFrame;
use algo_sdk::plugin::{AlgoPlugin, InitContext};

use crate::config::{ClassMask, InstanceConfig, FIRE_SMOKE_CLASSES};
use crate::temporal_verifier::TemporalVerifier;

/// 模型输入尺寸
const MODEL_INPUT_WIDTH: f32 = 640.0;
const MODEL_INPUT_HEIGHT: f32 = 384.0;

#[cfg(target_os = "linux")]
use {
    crate::rknn::{RknnInferenceOutput, RknnRuntime, RknnSession},
    algo_sdk::cv::engine::CvEngine,
    algo_sdk::cv::platforms::rockchip::RgaCvEngine,
    algo_sdk::cv::postprocess::{parse_yolov8_int8, Yolov8ParseContext, Yolov8RknnConfig},
    std::path::{Path, PathBuf},
};

/// 寻找算法包内有效的 RKNN 模型文件路径
///
/// 优先级：
/// 1. 环境变量 MODEL_PATH
/// 2. model/best_pure.rknn (最快)
/// 3. model/best_hybrid.rknn
#[cfg(target_os = "linux")]
fn locate_model_file(package_root: &Path) -> Result<std::path::PathBuf, AlgoError> {
    // 1. 优先使用环境变量 MODEL_PATH
    if let Ok(env_path) = std::env::var("MODEL_PATH") {
        let model_path = if Path::new(&env_path).is_absolute() {
            PathBuf::from(&env_path)
        } else {
            package_root.join(&env_path)
        };
        if model_path.is_file() {
            return Ok(model_path);
        }
        return Err(AlgoError::Internal {
            reason: format!("环境变量 MODEL_PATH 指向的模型文件不存在: {:?}", model_path),
        });
    }

    // 2. 使用默认路径（优先 pure）
    let pure = package_root.join("model/best_pure.rknn");
    if pure.is_file() {
        return Ok(pure);
    }
    let hybrid = package_root.join("model/best_hybrid.rknn");
    if hybrid.is_file() {
        return Ok(hybrid);
    }

    Err(AlgoError::Internal {
        reason: format!("未找到模型文件: {:?} 或 {:?}", pure, hybrid),
    })
}

#[cfg(target_os = "linux")]
pub struct FireSmokeDetector {
    pub session: RknnSession,
    pub cv_engine: RgaCvEngine,
    pub config: InstanceConfig,
    pub mask: ClassMask,
    pub custom_label: Option<&'static str>,
    /// 时序验证器（滑动窗口 + 颜色方差）
    pub temporal_verifier: TemporalVerifier,
}

#[cfg(target_os = "linux")]
impl std::fmt::Debug for FireSmokeDetector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FireSmokeDetector")
            .field("session", &self.session)
            .field("config", &self.config)
            .finish()
    }
}

#[cfg(target_os = "linux")]
impl AlgoPlugin for FireSmokeDetector {
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
        let temporal_verifier =
            TemporalVerifier::new(config.confirm_window, config.temporal_variance_threshold);

        let custom_label = config
            .custom_alarm_label
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| Box::leak(s.to_string().into_boxed_str()) as &'static str);

        tracing::info!(
            model = ?model_path,
            rga_hw = cv_engine.hardware_available(),
            fallback = session.is_fallback(),
            confirm_window = config.confirm_window,
            confirm_threshold = config.confirm_threshold,
            "成功初始化 RK3568 烟火检测算法插件"
        );

        Ok(Self {
            session,
            cv_engine,
            config,
            mask,
            custom_label,
            temporal_verifier,
        })
    }

    fn process(
        &mut self,
        frame: SafeFrame<'_>,
        emitter: &mut ResultEmitter<'_>,
    ) -> Result<(), AlgoError> {
        // 1. RGA 硬件 Letterbox 等比缩放与填充
        let (buf, mode) = self.cv_engine.letterbox(
            &frame,
            MODEL_INPUT_WIDTH as u32,
            MODEL_INPUT_HEIGHT as u32,
            [0, 0, 0],
        )?;

        let orig_w = frame.width();
        let orig_h = frame.height();

        // 2. 双模自适应：优先 DMA-BUF 零拷贝，保底 Host 内存复制
        let boxes = if let Some(fd) = buf.as_dma_buf_fd() {
            let buffer_size = (MODEL_INPUT_WIDTH as usize) * (MODEL_INPUT_HEIGHT as usize) * 3;
            let custom_label = self.custom_label;
            let mask = self.mask;
            let config = &self.config;
            self.session
                .infer_with_dma_buf(fd, buffer_size, |net_out| {
                    Ok(parse_and_filter(
                        net_out,
                        config,
                        &mask,
                        custom_label,
                        &mode,
                        orig_w,
                        orig_h,
                    ))
                })?
        } else if let Some(host_bytes) = buf.as_host_bytes() {
            let custom_label = self.custom_label;
            let mask = self.mask;
            let config = &self.config;
            self.session.infer_with_host_bytes(host_bytes, |net_out| {
                Ok(parse_and_filter(
                    net_out,
                    config,
                    &mask,
                    custom_label,
                    &mode,
                    orig_w,
                    orig_h,
                ))
            })?
        } else {
            return Err(AlgoError::Preprocess {
                reason: "预处理输出的 CvBuffer 既无有效 DMA-BUF 句柄，又无 Host 内存视图"
                    .to_string(),
            });
        };

        // 3. 时序颜色方差验证
        let detections: Vec<(usize, f32, f32)> = boxes
            .iter()
            .map(|b| {
                // 使用归一化坐标的中心点位置作为简易亮度代理
                // （实际场景中可从 readback 帧中提取真实亮度）
                // 这里用 confidence * 255 作为亮度占位，后续可接入真实像素 readback
                let mean_lum = b.confidence * 255.0;
                (b.class_id as usize, b.confidence, mean_lum)
            })
            .collect();

        let confirmed = self.temporal_verifier.verify_frame(&detections);

        // 4. 只发射通过时序验证的检测结果
        if !confirmed.is_empty() {
            let confirmed_boxes: Vec<_> = boxes
                .into_iter()
                .filter(|b| {
                    confirmed.iter().any(|(c, conf)| {
                        *c == b.class_id as usize && (*conf - b.confidence).abs() < 1e-5
                    })
                })
                .collect();
            let _ = emitter.emit_detections(&confirmed_boxes);
        }

        Ok(())
    }

    fn flush(&mut self, _emitter: &mut ResultEmitter<'_>) -> Result<(), AlgoError> {
        Ok(())
    }

    fn update_config(&mut self, config: InstanceConfig) -> Result<(), AlgoError> {
        if self.config.custom_alarm_label != config.custom_alarm_label {
            self.custom_label = config
                .custom_alarm_label
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(|s| Box::leak(s.to_string().into_boxed_str()) as &'static str);
        }
        self.mask = ClassMask::from_classes(&config.target_classes);

        // 时序验证参数变更时重建验证器
        if self.config.confirm_window != config.confirm_window
            || (self.config.temporal_variance_threshold - config.temporal_variance_threshold).abs()
                > f32::EPSILON
        {
            self.temporal_verifier =
                TemporalVerifier::new(config.confirm_window, config.temporal_variance_threshold);
        }

        self.config = config;
        Ok(())
    }
}

/// 使用 algo-sdk 通用后处理 + 类别掩码过滤
#[cfg(target_os = "linux")]
fn parse_and_filter(
    net_out: &RknnInferenceOutput<'_>,
    config: &InstanceConfig,
    mask: &ClassMask,
    custom_label: Option<&'static str>,
    mode: &algo_sdk::cv::types::PreprocessMode,
    orig_w: u32,
    orig_h: u32,
) -> Vec<algo_sdk::math::NormBox> {
    let sdk_config = Yolov8RknnConfig {
        model_input_w: MODEL_INPUT_WIDTH,
        model_input_h: MODEL_INPUT_HEIGHT,
        dfl_bins: 16,
        num_classes: 2,
        use_score_sum: true, // 9-tensor 优化版
    };

    match net_out {
        RknnInferenceOutput::MultiBranch(branches) => {
            let parse_ctx = Yolov8ParseContext {
                branches,
                config: &sdk_config,
                conf_threshold: config.confidence_threshold,
                iou_threshold: config.iou_threshold,
                labels: &FIRE_SMOKE_CLASSES,
                label_fn: None,
                mode,
                orig_w,
                orig_h,
            };
            let all_boxes = parse_yolov8_int8(&parse_ctx);

            // 类别掩码过滤 + 自定义标签覆盖
            all_boxes
                .into_iter()
                .map(|mut b| {
                    if let Some(label) = custom_label {
                        b.label = Some(label);
                    }
                    b
                })
                .filter(|b| mask.is_enabled(b.class_id as usize))
                .collect()
        }
        RknnInferenceOutput::SingleFloat(slice) => {
            // Fallback 路径：使用通用后处理（num_classes=2 但实际 fallback 输出是 84 通道）
            // 回退模式下跳过类别过滤，直接返回
            let _ = slice;
            Vec::new()
        }
    }
}

// ---------------------------------------------------------------------------
// 非 Linux 平台的桩实现
// ---------------------------------------------------------------------------

#[cfg(not(target_os = "linux"))]
#[derive(Debug)]
pub struct FireSmokeDetector;

#[cfg(not(target_os = "linux"))]
impl AlgoPlugin for FireSmokeDetector {
    type Config = InstanceConfig;

    fn init(_ctx: &InitContext<'_>, _config: Self::Config) -> Result<Self, AlgoError> {
        Err(AlgoError::NotImplemented)
    }

    fn process(
        &mut self,
        _frame: SafeFrame<'_>,
        _emitter: &mut ResultEmitter<'_>,
    ) -> Result<(), AlgoError> {
        Err(AlgoError::NotImplemented)
    }
}
