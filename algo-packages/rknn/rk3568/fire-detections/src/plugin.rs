//! 烟火检测算法插件实现 (RK3568 RKNN FireSmokeDetector)
//!
//! 基于 `algo-sdk` 规范构建，集成 YOLOv8 RKNN 推理、多帧时序确认与防误报滤波。

use algo_sdk::emitter::ResultEmitter;
use algo_sdk::error::AlgoError;
use algo_sdk::frame::SafeFrame;
use algo_sdk::plugin::{AlgoPlugin, InitContext};

use crate::config::InstanceConfig;
#[cfg(target_os = "linux")]
use crate::config::{ClassMask, FIRE_SMOKE_CLASSES};
#[cfg(target_os = "linux")]
use crate::temporal_verifier::TemporalVerifier;

/// 模型输入尺寸 (16:9) 与预计算倒数（乘法替代除法优化）
#[cfg(target_os = "linux")]
const MODEL_INPUT_WIDTH: f32 = 640.0;
#[cfg(target_os = "linux")]
const MODEL_INPUT_HEIGHT: f32 = 384.0;
#[cfg(target_os = "linux")]
const INV_MODEL_WIDTH: f32 = 1.0 / MODEL_INPUT_WIDTH;
#[cfg(target_os = "linux")]
const INV_MODEL_HEIGHT: f32 = 1.0 / MODEL_INPUT_HEIGHT;
#[cfg(target_os = "linux")]
const BUFFER_SIZE: usize = (MODEL_INPUT_WIDTH as usize) * (MODEL_INPUT_HEIGHT as usize) * 3;

#[cfg(target_os = "linux")]
use {
    crate::rknn::{RknnInferenceOutput, RknnRuntime, RknnSession},
    algo_sdk::cv::engine::CvEngine,
    algo_sdk::cv::platforms::rockchip::RgaCvEngine,
    algo_sdk::cv::postprocess::{parse_yolov8_int8, Yolov8ParseContext, Yolov8RknnConfig},
    std::path::Path,
};

/// 泄漏自定义业务标签为静态生命周期字符串
#[cfg(target_os = "linux")]
fn leak_label(label: Option<&str>) -> Option<&'static str> {
    label
        .filter(|s| !s.is_empty())
        .map(|s| Box::leak(s.to_string().into_boxed_str()) as &'static str)
}

/// 寻找算法包内有效的 RKNN 模型文件路径（优先使用当前包私有 .env 配置，零全局污染）
#[cfg(target_os = "linux")]
fn locate_model_file(
    package_root: &Path,
    env: &algo_sdk::env::PackageEnv,
) -> Result<std::path::PathBuf, AlgoError> {
    // 1. 优先使用当前包私有 .env 配置（不污染系统全局环境）
    if env.get("MODEL_PATH").is_some() {
        return env.resolve_model_path(package_root, "MODEL_PATH", "model/best_pure.rknn");
    }

    // 2. 依次检查默认模型工件（优先 pure，其次 hybrid）
    for model_rel in ["model/best_pure.rknn", "model/best_hybrid.rknn"] {
        let candidate = package_root.join(model_rel);
        if candidate.is_file() {
            return candidate.canonicalize().map_err(|e| AlgoError::ModelLoad {
                reason: format!("规范化模型路径失败: {e}"),
            });
        }
    }

    env.resolve_model_path(package_root, "MODEL_PATH", "model/best_pure.rknn")
}

#[cfg(target_os = "linux")]
pub struct FireSmokeDetector {
    pub session: RknnSession,
    pub cv_engine: RgaCvEngine,
    pub config: InstanceConfig,
    pub mask: ClassMask,
    pub custom_label: Option<&'static str>,
    /// 多帧时序确认验证器（滑动窗口 + 命中阈值）
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

    fn init(ctx: &InitContext<'_>, mut config: Self::Config) -> Result<Self, AlgoError> {
        let env = ctx.load_env();
        config.apply_env(&env);
        let model_path = locate_model_file(ctx.package_root, &env)?;
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
        let temporal_verifier = TemporalVerifier::new(
            config.confirm_window,
            config.confirm_threshold,
            config.temporal_variance_threshold,
        );

        let custom_label = leak_label(config.custom_alarm_label.as_deref());

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
        // 1. RGA 硬件 Letterbox 等比缩放与填充 (640x384)
        let (buf, mode) = self.cv_engine.letterbox(
            &frame,
            MODEL_INPUT_WIDTH as u32,
            MODEL_INPUT_HEIGHT as u32,
            [0, 0, 0],
        )?;

        let orig_w = frame.width();
        let orig_h = frame.height();

        // 2. 双模自适应：优先 DMA-BUF 零拷贝直通，保底 Host 内存复制
        let custom_label = self.custom_label;
        let mask = self.mask;
        let config = &self.config;

        let parse = |net_out: &RknnInferenceOutput<'_>| {
            Ok(parse_and_filter(
                net_out,
                config,
                &mask,
                custom_label,
                &mode,
                orig_w,
                orig_h,
            ))
        };

        let mut boxes = if let Some(fd) = buf.as_dma_buf_fd() {
            self.session.infer_with_dma_buf(fd, BUFFER_SIZE, parse)?
        } else if let Some(host_bytes) = buf.as_host_bytes() {
            self.session.infer_with_host_bytes(host_bytes, parse)?
        } else {
            return Err(AlgoError::Preprocess {
                reason: "预处理输出的 CvBuffer 既无有效 DMA-BUF 句柄，又无 Host 内存视图"
                    .to_string(),
            });
        };

        // 3. 多帧时序确认过滤（零堆分配流式提取，消除偶发噪点与瞬态反光）
        let confirmed_mask = self
            .temporal_verifier
            .verify_frame_mask(boxes.iter().map(|b| (b.class_id as usize, b.confidence)));

        // 4. 原地保留确认类别并立即发射（Zero-alloc retain）
        if confirmed_mask != 0 {
            boxes.retain(|b| (confirmed_mask & (1 << b.class_id)) != 0);
            if !boxes.is_empty() {
                let _ = emitter.emit_detections(&boxes);
            }
        }

        Ok(())
    }

    fn flush(&mut self, _emitter: &mut ResultEmitter<'_>) -> Result<(), AlgoError> {
        Ok(())
    }

    fn update_config(&mut self, config: InstanceConfig) -> Result<(), AlgoError> {
        if self.config.custom_alarm_label != config.custom_alarm_label {
            self.custom_label = leak_label(config.custom_alarm_label.as_deref());
        }
        self.mask = ClassMask::from_classes(&config.target_classes);

        // 时序验证参数变更时重建验证器
        if self.config.confirm_window != config.confirm_window
            || self.config.confirm_threshold != config.confirm_threshold
            || (self.config.temporal_variance_threshold - config.temporal_variance_threshold).abs()
                > f32::EPSILON
        {
            self.temporal_verifier = TemporalVerifier::new(
                config.confirm_window,
                config.confirm_threshold,
                config.temporal_variance_threshold,
            );
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
            let mut boxes = parse_yolov8_int8(&parse_ctx);

            // 类别掩码过滤 + 自定义标签覆盖（就地修改，零二次分配）
            if let Some(label) = custom_label {
                boxes.iter_mut().for_each(|b| b.label = Some(label));
            }
            boxes.retain(|b| mask.is_enabled(b.class_id as usize));
            boxes
        }
        RknnInferenceOutput::SingleFloat(slice) => {
            parse_single_float_fallback(slice, config, mask, custom_label, mode, orig_w, orig_h)
        }
    }
}

/// 单浮点输出的 fallback 解析（debug_cpu_fallback_path 使用，乘法替代除法优化）
#[cfg(target_os = "linux")]
fn parse_single_float_fallback(
    net_out: &[f32],
    config: &InstanceConfig,
    mask: &ClassMask,
    custom_label: Option<&'static str>,
    mode: &algo_sdk::cv::types::PreprocessMode,
    orig_w: u32,
    orig_h: u32,
) -> Vec<algo_sdk::math::NormBox> {
    use algo_sdk::math::{fast_nms, unmap_box, NormBox};

    const TOTAL_ANCHORS: usize = 5040;
    const NUM_CHANNELS: usize = 84;

    if net_out.len() < NUM_CHANNELS * TOTAL_ANCHORS || orig_w == 0 || orig_h == 0 {
        return Vec::new();
    }

    let mut candidates = Vec::with_capacity(32);
    let conf_thresh = config.confidence_threshold;

    for i in 0..TOTAL_ANCHORS {
        let score_0 = net_out[4 * TOTAL_ANCHORS + i];
        let score_1 = net_out[5 * TOTAL_ANCHORS + i];

        let (best_class, max_score) = if score_0 >= score_1 {
            (0usize, score_0)
        } else {
            (1usize, score_1)
        };

        if !max_score.is_finite() || max_score < conf_thresh || !mask.is_enabled(best_class) {
            continue;
        }

        let cx = net_out[i];
        let cy = net_out[TOTAL_ANCHORS + i];
        let w = net_out[2 * TOTAL_ANCHORS + i];
        let h = net_out[3 * TOTAL_ANCHORS + i];

        let raw = NormBox {
            x: ((cx - w * 0.5) * INV_MODEL_WIDTH).clamp(0.0, 1.0),
            y: ((cy - h * 0.5) * INV_MODEL_HEIGHT).clamp(0.0, 1.0),
            w: (w * INV_MODEL_WIDTH).clamp(0.0, 1.0),
            h: (h * INV_MODEL_HEIGHT).clamp(0.0, 1.0),
            confidence: max_score,
            class_id: best_class as u32,
            label: custom_label.or(FIRE_SMOKE_CLASSES.get(best_class).copied()),
        };
        candidates.push(unmap_box(&raw, mode, orig_w, orig_h));
    }

    if !candidates.is_empty() {
        fast_nms(&mut candidates, config.iou_threshold);
    }
    candidates
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
