//! 通用 YOLO 系列检测器模板与解码器策略 (`GenericYoloDetector` & `YoloDecoder`)
//!
//! 提供类似 PyTorch / TorchVision 的标准化模型抽象：
//! 1. `YoloSpec`: 声明模型规格（分辨率、类别数、标签、权重相对路径）；
//! 2. `YoloDecoder`: 解码策略契约，支持官方标准模型或任意魔改定制头；
//! 3. `GenericDetector`: 自动闭环“配置注入 -> 硬件预处理 -> NPU 异构推理 -> 解码 -> 结果发射”；
//! 4. `GenericYoloDetector<Spec>`: 默认绑定标准 YOLOv8/v11 解码器与平台会话，实现 ~15 行代码交付算法包。

use std::marker::PhantomData;

use crate::algo_config;
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
        let parse_cfg = Yolov8RknnConfig {
            model_input_w: S::INPUT_DIM.0 as f32,
            model_input_h: S::INPUT_DIM.1 as f32,
            dfl_bins: S::DFL_BINS,
            num_classes: S::NUM_CLASSES,
            use_score_sum: S::USE_SCORE_SUM,
        };

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
        let session = RuntimeSession::open(ctx.package_root, &model_file)?;

        let transform = HwLetterbox::new(S::INPUT_DIM.0, S::INPUT_DIM.1);
        let decoder = D::default();
        let custom_label = normalize_custom_label(config.custom_alarm_label.as_deref());

        Ok(Self {
            session,
            transform,
            decoder,
            config,
            custom_label,
            _spec: PhantomData,
        })
    }

    fn process(
        &mut self,
        frame: SafeFrame<'_>,
        emitter: &mut ResultEmitter<'_>,
    ) -> Result<(), AlgoError> {
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

            let boxes = self.decoder.decode(&decode_ctx)?;

            if !boxes.is_empty() {
                emitter.emit_detections_with_label(&boxes, self.custom_label.as_deref())?;
            }
            Ok(())
        })
    }

    fn update_config(&mut self, new_config: Self::Config) -> Result<(), AlgoError> {
        self.custom_label = normalize_custom_label(new_config.custom_alarm_label.as_deref());
        self.config = new_config;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
