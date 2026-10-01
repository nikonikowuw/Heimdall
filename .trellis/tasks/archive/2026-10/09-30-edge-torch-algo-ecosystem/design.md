# Technical Design: PyTorch-like Composable Edge AI Algorithm Ecosystem

> **文档状态**：2026-10-01 按**实际落地实现**校准（as-built）。
> 原始设计期草案中的模块路径与类型签名已与代码发生系统性漂移，本文档以源码为准重写；
> 漂移清单保留在 §7 以便审计。所有代码片段均可对应到现有源码。

## 1. 架构总览 (Architecture Overview)

将 PyTorch 的核心工程理念（组件化、流式预处理、树状组合、声明式交付）引入 Heimdall 边缘端异构算力环境。

```
┌───────────────────────────────────────────────────────────────────────────────┐
│                    User-Facing Algorithm Package API                          │
│                                                                               │
│  [Pattern A: 单模型声明式交付]           [Pattern B: 包内手工编排]              │
│  impl YoloSpec for MySpec                impl AlgoPlugin for MyDetector        │
│  type D = GenericYoloDetector<MySpec>    (多模型级联、状态机、特征比对)          │
│  export_algo!(D, ...)                    export_algo!(MyDetector, ...)         │
└──────────────────────────────────────┬────────────────────────────────────────┘
                                       │
┌──────────────────────────────────────▼────────────────────────────────────────┐
│                     algo-sdk 核心组件库 (Core Modules)                        │
│ ┌──────────────────────────┐ ┌─────────────────────────┐ ┌──────────────────┐ │
│ │ cv::transforms           │ │ models::yolo            │ │ track::bytetrack │ │
│ │ - Transform trait        │ │ - YoloSpec trait        │ │ - ByteTracker    │ │
│ │ - HwLetterbox            │ │ - GenericDetector<S,D,S>│ │ - KalmanBoxTrack │ │
│ │   (已交付，见 §2.1)       │ │ - YoloDecoder 策略      │ │ - 匈牙利匹配      │ │
│ │ - HwCrop / HwAffine      │ │ - Yolov8SpecDecoder<S>  │ │                  │ │
│ │   (未交付，见 §7)         │ │ - CLS_IS_LOGITS 语义    │ │                  │ │
│ └──────────────────────────┘ └─────────────────────────┘ └──────────────────┘ │
│ ┌───────────────────────────────────────────────────────────────────────────┐ │
│ │ runtime           — NpuSession trait / RuntimeSession 路由 / CPU Fallback  │ │
│ │ cv::platforms     — CvEngine trait + rockchip(RGA) / apple / cpu / host    │ │
│ │ cv::postprocess   — quantize / dfl / yolov8_rknn 解码工具                  │ │
│ │ face              — align / orientation / quality / gallery                │ │
│ └───────────────────────────────────────────────────────────────────────────┘ │
└──────────────────────────────────────┬────────────────────────────────────────┘
                                       │ 驱动
┌──────────────────────────────────────▼────────────────────────────────────────┐
│                   异构算力与硬件加速层 (Heterogeneous HAL)                    │
│ ┌───────────────────────────────────────┐ ┌─────────────────────────────────┐ │
│ │ 2D 加速器 SPI (CvEngine)              │ │ 推理引擎 SPI (NpuSession)       │ │
│ │ [已交付] Rockchip RGA (DMA-BUF)       │ │ [已交付] Rockchip RKNN (INT8)   │ │
│ │ [已交付] Apple CoreVideo              │ │ [已交付] CPU Fallback (调试)     │ │
│ │ [已交付] CPU / 宿主 AvImageOps 代理    │ │ [未开始] Ascend ACL / AIPP      │ │
│ │ [未开始] Ascend VPC (128×16 对齐)      │ │ [未开始] NVIDIA TensorRT        │ │
│ │ [未开始] NVIDIA CUDA/VIC              │ │ [未开始] Apple CoreML (ANE)     │ │
│ └───────────────────────────────────────┘ └─────────────────────────────────┘ │
└───────────────────────────────────────────────────────────────────────────────┘
```

**模块归属原则**：`models` 是顶层模块（不在 `cv` 下）；`rknn` 已下沉为 `runtime::platforms::rockchip`，仅通过 `pub use ... as rknn` 保留旧路径别名；`cv` 回归纯粹的 2D 视觉预处理层。

## 2. 核心组件详细设计 (Detailed Module Design)

### 2.1 硬件预处理算子链 (`algo_sdk::cv::transforms`)

模仿 `torchvision.transforms`，将异构 2D 硬件操作抽象为统一的 `Transform`：

```rust
use crate::cv::buffer::CvBuffer;
use crate::cv::types::PreprocessMode;
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

impl Transform for HwLetterbox {
    fn apply(&self, frame: &SafeFrame<'_>) -> Result<(CvBuffer, PreprocessMode), AlgoError> {
        // 经 `active_engine()` 分派到当前实例作用域的引擎（宿主注入 > 平台默认）
        crate::cv::letterbox(frame, self.target_w, self.target_h, self.fill_color)
    }
}
```

**引擎分派链**：`Transform::apply` → `cv::active_engine()` → 线程局部 `ENGINE_STACK`（实例级，由 `with_engine` 压栈）→ 回落 `DEFAULT_ENGINE`（平台默认）→ `RgaCvEngine` / `AppleCvEngine` / `CpuCvEngine`。

**引擎生命周期**：`DEFAULT_ENGINE` 位于 `static OnceLock`，而 Rust 静态变量永不执行 `Drop`；因此由 `DefaultEngineLease`（实例计数归零触发）显式调用 `release_hardware()`，回收 RGA DMA-BUF 导入句柄，避免内核 `rga_mm: Destroy handle ... when the user exits` 噪声。

> **未交付算子**：`HwCrop` 与 `HwAffineWarp5Points` 未实现。ROI 抠图目前经 `CvEngine::crop_rgb` / `cv::crop_rgb` 提供（非 `Transform` trait 实现），五点仿射仅在 CPU 侧 `face::align` 提供。详见 §7。

### 2.2 解码器策略解耦 (`algo_sdk::models::yolo::decoder`)

为适配工业界魔改 YOLO（4 尺度 P2 微小目标头、Anchor 先验框机制、单张量融合输出），后处理张量解码抽象为策略 Trait：

```rust
/// 解码上下文：把「输出 + 阈值 + 几何映射」收敛为单一入参
#[derive(Debug, Clone)]
pub struct YoloDecodeContext<'a> {
    /// 异构推理引擎输出统一抽象（多分支 INT8 / 单张量 FP32）
    pub output: &'a InferenceOutput<'a>,
    pub conf_threshold: f32,
    pub iou_threshold: f32,
    /// Letterbox 几何映射模式，用于坐标反算
    pub mode: &'a PreprocessMode,
    pub orig_w: u32,
    pub orig_h: u32,
}

pub trait YoloDecoder: Send + Sync + 'static {
    fn decode(&self, ctx: &YoloDecodeContext<'_>) -> Result<Vec<NormBox>, AlgoError>;
}

/// 自动绑定 `YoloSpec` 元信息的默认 YOLOv8/v11 解码器
pub struct Yolov8SpecDecoder<S: YoloSpec>(PhantomData<S>);
```

**设计取舍**：模型架构常量（`INPUT_DIM` / `DFL_BINS` / `NUM_CLASSES` / `USE_SCORE_SUM` / `CLS_IS_LOGITS`）挂在 `YoloSpec` 上而非解码器泛型参数上，因此 `Yolov8SpecDecoder<S>` 无需 const 泛型即可复用同一套解码实现——这是与设计期草案（`Yolov8StandardDecoder<const USE_SCORE_SUM, const DFL_BINS>`）的关键差异。

### 2.3 分类分支激活语义 (`CLS_IS_LOGITS`)

> **本节为设计期缺失、实现期发现的关键知识。** 模型导出图若把 sigmoid 移出计算图，score 分支含负值；漏设该常量会把负 logit 当作置信度直接与阈值比较，导致低分尺度**零检出**、其余尺度系统性偏低。

```rust
/// 通过 `Yolov8RknnConfig::from_spec` 在构造期拒绝非法组合，使其无法被表达
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassActivation {
    /// 图中已含 sigmoid：原始值即概率，直接与阈值比较
    Probability,
    /// sigmoid 已移出计算图：原始值含负值，需还原并换算阈值
    Logits,
}
```

两条硬约束：

1. **互斥性**：`score_sum` 预筛依赖 `score_sum >= max_class_score >= 阈值`，该不等式仅在概率语义下成立。因此 **9-tensor + logits 是非法组合**，`Yolov8RknnConfig::from_spec` 在构造期返回 `ConfigParse` 错误而非静默漏检。字段全部私有化，使该组合在类型层面不可表达。
2. **阈值量化方向**：`Logits` 语义下阈值需换算到 logit 空间（`ln(p/(1-p))`，因 sigmoid 单调故 `sigmoid(x) > p ⟺ x > ln(p/(1-p))`）。INT8 量化时**必须向下取整而非四舍五入**——对整数原始值，`v > t` 严格等价于 `v > floor(t)`，舍入会抬高阈值并静默丢弃边界网格。

### 2.4 通用检测器骨架 (`algo_sdk::models::yolo`)

```rust
use crate::cv::diagnostic::{DiagnosticConfig, FailureTracker};
use crate::cv::transforms::HwLetterbox;
use crate::models::yolo::{StandardYoloConfig, YoloDecodeContext, YoloDecoder, YoloSpec};
use crate::runtime::{NpuSession, RuntimeSession};

/// YOLO 架构规格定义元信息
pub trait YoloSpec: Send + Sync + 'static {
    const INPUT_DIM: (u32, u32);
    const NUM_CLASSES: usize;
    const LABELS: &'static [&'static str];
    const MODEL_PATH: &'static str;
    /// 9-tensor score_sum 预筛（YOLOv8 INT8），默认 true
    const USE_SCORE_SUM: bool = true;
    const DFL_BINS: usize = 16;
    /// 分类分支是否输出未激活 logits，默认 false（图中已 sigmoid）
    const CLS_IS_LOGITS: bool = false;
}

/// 三个泛型参数：Spec 元信息 / Decoder 策略 / Session 推理后端
pub struct GenericDetector<
    S: YoloSpec,
    D: YoloDecoder = Yolov8SpecDecoder<S>,
    Sess: NpuSession = RuntimeSession,
> {
    pub session: Sess,
    pub transform: HwLetterbox,
    pub decoder: D,
    pub config: StandardYoloConfig,
    /// 配置覆盖标签。用 `Option<String>` 托管，杜绝 `'static` 内存泄漏
    pub custom_label: Option<String>,
    /// 预处理/推理连续失败追踪（跨平台，纯计数，不推送业务告警）
    pub failure_tracker: FailureTracker,
    _spec: PhantomData<S>,
}

/// 标准开箱即用 YOLOv8/v11 检测器类型别名
pub type GenericYoloDetector<S> = GenericDetector<S, Yolov8SpecDecoder<S>, RuntimeSession>;
```

`AlgoPlugin::process` 的闭环与**计数边界**：

```rust
fn process(&mut self, frame: SafeFrame<'_>, emitter: &mut ResultEmitter<'_>) -> Result<(), AlgoError> {
    // 仅对硬件段（预处理 + 推理）计成败：FailureTracker 表征硬件降级状态。
    // 解码与结果发射属业务侧，其失败（如宿主回调断开）不得计入硬件失败，
    // 否则会累积到阈值并打出伪造的「硬件恢复正常」日志。
    let boxes = match self.infer(frame) {
        Ok(boxes) => { self.failure_tracker.record_success(); boxes }
        Err(error) => { self.failure_tracker.record_failure(); return Err(error); }
    };

    if !boxes.is_empty() {
        emitter.emit_detections_with_label(&boxes, self.custom_label.as_deref())?;
    }
    Ok(())
}

/// 硬件段：Transform 预处理 + NpuSession 推理，产出解码后的候选框
fn infer(&mut self, frame: SafeFrame<'_>) -> Result<Vec<NormBox>, AlgoError> {
    let (buf, mode) = self.transform.apply(&frame)?;
    let orig_w = frame.width();
    let orig_h = frame.height();

    self.session.infer_with(&buf, |net_out| {
        let ctx = YoloDecodeContext {
            output: net_out,
            conf_threshold: self.config.confidence_threshold,
            iou_threshold: self.config.iou_threshold,
            mode: &mode,
            orig_w,
            orig_h,
        };
        self.decoder.decode(&ctx)
    })
}
```

**模型路径解析**：`env.resolve_model_path(ctx.package_root, "MODEL_PATH", S::MODEL_PATH)?` —— 优先包私有 `.env`，否则用 `YoloSpec::MODEL_PATH`，实现包间零全局污染。

### 2.5 异构推理会话抽象 (`algo_sdk::runtime`)

```rust
/// 异构推理引擎的原始输出统一视图
pub enum InferenceOutput<'a> {
    /// 9 张量/多分支 INT8 结构化特征图输出（如 Rockchip RKNN）
    MultiBranch(Vec<RknnTensorOutput<'a>>),
    /// 单通道平铺浮点张量输出（如 Ascend / CoreML / 调试回退模拟路径）
    SingleFloat(&'a [f32]),
}

pub trait NpuSession: Send + 'static {
    /// 将经 HAL 处理后的显存容器送入推理引擎，
    /// 并在输出张量借用生命周期内安全执行后处理闭包 f
    fn infer_with<F, R>(&mut self, input: &CvBuffer, f: F) -> Result<R, AlgoError>
    where
        F: FnOnce(&InferenceOutput<'_>) -> Result<R, AlgoError>;
}

pub enum RuntimeSession {
    #[cfg(feature = "rknn")]
    Rockchip(Box<platforms::rockchip::RknnSession>),
    Fallback(CpuFallbackSession),
}
```

**设计取舍**：单一 `infer_with(input, closure)` 取代设计期草案的 `infer_with_dma_buf` + `infer_with_host_bytes` 双入口。`CvBuffer` 内部分支（DMA-BUF / CVPixelBuffer / Ascend device ptr / Host bytes）由 session 自行判定，调用方不再承担分支责任，也避免了「哪条路径才是零拷贝」的判断泄漏到业务层。

**包名别名**：`pub use runtime::platforms::rockchip as rknn;` 保留旧路径兼容；`crate::platforms::rknn` 亦可用。

## 3. 级联与时序组合机制 (Cascade & Temporal Composition)

### 3.1 现状：生产实践已存在，SDK 抽象未落地

级联编排**已在生产算法包中运行**，但以包内私有实现存在，未抽象为 SDK 契约：

| 包 | plugin.rs 行数 | 级联形态 |
|---|---|---|
| `algo-packages/rknn/rk3568/face_recognition` | 818 | 人脸+人体双检测 → `cv::crop_rgb` 抠图 → 五点对齐 → 特征提取 → `ByteTracker` |
| `algo-packages/rknn/rk3588/face_recognition` | 818 | 同上；额外复用**一份 640×384 RGA 输出**分别绑定两个 RKNN session |

SDK 已提供、可直接用于级联的组件：

- `CvEngine::crop_rgb` / `cv::crop_rgb` — 设备侧 ROI 抠图，Rockchip 路径返回 DMA-BUF
- `face::align` — Umeyama 相似变换与五点对齐
- `track::ByteTracker` — 纯 Rust 卡尔曼滤波 + 匈牙利最优二分图匹配
- `NpuSession` — 可作为级联第二阶段的推理入口

### 3.2 机制要点

1. **设备侧中间流转**：阶段 2 的输入 `CvBuffer` 由 `cv::crop_rgb` 在 RGA 中直接产出并返回 DMA-BUF，中间不经 CPU 像素拷贝；产出的 `CvBuffer` 可直接作为第二个 `NpuSession::infer_with` 的输入。
2. **状态机常驻**：`ByteTracker`、滑动窗口时序确认作为流水线结构体字段跨帧更新状态（`AlgoPlugin` 实例串行调用，内部状态只在其所属线程访问）。
3. **单路输出复用**：`rk3588` 的「一份 RGA 输出 → 两个 RKNN session」是已验证的优化范式，避免为每个模型重复 Letterbox。

### 3.3 未交付部分（交接给后续任务）

- **`CascadePipeline` 抽象**：设计期草案中的 `CascadePipeline { Det, Align, Ext, Trk }` 未实现。当前范式是「范式 B：包内手工编排」。
- **`CvBuffer` 池化管理**：SDK 层**无**公共 `CvBuffer` 池。`CvBufferKind` 为 `pub(crate)` 密封，池是 `RgaCvEngine` 内部私有的 `RgaBufferPool`。设计期承诺的「Ring/Pool Buffer 杜绝高频 malloc」在 SDK 层不成立。
  - ⚠️ 池上限 `MAX_CACHED_POOLS = 64`（`cv/platforms/rockchip/engine.rs:27`）；`rk3568`/`rk3588` face_recognition 的注释仍写 `(16)`，且「档位只增不减」的论证建立在 16 这个预算上，常量上调后该论证前提已失效，需复核。
- **DMA-to-DMA 中间流转的正确性测试**：无级联集成测试，无「高频生成 Chip 图无内存泄漏、无多余 CPU 拷贝」的自动化验证。

## 4. 跨平台扩展接口预留 (Ascend / NVIDIA SPI Hooks)

### 4.1 已交付的扩展点

| 扩展点 | 位置 | 状态 |
|---|---|---|
| `CvBufferKind::AscendDeviceMemory` | `cv/buffer.rs` | ✅ 已有变体 + `from_ascend_device_memory[_with_release]` 构造 |
| `AV_OPAQUE_ASCEND_DEVICE_MEMORY` | `c_abi.rs` (`0x3001`) | ✅ ABI 常量已定义，`FrameHandleView` 已识别 |
| `CvEngine` trait | `cv/engine.rs` | ✅ `letterbox` / `resize` / `crop_rgb` + `release_hardware` |
| `NpuSession` trait | `runtime/mod.rs` | ✅ 单一 `infer_with` 入口 |
| `InferenceOutput::SingleFloat` | `runtime/mod.rs` | ✅ 为 Ascend / CoreML 单张量输出预留 |
| `CvBufferKind::CudaDeviceMemory` | — | ❌ 设计期草案提到，代码中**不存在** |
| `AscendVpcEngine` / `CudaCvEngine` | — | ❌ 0 行代码 |

### 4.2 接入新平台时需要遵守的约束

1. **对齐规则在底座抚平**：Rockchip 16B 行跨距、Ascend DVPP 输入 16×2 宽高对齐 / VPC 输出 128×16 跨距，全部在 `CvEngine` 实现内部消化，不向业务代码泄露硬件怪癖。
2. **色彩空间差异由 session 承担**：Rockchip 路径输出 RGB888；Ascend 预期 NV12 + NPU AIPP 硬件转换。该差异收敛在 `CvEngine` 输出格式与 `NpuSession` 输入绑定的组合中，不由 `GenericDetector` 感知。
3. **`core_mask` 已是 session 构造参数**：`RknnSessionOptions { core_mask }`，但 `RuntimeSession::open` 未暴露 options 透传口，声明式路径固定使用 `RKNN_NPU_CORE_AUTO`。分核策略的宿主协同设计见 `10-01-npu-core-allocation` 任务。

## 5. 质量门禁与安全性保证

- **Panic 隔离**：`export_algo!` 导出的 C ABI 入口以 `catch_unwind` 包裹，Panic 不跨 FFI 边界传播。
- **RAII 生命周期**：`CvBuffer` 持有 `close_fd` / `release` 回调 / `_guard: Box<dyn Any + Send>`；`DefaultEngineLease` 管理进程级引擎回收。
- **`unsafe` 边界集中**：裸指针与平台 C 类型仅存在于 `cv/platforms/*`、`runtime/platforms/*`、`c_abi.rs`，不逃逸到安全层。
- **ABI 布局冻结**：`AvAlgoAbi` 与 `AvFrameDesc`（120 字节）由 `tests/c_abi_layout_tests.rs` 双侧断言守护。
- **无驱动可运行**：`RuntimeSession::open` 经 `open_or_fallback`，无 `librknnrt.so` 时退回 `CpuFallbackSession` 保证开发机与 CI 可跑通全链路。

> ⚠️ **已知风险（未在本任务范围内解决）**：`open_or_fallback` 的降级条件比 `debug_cpu_fallback_path` 的定义更宽——只要 librknnrt **加载失败**（含 `package_root` 拼写错误、ABI 不匹配）即静默降级并返回固定模拟检测框；`is_fallback()` 为 `pub` API，但 `crates/infer` 侧无任何消费者，生产环境无法观测该降级；`RknnSessionOptions` 亦无「禁止降级」开关。该风险已登记为独立交接项。

## 6. 生产范式对照 (Pattern A vs Pattern B)

| 维度 | 范式 A：声明式 | 范式 B：手工编排 |
|---|---|---|
| 适用 | 标准 YOLOv8/v11 单模型检测 | 多模型级联、状态机、特征比对 |
| 交付成本 | `impl YoloSpec`（`safetyhelmet_detection` 实测 11 行含注释） | 数百行 |
| 已迁移包 | `rk3568/safetyhelmet_detection`（1 个） | 其余 7 个包 |
| 硬件预处理 | `HwLetterbox` + `active_engine()` 自动分派 | 自行调用 `cv_engine.letterbox` |
| 失败追踪 | 内置 `FailureTracker` | 需自行接入 |
| 标签覆盖 | `Option<String>` 无泄漏 | `fire-detections` 仍用 `Box::leak` |

## 7. 设计期草案与落地差异清单 (Drift Record)

保留审计线索：设计期草案的代码片段与最终实现存在系统性漂移，以下为逐项对照。

| # | 设计期草案 | 实际落地 | 原因 |
|---|---|---|---|
| 1 | `crate::cv::models::yolo` | `crate::models::yolo`（顶层模块） | 模块正交归位：`cv` 回归纯 2D 预处理层 |
| 2 | `algo_sdk::rknn::RknnSession` | `runtime::platforms::rockchip::RknnSession` + `as rknn` 别名 | 统一异构运行时命名空间 |
| 3 | `infer_with_dma_buf` + `infer_with_host_bytes` | 单一 `NpuSession::infer_with(&CvBuffer, closure)` | 分支判定下沉至 session |
| 4 | `UnifiedTensorOutput` | `InferenceOutput::{MultiBranch, SingleFloat}` | 命名收敛 |
| 5 | `Yolov8StandardDecoder<const USE_SCORE_SUM, const DFL_BINS>` | `Yolov8SpecDecoder<S>` | 常量改挂 `YoloSpec`，免除 const 泛型 |
| 6 | `YoloDecoder::decode(outputs, conf, iou, mode, w, h)` | `decode(&YoloDecodeContext<'_>)` | 7 个散参收敛为上下文结构 |
| 7 | `GenericDetector<S, D>` | `GenericDetector<S, D, Sess>` | 支持宿主注入自定义 `NpuSession` |
| 8 | `init` 无标签字段 | 新增 `custom_label: Option<String>` | 消除 `'static` 标签泄漏 |
| 9 | 无失败追踪 | 新增 `FailureTracker`（硬件段计数） | 硬件降级可观测 |
| 10 | 全文未提 `CLS_IS_LOGITS` | 新增 `YoloSpec::CLS_IS_LOGITS` + `ClassActivation` | 模型导出图语义判定（§2.3） |
| 11 | `HwCrop` / `HwAffineWarp5Points` 列于 `cv::transforms` | 未实现；`crop_rgb` 在 `CvEngine`，仿射在 `face::align` | 见 §3.3 交接项 |
| 12 | `CvBufferKind::CudaDeviceMemory` | 不存在 | 平台适配未启动 |
| 13 | `CascadePipeline { Det, Align, Ext, Trk }` | 未实现（范式 B 手工编排） | 见 §3.3 交接项 |
| 14 | `InferenceSession` 返回 `UnifiedTensorOutputs` | `NpuSession`（`model.rs` 的 `InferenceSession` 是另一套既有抽象） | 命名冲突需注意 |
