# Technical Design: PyTorch-like Composable Edge AI Algorithm Ecosystem

## 1. 架构总览 (Architecture Overview)

本项目旨在将 PyTorch 的核心工程理念（组件化、流式预处理、树状组合、声明式交付）引入 Heimdall 边缘端异构算力环境。

```
┌───────────────────────────────────────────────────────────────────────────────┐
│                    User-Facing Algorithm Package API                          │
│                                                                               │
│  [Pattern A: 单模型快速交付 (~15行)]      [Pattern B: 级联多模型与时序拼装]       │
│  type Detector = GenericYoloDetector<S>   CascadePipeline { Det, Align, Ext, Trk }│
│  export_algo!(Detector, ...)             export_algo!(CascadePipeline, ...)   │
└──────────────────────────────────────┬────────────────────────────────────────┘
                                       │
┌──────────────────────────────────────▼────────────────────────────────────────┐
│                     algo-sdk 核心组件库 (Core Modules)                        │
│ ┌──────────────────────────┐ ┌─────────────────────────┐ ┌──────────────────┐ │
│ │ cv::transforms           │ │ cv::models::yolo        │ │ track::bytetrack │ │
│ │ - Transform trait        │ │ - YoloSpec trait        │ │ - ByteTracker    │ │
│ │ - HwLetterbox            │ │ - GenericYoloDetector   │ │ - KalmanFilter   │ │
│ │ - HwCrop                 │ │ - parse_yolov8_int8     │ │ - HungarianMatch │ │
│ └──────────────────────────┘ └─────────────────────────┘ └──────────────────┘ │
└──────────────────────────────────────┬────────────────────────────────────────┘
                                       │ 驱动
┌──────────────────────────────────────▼────────────────────────────────────────┐
│                   异构算力与硬件加速层 (Heterogeneous HAL)                    │
│ ┌───────────────────────────────────────┐ ┌─────────────────────────────────┐ │
│ │ 2D 加速器 SPI (CvEngine)              │ │ 推理引擎 SPI (InferenceSession) │ │
│ │ - Rockchip RGA (16B 对齐, DMA-BUF)    │ │ - Rockchip RKNN (INT8 / NPU)    │ │
│ │ - 华为 Ascend VPC (128x16B 对齐, NV12) │ │ - 华为 Ascend ACL (.om / AIPP)  │ │
│ │ - NVIDIA CUDA/VIC (Pitch-Linear)      │ │ - NVIDIA TensorRT (.engine)     │ │
│ │ - Apple CoreVideo (CVPixelBuffer)     │ │ - Apple CoreML (ANE / Metal)    │ │
│ └───────────────────────────────────────┘ └─────────────────────────────────┘ │
└───────────────────────────────────────────────────────────────────────────────┘
```

---

## 2. 核心组件详细设计 (Detailed Module Design)

### 2.1 硬件预处理算子链 (`algo_sdk::cv::transforms`)

模仿 `torchvision.transforms`，将异构 2D 硬件操作抽象为统一的 `Transform`：

```rust
use crate::cv::buffer::CvBuffer;
use crate::cv::types::{CropRect, PreprocessMode};
use crate::error::AlgoError;
use crate::frame::SafeFrame;

/// 硬件加速变换算子契约
pub trait Transform: Send + Sync {
    /// 对输入帧执行变换，产出驻留在物理设备显存/DMA-BUF 中的 CvBuffer 与几何映射模式
    fn apply(&self, frame: &SafeFrame<'_>) -> Result<(CvBuffer, PreprocessMode), AlgoError>;
}

/// 硬件级等比缩放与居中填充算子 (Letterbox)
#[derive(Debug, Clone)]
pub struct HwLetterbox {
    pub target_w: u32,
    pub target_h: u32,
    pub fill_color: [u8; 3],
}

impl HwLetterbox {
    pub fn new(target_w: u32, target_h: u32) -> Self {
        Self {
            target_w,
            target_h,
            fill_color: [114, 114, 114],
        }
    }

    pub fn with_fill(mut self, fill: [u8; 3]) -> Self {
        self.fill_color = fill;
        self
    }
}

impl Transform for HwLetterbox {
    fn apply(&self, frame: &SafeFrame<'_>) -> Result<(CvBuffer, PreprocessMode), AlgoError> {
        let engine = crate::cv::default_engine();
        engine.letterbox(frame, self.target_w, self.target_h, self.fill_color)
    }
}
```

### 2.2 解码器策略解耦与标准实现 (`algo_sdk::cv::models::yolo::decoder`)

为了完美支持工业界各种魔改 YOLO（4 尺度 P2 微小目标头、Anchor 先验框机制、单张量融合输出等），将后处理张量解码抽象为策略 Trait：

```rust
use crate::cv::types::PreprocessMode;
use crate::error::AlgoError;
use crate::math::NormBox;
use crate::rknn::RknnTensorOutput;

/// 输出张量解码策略 Trait
pub trait YoloDecoder: Send + Sync + 'static {
    fn decode(
        &self,
        outputs: &[RknnTensorOutput],
        conf_threshold: f32,
        iou_threshold: f32,
        mode: &PreprocessMode,
        orig_w: u32,
        orig_h: u32,
    ) -> Result<Vec<NormBox>, AlgoError>;
}

/// 官方标准 YOLOv8 / YOLOv11 解码器（基于 DFL 与可选 Score-Sum 快筛）
#[derive(Debug, Clone, Copy, Default)]
pub struct Yolov8StandardDecoder<const USE_SCORE_SUM: bool = true, const DFL_BINS: usize = 16>;

impl<const USE_SCORE_SUM: bool, const DFL_BINS: usize> Yolov8StandardDecoder<USE_SCORE_SUM, DFL_BINS> {
    pub const fn new() -> Self {
        Self
    }
}
```

### 2.3 工业级通用检测器骨架 (`algo_sdk::cv::models::yolo`)

通用检测器采用策略模式，默认使用 `Yolov8StandardDecoder`，同时支持插入自定义魔改解码器：

```rust
use crate::algo_config;
use crate::cv::postprocess::{parse_yolov8_int8, Yolov8ParseContext, Yolov8RknnConfig};
use crate::cv::transforms::HwLetterbox;
use crate::emitter::ResultEmitter;
use crate::error::AlgoError;
use crate::frame::SafeFrame;
use crate::plugin::{AlgoPlugin, InitContext};
use crate::rknn::{RknnRuntime, RknnSession};

/// YOLO 架构规格定义元信息
pub trait YoloSpec: 'static {
    /// 模型输入分辨率 (宽, 高)
    const INPUT_DIM: (u32, u32);
    /// 类别数量
    const NUM_CLASSES: usize;
    /// 类别标签字符切片
    const LABELS: &'static [&'static str];
    /// 模型权重文件相对路径（相对于 package_root）
    const MODEL_PATH: &'static str;
    /// 是否使用 9-tensor score_sum 预筛优化 (YOLOv8 INT8)
    const USE_SCORE_SUM: bool = true;
    /// DFL bins 数量 (默认为 16)
    const DFL_BINS: usize = 16;
}

algo_config! {
    /// 标准检测配置（自动包含 explicit_fields 与三级优先级支持）
    #[derive(Debug, Clone, PartialEq)]
    pub struct StandardYoloConfig {
        pub confidence_threshold: f32 = 0.45,
        pub iou_threshold: f32 = 0.45,
        pub custom_alarm_label: Option<String> = None,
    }
}

/// 工业级通用检测器（支持自定义解码器 D）
pub struct GenericDetector<S: YoloSpec, D: YoloDecoder> {
    pub session: RknnSession,
    pub transform: HwLetterbox,
    pub decoder: D,
    pub config: StandardYoloConfig,
    _spec: std::marker::PhantomData<S>,
}

/// 标准 YOLOv8/v11 检测器类型别名（开箱即用 15 行交付）
pub type GenericYoloDetector<S> = GenericDetector<S, Yolov8SpecDecoder<S>>;

/// 自动绑定 S 元信息的默认解码器代理
pub struct Yolov8SpecDecoder<S: YoloSpec>(std::marker::PhantomData<S>);

impl<S: YoloSpec> Default for Yolov8SpecDecoder<S> {
    fn default() -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<S: YoloSpec> YoloDecoder for Yolov8SpecDecoder<S> {
    fn decode(
        &self,
        outputs: &[RknnTensorOutput],
        conf_threshold: f32,
        iou_threshold: f32,
        mode: &crate::cv::types::PreprocessMode,
        orig_w: u32,
        orig_h: u32,
    ) -> Result<Vec<crate::math::NormBox>, AlgoError> {
        let parse_cfg = Yolov8RknnConfig {
            model_input_w: S::INPUT_DIM.0 as f32,
            model_input_h: S::INPUT_DIM.1 as f32,
            dfl_bins: S::DFL_BINS,
            num_classes: S::NUM_CLASSES,
            use_score_sum: S::USE_SCORE_SUM,
        };

        let boxes = parse_yolov8_int8(&Yolov8ParseContext {
            branches: outputs,
            config: &parse_cfg,
            conf_threshold,
            iou_threshold,
            labels: S::LABELS,
            label_fn: None,
            mode,
            orig_w,
            orig_h,
        });

        Ok(boxes)
    }
}

impl<S: YoloSpec, D: YoloDecoder + Default> AlgoPlugin for GenericDetector<S, D> {
    type Config = StandardYoloConfig;

    fn init(ctx: &InitContext<'_>, mut config: Self::Config) -> Result<Self, AlgoError> {
        let env = ctx.load_env();
        config.apply_env(&env);

        let model_file = ctx.package_root.join(S::MODEL_PATH);
        let session = match RknnRuntime::load(ctx.package_root) {
            Ok(runtime) => RknnSession::new(runtime, &model_file)?,
            Err(e) => {
                tracing::warn!(reason = ?e, "未检测到物理运行时，启动 CPU 模拟会话");
                RknnSession::new_fallback(&model_file)?
            }
        };

        let transform = HwLetterbox::new(S::INPUT_DIM.0, S::INPUT_DIM.1);
        let decoder = D::default();

        Ok(Self {
            session,
            transform,
            decoder,
            config,
            _spec: std::marker::PhantomData,
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

        let run_postprocess = |outputs: &[crate::rknn::RknnTensorOutput]| {
            let boxes = self.decoder.decode(
                outputs,
                self.config.confidence_threshold,
                self.config.iou_threshold,
                &mode,
                orig_w,
                orig_h,
            )?;

            if !boxes.is_empty() {
                emitter.emit_detection_boxes(&boxes)?;
            }
            Ok(())
        };

        if let Some(fd) = buf.as_dma_buf_fd() {
            let size = S::INPUT_DIM.0 * S::INPUT_DIM.1 * 3;
            self.session.infer_with_dma_buf(fd, size, run_postprocess)
        } else if let Some(bytes) = buf.as_host_bytes() {
            self.session.infer_with_host_bytes(bytes, run_postprocess)
        } else {
            Err(AlgoError::IncompatibleFrame)
        }
    }

    fn update_config(&mut self, new_config: Self::Config) -> Result<(), AlgoError> {
        self.config = new_config;
        Ok(())
    }
}
```

---

## 3. 级联与时序组合机制 (Cascade & Temporal Composition)

级联流水线无需单一庞大模板，而是通过组件在 `process()` 中自由组合：

1. **零拷贝中间流转**：
   - 阶段 1：`ModelA` 推理产生 `BoundingBox`；
   - 阶段 1.5：`HwCrop` 或 `HwAffineWarp` 直接以 `SafeFrame` 原图作为输入，利用 2D 硬件（RGA/VPC）将子区域抠图缩放到目标尺寸，产出 `CvBuffer`；
   - 阶段 2：`ModelB` 接收该 `CvBuffer` 执行特征提取或二次分类。
2. **状态机常驻**：
   - `ByteTracker`、滑动窗口 `TemporalVerifier` 直接作为流水线结构体内部字段，跨帧更新状态。

---

## 4. 跨平台扩展接口预留 (Ascend / NVIDIA SPI Hooks)

为了保证后续 Ascend 与 NVIDIA 平台接入时保持 100% 接口兼容性：
1. **`CvBuffer` 保持 Opaque 句柄**：内部通过 `CvBufferKind::AscendDeviceMemory` 和预留的 `CvBufferKind::CudaDeviceMemory` 封装指针；
2. **`CvEngine` 保持统一抽象**：
   - Rockchip: `RgaCvEngine` (RGB888 输出, 16 字节对齐)；
   - 华为 Ascend: `AscendVpcEngine` (NV12 输出, 128×16 字节对齐, 配合 NPU AIPP)；
   - NVIDIA: `CudaCvEngine` (CUDA Fused Kernel, Pitch-Linear)。
3. **`InferenceSession` 保持统一抽象**：接收 `&CvBuffer` 并返回 `UnifiedTensorOutputs`。

---

## 5. 质量门禁与安全性保证

- **无内存分配热路径**：所有输入输出均使用池化显存；
- **Panic 隔离**：继续通过 `export_algo!` 的 `catch_unwind` 隔离底层崩溃；
- **全平台编译门禁**：保证 `cargo check`, `cargo test`, `cargo clippy -- -D warnings` 在 root 工作区及 4 个算法平台工作区（`macos`, `rk3568`, `rk3576`, `rk3588`）全绿。
