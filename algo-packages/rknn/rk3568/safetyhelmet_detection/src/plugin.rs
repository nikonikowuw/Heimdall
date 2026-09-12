//! 安全帽检测算法插件实现 (RK3568 RKNN SafetyHelmetDetector)
//!
//! 基于 `algo-sdk` 规范构建，提供 C ABI 导出、Rockchip RGA 硬件零拷贝预处理与 RK3568 NPU 推理接入。

use algo_sdk::emitter::ResultEmitter;
use algo_sdk::error::AlgoError;
use algo_sdk::frame::SafeFrame;
use algo_sdk::plugin::{AlgoPlugin, InitContext};

use crate::config::InstanceConfig;

#[cfg(target_os = "linux")]
use {
    crate::postprocess::{parse_and_unmap_output, MODEL_INPUT_HEIGHT, MODEL_INPUT_WIDTH},
    crate::rknn::{RknnRuntime, RknnSession},
    algo_sdk::cv::engine::CvEngine,
    algo_sdk::cv::platforms::rockchip::{DiagnosticConfig, FailureTracker, RgaCvEngine},
    std::path::{Path, PathBuf},
};

/// 算法包内有效的 RKNN 模型文件查找
///
/// 优先级：
/// 1. 环境变量 MODEL_PATH
/// 2. 默认路径 model/best_hybrid.rknn
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

    // 2. 使用默认路径
    let model_path = package_root.join("model/best_hybrid.rknn");
    if model_path.is_file() {
        return Ok(model_path);
    }

    Err(AlgoError::Internal {
        reason: format!("未找到模型文件: {:?}", model_path),
    })
}

#[cfg(target_os = "linux")]
pub struct SafetyHelmetDetector {
    pub session: RknnSession,
    pub cv_engine: RgaCvEngine,
    pub config: InstanceConfig,
    pub custom_label: Option<&'static str>,
    pub failure_tracker: FailureTracker,
}

#[cfg(target_os = "linux")]
impl std::fmt::Debug for SafetyHelmetDetector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SafetyHelmetDetector")
            .field("session", &self.session)
            .field("config", &self.config)
            .finish()
    }
}

#[cfg(target_os = "linux")]
impl AlgoPlugin for SafetyHelmetDetector {
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

        // 仅在初始化阶段缓存一次性静态自定义标签，杜绝每帧重复分配
        let custom_label = config
            .custom_alarm_label
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| Box::leak(s.to_string().into_boxed_str()) as &'static str);

        // 初始化失败跟踪器：连续 30 帧失败视为异常状态
        let failure_tracker = FailureTracker::new(DiagnosticConfig {
            failure_threshold: 30,
        });

        tracing::info!(
            model = ?model_path,
            rga_hw = cv_engine.hardware_available(),
            fallback = session.is_fallback(),
            failure_threshold = failure_tracker.failure_threshold(),
            "成功初始化 RK3568 安全帽检测算法插件"
        );

        Ok(Self {
            session,
            cv_engine,
            config,
            custom_label,
            failure_tracker,
        })
    }

    fn process(
        &mut self,
        frame: SafeFrame<'_>,
        emitter: &mut ResultEmitter<'_>,
    ) -> Result<(), AlgoError> {
        // 1. RGA 硬件 Letterbox 等比缩放与填充
        let result = self.cv_engine.letterbox(
            &frame,
            MODEL_INPUT_WIDTH as u32,
            MODEL_INPUT_HEIGHT as u32,
            [114, 114, 114],
        );

        match result {
            Ok((buf, mode)) => {
                // 2. 处理成功，重置失败计数器
                self.failure_tracker.record_success();

                let orig_w = frame.width();
                let orig_h = frame.height();

                // 3. 双模自适应：优先 DMA-BUF 零拷贝，保底 Host 内存复制
                if let Some(fd) = buf.as_dma_buf_fd() {
                    let buffer_size =
                        (MODEL_INPUT_WIDTH as usize) * (MODEL_INPUT_HEIGHT as usize) * 3;

                    let custom_label = self.custom_label;
                    self.session
                        .infer_with_dma_buf(fd, buffer_size, |net_out| {
                            let boxes = parse_and_unmap_output(
                                net_out,
                                &self.config,
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
            Err(error) => {
                // 4. 处理失败，记录失败计数（告警由 tracing::error! 在 engine.rs 中记录）
                self.failure_tracker.record_failure();
                Err(error)
            }
        }
    }

    fn flush(&mut self, _emitter: &mut ResultEmitter<'_>) -> Result<(), AlgoError> {
        self.failure_tracker.reset();
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
        self.config = config;
        Ok(())
    }
}

#[cfg(not(target_os = "linux"))]
#[derive(Debug)]
pub struct SafetyHelmetDetector;

#[cfg(not(target_os = "linux"))]
impl AlgoPlugin for SafetyHelmetDetector {
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
