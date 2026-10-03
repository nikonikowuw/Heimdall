//! 通用 YOLO 系列检测器模板与解码器策略 (`GenericYoloDetector` & `YoloDecoder`)
//!
//! 提供类似 PyTorch / TorchVision 的标准化模型抽象：
//! 1. `YoloSpec`: 声明模型规格（分辨率、类别数、标签、权重相对路径）；
//! 2. `YoloDecoder`: 解码策略契约，支持官方标准模型或任意魔改定制头；
//! 3. `GenericDetector`: 自动闭环“配置注入 -> 硬件预处理 -> NPU 异构推理 -> 解码 -> 结果发射”；
//! 4. `GenericYoloDetector<Spec>`: 默认绑定标准 YOLOv8/v11 解码器与平台会话，实现 ~15 行代码交付算法包。

use std::marker::PhantomData;

use crate::algo_config;
use crate::cv::diagnostic::{DiagnosticConfig, FailureTracker};
use crate::cv::postprocess::yolov8_rknn::{
    parse_yolov8_int8, Yolov8ParseContext, Yolov8RknnConfig,
};
use crate::cv::transforms::{HwLetterbox, Transform};
use crate::cv::types::PreprocessMode;
use crate::emitter::ResultEmitter;
use crate::error::AlgoError;
use crate::frame::SafeFrame;
use crate::math::NormBox;
use crate::plugin::{AlgoPlugin, InitContext};
use crate::runtime::{InferenceOutput, NpuSession, RuntimeSession};

/// YOLO 架构规格定义元信息
pub trait YoloSpec: Send + Sync + 'static {
    /// 模型输入分辨率 (宽, 高)，例如 `(640, 640)`
    const INPUT_DIM: (u32, u32);
    /// 类别数量
    const NUM_CLASSES: usize;
    /// 类别标签字符切片
    const LABELS: &'static [&'static str];
    /// 模型权重文件相对路径（相对于 package_root），例如 `"model/model.rknn"`
    const MODEL_PATH: &'static str;
    /// 是否使用 9-tensor score_sum 预筛选优化（YOLOv8 INT8 格式，默认为 true）
    const USE_SCORE_SUM: bool = true;
    /// DFL bins 数量（YOLOv8 默认为 16）
    const DFL_BINS: usize = 16;
    /// 分类分支是否输出未激活的 logits
    ///
    /// 导出 ONNX 时若将 sigmoid 移出计算图，模型的 score 分支将含负值，
    /// 需置为 `true` 让解码器还原置信度并换算阈值。默认 `false`（图中已激活）。
    const CLS_IS_LOGITS: bool = false;
}

algo_config! {
    /// 标准检测配置（自动包含 `explicit_fields` 与宿主 > `.env` > 代码默认值三级优先级隔离）
    #[derive(Debug, Clone, PartialEq)]
    pub struct StandardYoloConfig {
        /// 置信度阈值
        pub confidence_threshold: f32 = 0.45,
        /// NMS 重合度抑制阈值
        pub iou_threshold: f32 = 0.45,
        /// 自定义业务告警标签覆盖（可选）
        pub custom_alarm_label: Option<String> = None,
    }
}

/// 解码上下文环境参数封装
#[derive(Debug, Clone)]
pub struct YoloDecodeContext<'a> {
    /// 异构推理引擎输出统一抽象
    pub output: &'a InferenceOutput<'a>,
    /// 置信度阈值
    pub conf_threshold: f32,
    /// IOU 抑制阈值
    pub iou_threshold: f32,
    /// 预处理Letterbox几何变换模式
    pub mode: &'a PreprocessMode,
    /// 原图宽度
    pub orig_w: u32,
    /// 原图高度
    pub orig_h: u32,
}

/// 输出张量解码策略 Trait
pub trait YoloDecoder: Send + Sync + 'static {
    /// 将 NPU 原始输出解码为归一化坐标候选框
    fn decode(&self, ctx: &YoloDecodeContext<'_>) -> Result<Vec<NormBox>, AlgoError>;
}

/// 自动绑定 `YoloSpec` 元信息的默认 YOLOv8/v11 解码器
pub struct Yolov8SpecDecoder<S: YoloSpec>(PhantomData<S>);

impl<S: YoloSpec> std::fmt::Debug for Yolov8SpecDecoder<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Yolov8SpecDecoder").finish()
    }
}

impl<S: YoloSpec> Default for Yolov8SpecDecoder<S> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<S: YoloSpec> YoloDecoder for Yolov8SpecDecoder<S> {
    fn decode(&self, ctx: &YoloDecodeContext<'_>) -> Result<Vec<NormBox>, AlgoError> {
        let parse_cfg = Yolov8RknnConfig::from_spec(
            S::INPUT_DIM.0 as f32,
            S::INPUT_DIM.1 as f32,
            S::DFL_BINS,
            S::NUM_CLASSES,
            S::USE_SCORE_SUM,
            S::CLS_IS_LOGITS,
        )?;

        match ctx.output {
            InferenceOutput::MultiBranch(branches) => {
                let boxes = parse_yolov8_int8(&Yolov8ParseContext {
                    branches,
                    config: &parse_cfg,
                    conf_threshold: ctx.conf_threshold,
                    iou_threshold: ctx.iou_threshold,
                    labels: S::LABELS,
                    label_fn: None,
                    mode: ctx.mode,
                    orig_w: ctx.orig_w,
                    orig_h: ctx.orig_h,
                });

                Ok(boxes)
            }
            InferenceOutput::SingleFloat(data) => Ok(decode_single_float::<S>(data, ctx)),
        }
    }
}

/// 解析单张量平铺 FP32 YOLO 输出 `[1, 4 + classes, anchors]`（内部私有解码实现）
///
/// **激活语义**：该分支承载的是模拟推理桩（`debug_cpu_fallback_path`，见
/// `RknnSession::fallback_outputs`）的合成输出，其分类通道直接是概率，因此不应用
/// `S::CLS_IS_LOGITS`。该标志只描述真实 RKNN 量化图（`MultiBranch`）的分类分支语义；
/// 回退桩在物理上不是模型前向结果，无法也无需忠实复现 logits。
fn decode_single_float<S: YoloSpec>(data: &[f32], ctx: &YoloDecodeContext<'_>) -> Vec<NormBox> {
    let model_w = S::INPUT_DIM.0 as f32;
    let model_h = S::INPUT_DIM.1 as f32;
    if ctx.orig_w == 0
        || ctx.orig_h == 0
        || model_w <= 0.0
        || model_h <= 0.0
        || S::NUM_CLASSES == 0
        || data.is_empty()
    {
        return Vec::new();
    }

    let min_channels = 4 + S::NUM_CLASSES;
    let (channels, anchors) = if data.len() == 84 * 5040 {
        (84, 5040)
    } else {
        let w = model_w as usize;
        let h = model_h as usize;
        let default_anchors = (w / 8) * (h / 8) + (w / 16) * (h / 16) + (w / 32) * (h / 32);
        if default_anchors > 0 && data.len().is_multiple_of(default_anchors) {
            (data.len() / default_anchors, default_anchors)
        } else if data.len().is_multiple_of(min_channels) {
            (min_channels, data.len() / min_channels)
        } else {
            return Vec::new();
        }
    };

    if channels < min_channels || anchors == 0 {
        return Vec::new();
    }
    let mut candidates = Vec::new();

    for anchor in 0..anchors {
        let mut class_id = 0;
        let mut confidence = f32::NEG_INFINITY;
        for candidate in 0..S::NUM_CLASSES {
            let score = data[(4 + candidate) * anchors + anchor];
            if score > confidence {
                confidence = score;
                class_id = candidate;
            }
        }
        if !confidence.is_finite() || confidence < ctx.conf_threshold {
            continue;
        }

        let cx = data[anchor];
        let cy = data[anchors + anchor];
        let width = data[2 * anchors + anchor];
        let height = data[3 * anchors + anchor];
        if ![cx, cy, width, height]
            .iter()
            .all(|value| value.is_finite())
            || width <= 0.0
            || height <= 0.0
        {
            continue;
        }

        let raw = NormBox {
            x: ((cx - width * 0.5) / model_w).clamp(0.0, 1.0),
            y: ((cy - height * 0.5) / model_h).clamp(0.0, 1.0),
            w: (width / model_w).clamp(0.0, 1.0),
            h: (height / model_h).clamp(0.0, 1.0),
            confidence,
            class_id: class_id as u32,
            label: S::LABELS.get(class_id).copied(),
        };
        candidates.push(crate::math::unmap_box(
            &raw, ctx.mode, ctx.orig_w, ctx.orig_h,
        ));
    }

    if candidates.is_empty() {
        return candidates;
    }
    crate::math::fast_nms(&mut candidates, ctx.iou_threshold);
    candidates
}

/// 工业级通用检测器（支持装配任意自定义解码器 `D` 与异构推理会话 `Sess`）
pub struct GenericDetector<
    S: YoloSpec,
    D: YoloDecoder = Yolov8SpecDecoder<S>,
    Sess: NpuSession = RuntimeSession,
> {
    pub session: Sess,
    pub transform: HwLetterbox,
    pub decoder: D,
    pub config: StandardYoloConfig,
    pub custom_label: Option<String>,
    /// 预处理/推理连续失败追踪（跨平台，纯计数，不推送业务告警）
    pub failure_tracker: FailureTracker,
    _spec: PhantomData<S>,
}

impl<S: YoloSpec, D: YoloDecoder, Sess: NpuSession + std::fmt::Debug> std::fmt::Debug
    for GenericDetector<S, D, Sess>
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GenericDetector")
            .field("session", &self.session)
            .field("transform", &self.transform)
            .field("config", &self.config)
            .finish()
    }
}

/// 标准开箱即用 YOLOv8/v11 检测器类型别名
pub type GenericYoloDetector<S> = GenericDetector<S, Yolov8SpecDecoder<S>, RuntimeSession>;

fn normalize_custom_label(label: Option<&str>) -> Option<String> {
    label.filter(|s| !s.is_empty()).map(str::to_owned)
}

impl<S: YoloSpec, D: YoloDecoder + Default> AlgoPlugin for GenericDetector<S, D, RuntimeSession> {
    type Config = StandardYoloConfig;

    fn init(ctx: &InitContext<'_>, mut config: Self::Config) -> Result<Self, AlgoError> {
        let env = ctx.load_env();
        config.apply_env(&env);

        let model_file = env.resolve_model_path(ctx.package_root, "MODEL_PATH", S::MODEL_PATH)?;

        // 回退策略由宿主自报的 `platform_id` 与是否处于安装自检决定，
        // 允许调用方（本地开发工具）显式声明，并由包私有 `.env` 提供**显式**覆盖。
        // 自检模式下策略强制为 `RequireHardware`，因此无硬件时 `init` 直接失败，
        // 宿主 `instance_create` 返回 `AV_ERR_MODEL_LOAD_FAILED`，自检不会误判通过。
        let policy = RuntimeSession::resolve_policy(
            ctx.is_self_test,
            ctx.platform_id,
            Some(&env),
            ctx.fallback_policy_override,
        );
        let core_mask = ctx.target_core_mask();
        let session =
            RuntimeSession::open_with_options(ctx.package_root, &model_file, policy, core_mask)?;

        let transform = HwLetterbox::new(S::INPUT_DIM.0, S::INPUT_DIM.1);
        let decoder = D::default();
        let custom_label = normalize_custom_label(config.custom_alarm_label.as_deref());

        Ok(Self {
            session,
            transform,
            decoder,
            config,
            custom_label,
            failure_tracker: FailureTracker::new(DiagnosticConfig::default()),
            _spec: PhantomData,
        })
    }

    fn process(
        &mut self,
        frame: SafeFrame<'_>,
        emitter: &mut ResultEmitter<'_>,
    ) -> Result<(), AlgoError> {
        // 仅对硬件段（预处理 + 推理）计成败：`FailureTracker` 表征的是硬件降级状态。
        // 解码与结果发射属于业务侧，其失败（如宿主回调断开）不得计入硬件失败，
        // 否则会累积到阈值并打出伪造的“硬件恢复正常”日志。
        let boxes = match self.infer(frame) {
            Ok(boxes) => {
                self.failure_tracker.record_success();
                boxes
            }
            Err(error) => {
                self.failure_tracker.record_failure();
                return Err(error);
            }
        };

        if !boxes.is_empty() {
            emitter.emit_detections_with_label(&boxes, self.custom_label.as_deref())?;
        }
        Ok(())
    }

    fn flush(&mut self, _emitter: &mut ResultEmitter<'_>) -> Result<(), AlgoError> {
        self.failure_tracker.reset();
        Ok(())
    }

    fn update_config(&mut self, new_config: Self::Config) -> Result<(), AlgoError> {
        self.custom_label = normalize_custom_label(new_config.custom_alarm_label.as_deref());
        self.config = new_config;
        Ok(())
    }
}

impl<S: YoloSpec, D: YoloDecoder + Default> GenericDetector<S, D, RuntimeSession> {
    /// 硬件段：预处理（`Transform`）与推理（`NpuSession`），产出解码后的候选框。
    ///
    /// 该方法即 `FailureTracker` 的计数边界；结果发射被刻意排除在外。
    fn infer(&mut self, frame: SafeFrame<'_>) -> Result<Vec<NormBox>, AlgoError> {
        let (buf, mode) = self.transform.apply(&frame)?;
        let orig_w = frame.width();
        let orig_h = frame.height();

        self.session.infer_with(&buf, |net_out| {
            let decode_ctx = YoloDecodeContext {
                output: net_out,
                conf_threshold: self.config.confidence_threshold,
                iou_threshold: self.config.iou_threshold,
                mode: &mode,
                orig_w,
                orig_h,
            };

            self.decoder.decode(&decode_ctx)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::FallbackPolicy;
    use crate::testing::{LocalPluginRunner, MockFrameBuilder};

    struct TestHelmetSpec;
    impl YoloSpec for TestHelmetSpec {
        const INPUT_DIM: (u32, u32) = (640, 640);
        const NUM_CLASSES: usize = 2;
        const LABELS: &'static [&'static str] = &["Hardhat", "NO-Hardhat"];
        const MODEL_PATH: &'static str = "model/model.rknn";
    }

    type TestHelmetDetector = GenericYoloDetector<TestHelmetSpec>;

    #[test]
    fn test_generic_yolo_detector_lifecycle() {
        let temp_dir = std::env::temp_dir().join(format!("algo_test_{}", uuid::Uuid::new_v4()));
        let model_dir = temp_dir.join("model");
        std::fs::create_dir_all(&model_dir).expect("create model dir");
        std::fs::write(model_dir.join("model.rknn"), b"mock rknn bytes").expect("write model");

        let ctx = InitContext {
            package_root: &temp_dir,
            platform_id: "test-platform",
            instance_id: "inst-0",
            is_self_test: false,
            fallback_policy_override: None,
            wire_placement: None,
        };

        let mut detector = TestHelmetDetector::init(&ctx, StandardYoloConfig::default())
            .expect("init should succeed with fallback");

        let frame = MockFrameBuilder::new()
            .dimensions(1920, 1080)
            .to_nv12(16)
            .build();

        let (elapsed_ms, boxes) = LocalPluginRunner::run_once(&mut detector, frame.as_safe_frame())
            .expect("process should succeed");
        assert!(elapsed_ms >= 0.0);
        assert_eq!(boxes.len(), 2, "CPU fallback 应完成 FP32 检测后处理");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    /// 本机是否真实具备 RKNN 运行时
    ///
    /// 仅用于区分"硬门合法放行（真机）"与"硬门失效（开发机却成功）"。
    /// 未编译 `rknn` feature 时，真实硬件路径在该构建下物理上不存在。
    #[cfg(feature = "rknn")]
    fn has_real_rknn_runtime(package_root: &std::path::Path) -> bool {
        crate::runtime::platforms::rockchip::RknnRuntime::load(package_root).is_ok()
    }

    #[cfg(not(feature = "rknn"))]
    fn has_real_rknn_runtime(_package_root: &std::path::Path) -> bool {
        false
    }

    /// 构造含 `model/model.rknn` 的临时算法包目录，可选写入包私有 `.env`
    fn temp_detector_package(env_content: Option<&str>) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("algo_test_{}", uuid::Uuid::new_v4()));
        let model_dir = dir.join("model");
        std::fs::create_dir_all(&model_dir).expect("create model dir");
        std::fs::write(model_dir.join("model.rknn"), b"mock rknn bytes").expect("write model");
        if let Some(content) = env_content {
            std::fs::write(dir.join(".env"), content).expect("write .env");
        }
        dir
    }

    /// A7（SDK 侧）：安装自检模式下，模拟降级不得让 `init` 成功
    ///
    /// 这是自检硬门在插件边界的证据：宿主 `AlgoSandbox` 从 `instance_create` 拿到
    /// 非 `AV_OK` 即判定自检失败，因此无需宿主新增校验代码。
    /// 前提是"无硬件运行时"；开发机天然满足，真机上则合法通过。
    #[test]
    fn test_self_test_init_requires_real_hardware() {
        let temp_dir = temp_detector_package(None);
        let ctx = InitContext {
            package_root: &temp_dir,
            platform_id: "linux-rknn",
            instance_id: "inst-self-test",
            is_self_test: true,
            fallback_policy_override: None,
            wire_placement: None,
        };

        match TestHelmetDetector::init(&ctx, StandardYoloConfig::default()) {
            Ok(_) => assert!(
                has_real_rknn_runtime(&temp_dir),
                "自检模式下不得用模拟会话冒充成功：本机无 librknnrt，但 init 却返回了会话"
            ),
            Err(error) => {
                assert!(
                    matches!(error, AlgoError::ModelLoad { .. }),
                    "自检硬门失败必须映射为 ModelLoad（-5），实际: {error:?}"
                );
                assert_eq!(error.to_c_status(), crate::c_abi::AV_ERR_MODEL_LOAD_FAILED);
                let reason = error.to_string();
                assert!(
                    reason.contains("ALLOW_CPU_FALLBACK"),
                    "错误必须给运维可操作指引: {reason}"
                );
            }
        }

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    /// A5：非硬件平台（开发机）上 `GenericDetector::init` 仍可用无驱动回退
    #[test]
    fn test_init_keeps_fallback_on_non_hardware_platform() {
        let temp_dir = temp_detector_package(None);
        let ctx = InitContext {
            package_root: &temp_dir,
            // 归一化后不属于硬件平台，即使 `is_self_test` 为 false 也应保持 Allow
            platform_id: "macos-arm64-coreml",
            instance_id: "inst-dev",
            is_self_test: false,
            fallback_policy_override: None,
            wire_placement: None,
        };

        let detector = TestHelmetDetector::init(&ctx, StandardYoloConfig::default())
            .expect("非硬件平台必须保留无驱动回退能力");
        assert!(detector.session.is_fallback());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    /// 本地开发工具路径：显式声明 `Allow` 在硬件平台上仍可跑模拟回退
    ///
    /// 这是 `run_local` 摆脱 `ALLOW_CPU_FALLBACK` 环境变量的直接证据：
    /// 开发意图写在代码里，而不是部署期外部文件里。
    #[test]
    fn test_init_explicit_allow_enables_local_dev_fallback_on_hardware_platform() {
        let temp_dir = temp_detector_package(None);
        let ctx = InitContext {
            package_root: &temp_dir,
            // 硬件平台：默认策略本会是 RequireHardware
            platform_id: "linux-rknn",
            instance_id: "inst-local-dev",
            is_self_test: false,
            fallback_policy_override: Some(FallbackPolicy::Allow),
            wire_placement: None,
        };

        let detector = TestHelmetDetector::init(&ctx, StandardYoloConfig::default())
            .expect("本地开发工具的显式 Allow 必须能在硬件平台上跑通模拟回退");
        assert!(detector.session.is_fallback());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    /// 回归锁：显式声明**不可**翻越安装自检硬门
    ///
    /// 若允许翻越，安装自检就又能用模拟会话冒充真实推理（本任务要消灭的失败模式）
    /// ——哪怕调用方拿着本地开发工具的心态传了 `Allow`。
    #[test]
    fn test_init_self_test_gate_ignores_explicit_allow() {
        let temp_dir = temp_detector_package(None);
        let ctx = InitContext {
            package_root: &temp_dir,
            platform_id: "linux-rknn",
            instance_id: "inst-self-test-explicit",
            is_self_test: true,
            fallback_policy_override: Some(FallbackPolicy::Allow),
            wire_placement: None,
        };

        match TestHelmetDetector::init(&ctx, StandardYoloConfig::default()) {
            Ok(_) => assert!(
                has_real_rknn_runtime(&temp_dir),
                "自检硬门必须忽略显式 Allow：本机无 librknnrt，但 init 却返回了会话"
            ),
            Err(error) => assert!(
                matches!(error, AlgoError::ModelLoad { .. }),
                "自检硬门失败必须映射为 ModelLoad（-5），实际: {error:?}"
            ),
        }

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    /// A6：`.env` 不得推翻安装自检硬门，且历史平台别名必须归一化为硬件平台
    #[test]
    fn test_init_self_test_gate_survives_env_override() {
        let temp_dir = temp_detector_package(Some("ALLOW_CPU_FALLBACK=1\n"));
        let ctx = InitContext {
            package_root: &temp_dir,
            platform_id: "linux-arm64-rknn", // 历史别名，必须归一化为硬件平台
            instance_id: "inst-self-test-env",
            is_self_test: true,
            fallback_policy_override: None,
            wire_placement: None,
        };

        match TestHelmetDetector::init(&ctx, StandardYoloConfig::default()) {
            Ok(_) => assert!(
                has_real_rknn_runtime(&temp_dir),
                "`.env` 不得推翻安装自检的硬件硬门"
            ),
            Err(error) => assert!(
                matches!(error, AlgoError::ModelLoad { .. }),
                "实际错误: {error:?}"
            ),
        }

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    // 测试插入自定义魔改解码器 (Custom Decoder)
    struct CustomMockDecoder;
    impl YoloDecoder for CustomMockDecoder {
        fn decode(&self, _ctx: &YoloDecodeContext<'_>) -> Result<Vec<NormBox>, AlgoError> {
            let label = "custom_box";
            Ok(vec![
                NormBox::new(0.1, 0.1, 0.2, 0.2, 0.99, 0).with_label(label)
            ])
        }
    }
    impl Default for CustomMockDecoder {
        fn default() -> Self {
            Self
        }
    }

    type CustomTestDetector = GenericDetector<TestHelmetSpec, CustomMockDecoder>;

    #[test]
    fn test_custom_decoder_strategy() {
        let temp_dir = std::env::temp_dir().join(format!("algo_test_{}", uuid::Uuid::new_v4()));
        let model_dir = temp_dir.join("model");
        std::fs::create_dir_all(&model_dir).expect("create model dir");
        std::fs::write(model_dir.join("model.rknn"), b"mock rknn bytes").expect("write model");

        let ctx = InitContext {
            package_root: &temp_dir,
            platform_id: "test-platform",
            instance_id: "inst-custom",
            is_self_test: false,
            fallback_policy_override: None,
            wire_placement: None,
        };

        let mut detector = CustomTestDetector::init(&ctx, StandardYoloConfig::default())
            .expect("init should succeed with custom decoder");

        let frame = MockFrameBuilder::new()
            .dimensions(1920, 1080)
            .to_nv12(16)
            .build();

        let (_elapsed_ms, boxes) =
            LocalPluginRunner::run_once(&mut detector, frame.as_safe_frame())
                .expect("process should succeed");
        assert_eq!(boxes.len(), 1);
        assert_eq!(boxes[0].label, Some("custom_box"));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_generic_yolo_config_update_custom_label() {
        let temp_dir = std::env::temp_dir().join(format!("algo_test_{}", uuid::Uuid::new_v4()));
        let model_dir = temp_dir.join("model");
        std::fs::create_dir_all(&model_dir).expect("create model dir");
        std::fs::write(model_dir.join("model.rknn"), b"mock rknn bytes").expect("write model");

        let ctx = InitContext {
            package_root: &temp_dir,
            platform_id: "test-platform",
            instance_id: "inst-config-update",
            is_self_test: false,
            fallback_policy_override: None,
            wire_placement: None,
        };

        let mut detector = TestHelmetDetector::init(&ctx, StandardYoloConfig::default())
            .expect("init should succeed");
        assert_eq!(detector.custom_label, None);

        detector
            .update_config(StandardYoloConfig {
                confidence_threshold: 0.5,
                iou_threshold: 0.5,
                custom_alarm_label: Some("safety_violation".to_string()),
                ..Default::default()
            })
            .expect("update_config should succeed");
        assert_eq!(
            detector.custom_label.as_deref(),
            Some("safety_violation"),
            "配置热更新应安全接管 custom_label"
        );

        let frame = MockFrameBuilder::new()
            .dimensions(1920, 1080)
            .to_nv12(16)
            .build();
        let (_elapsed_ms, boxes) =
            LocalPluginRunner::run_once(&mut detector, frame.as_safe_frame())
                .expect("process should succeed");
        assert_eq!(boxes.len(), 2);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    /// 锁住连续失败追踪契约：成功累加后归零、失败递增、`flush()` 静默重置。
    ///
    /// 这对应 [algo-sdk 规范](../../../../.trellis/spec/algo-sdk/backend/algo-sdk-guidelines.md)
    /// 「连续失败属于硬件内部状态，通过日志与健康检查暴露」的落点；若追踪器被
    /// 再次从检测器主流程剥离，本用例必须失败。
    #[test]
    fn test_failure_tracker_records_outcome_and_flushes() {
        let temp_dir = std::env::temp_dir().join(format!("algo_test_{}", uuid::Uuid::new_v4()));
        let model_dir = temp_dir.join("model");
        std::fs::create_dir_all(&model_dir).expect("create model dir");
        std::fs::write(model_dir.join("model.rknn"), b"mock rknn bytes").expect("write model");

        let ctx = InitContext {
            package_root: &temp_dir,
            platform_id: "test-platform",
            instance_id: "inst-health",
            is_self_test: false,
            fallback_policy_override: None,
            wire_placement: None,
        };
        let mut detector = TestHelmetDetector::init(&ctx, StandardYoloConfig::default())
            .expect("init should succeed");

        // 初始化后计数清零
        assert_eq!(detector.failure_tracker.consecutive_failures(), 0);

        let frame = MockFrameBuilder::new()
            .dimensions(1920, 1080)
            .to_nv12(16)
            .build();
        LocalPluginRunner::run_once(&mut detector, frame.as_safe_frame())
            .expect("process should succeed");

        // 成功路径保持连续失败为 0（不得因单帧成功而累积）
        assert_eq!(detector.failure_tracker.consecutive_failures(), 0);
        assert_eq!(detector.failure_tracker.total_failures(), 0);

        // flush 静默重置连续计数，且不伪造失败总量
        detector.failure_tracker.record_failure();
        assert_eq!(detector.failure_tracker.consecutive_failures(), 1);
        let mut emitter_owner = crate::testing::MockEmitter::new();
        // SAFETY: emitter_owner 存活至 flush 调用结束
        let mut emitter = unsafe { emitter_owner.as_emitter(1) };
        detector.flush(&mut emitter).expect("flush should succeed");
        assert_eq!(detector.failure_tracker.consecutive_failures(), 0);
        assert_eq!(detector.failure_tracker.total_failures(), 1);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    /// 计数边界：硬件段成功但结果发射失败时，不得计为硬件失败。
    ///
    /// 发射失败（如宿主回调断开）属于业务侧故障；若计入 `FailureTracker`，
    /// 持续断连会累积到阈值并伪造硬件降级信号。
    #[test]
    fn test_emitter_failure_does_not_count_as_hardware_failure() {
        let temp_dir = std::env::temp_dir().join(format!("algo_test_{}", uuid::Uuid::new_v4()));
        let model_dir = temp_dir.join("model");
        std::fs::create_dir_all(&model_dir).expect("create model dir");
        std::fs::write(model_dir.join("model.rknn"), b"mock rknn bytes").expect("write model");

        let ctx = InitContext {
            package_root: &temp_dir,
            platform_id: "test-platform",
            instance_id: "inst-emitter",
            is_self_test: false,
            fallback_policy_override: None,
            wire_placement: None,
        };
        let mut detector = TestHelmetDetector::init(&ctx, StandardYoloConfig::default())
            .expect("init should succeed");

        // `request_id = 0` 使 C 回调拒绝该次发射。
        let mut emitter_owner = crate::testing::MockEmitter::new();
        // SAFETY: emitter_owner 存活至 emitter 使用完毕
        let mut emitter = unsafe { emitter_owner.as_emitter(0) };

        let frame = MockFrameBuilder::new()
            .dimensions(1920, 1080)
            .to_nv12(16)
            .build();
        let result = detector.process(frame.as_safe_frame(), &mut emitter);

        // 无论发射成败，硬件段已经走完，计数器不得被发射路径污染
        assert_eq!(
            detector.failure_tracker.total_failures(),
            0,
            "结果发射失败不得计入硬件失败计数"
        );
        // 保持编译期使用，避免未使用告警掩盖真实行为
        let _ = result;

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
