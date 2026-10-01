# PRD: PyTorch-like Composable Edge AI Algorithm Ecosystem

## 1. 业务目标与愿景 (Goal & Vision)

在 `crates/algo-sdk` 中构建一个类似 **PyTorch / TorchVision** 风格的端侧模块化、可组合视觉算法开发生态。

通过统一抽象“硬件变换算子（Transforms）”、“异构推理会话（NpuSession / Module）”与“时序状态机（Trackers / StateMachines）”，实现：
1. **常规目标检测极简交付**：基于 `GenericYoloDetector<Spec>` 模板，仅需 ~15 行声明式代码即可完成一个符合工业级规范的算法包；
2. **级联多模型与时序算法自由拼装**：支持像写 PyTorch `forward()` 一样自由拼接多阶段模型（如人脸检测 -> 5点仿射变换 -> 特征提取 -> 航迹跟踪），同时在底层保持严格的**物理显存/DMA-BUF 零拷贝（DMA-to-DMA）**；
3. **跨芯片生态可扩展性**：算法业务层对底层硬件解耦，同一套业务拓扑可无缝平推至 Rockchip（RKNN/RGA）、华为昇腾（Ascend CANN/DVPP/VPC）、NVIDIA（TensorRT/CUDA）与 Apple（CoreML）。

---

## 2. 背景与现状 (Context & Problem Statement)

在之前的两轮重构中，我们已成功：
- 沉淀了统一的 `algo_sdk::rknn::RknnSession`，消除重复驱动代码并修复了潜藏的 Double-Free 隐患；
- 引入了 `algo_config!` 声明式宏，彻底消除了显式字段跟踪与三级配置优先级（Host > `.env` > Default）的样板代码；
- 引入了 `LocalPluginRunner` 与 `scripts/algo-new.sh` 脚手架，累计消除 3,421 行代码。

**当前仍存在的痛点与演进缺口**：
1. **算法包内部仍存在重复的“三段式”样板流程**：每个 YOLO 检测包仍需在 `plugin.rs` 中重复编写 `rga.letterbox -> session.infer_with_dma_buf -> parse_yolov8_int8 -> emit` 样板逻辑；
2. **级联多模型缺乏标准拼接骨架**：现存人脸包（`face_recognition`）逻辑高度定制，多模型间的中间显存传递、5点仿射与状态机流转缺乏标准 Module 级组合抽象；
3. **硬件后端未彻底面向异构芯片抽象化**：目前 `algo_sdk::rknn` 偏向 Rockchip，尚未形成可平滑扩展到华为昇腾（Ascend ACL/DVPP）与 NVIDIA（TensorRT）的统一 `NpuSession` SPI。

---

## 3. 功能需求与模块规划 (Functional Requirements)

### 3.1 硬件预处理流水线 (`algo_sdk::cv::transforms`)
- 提供类似 `torchvision.transforms` 的可组合算子接口：`Transform` trait。
- 内置标准硬件算子：
  - `HwLetterbox`: 保持比例等比缩放并在四周补齐底色，返回 `(CvBuffer, PreprocessMode)`；✅ 已交付
  - `HwCrop`: ROI 局部硬件切片裁切；❌ **未交付**——能力经 `CvEngine::crop_rgb` / `cv::crop_rgb` 提供，尚未收敛为 `Transform` 算子（交接 H1）
  - `HwAffineWarp5Points`: 针对人脸等五点相似变换的硬件加速仿射变换；❌ **未交付**——仅 CPU 侧 `face::align` 提供（交接 H1）
- 底层根据编译平台或运行期环境自动路由至 `RgaCvEngine`（Rockchip）、`AscendVpcEngine`（华为昇腾）、`CudaCvEngine`（NVIDIA）或 `CpuCvEngine`。
  - 已交付引擎：`RgaCvEngine`、`AppleCvEngine`、`CpuCvEngine`、宿主 `AvImageOps` 代理；`AscendVpcEngine` 与 `CudaCvEngine` 为 ❌ **未开始**（交接 H2）。

### 3.2 异构推理会话统一抽象 (`algo_sdk::runtime::NpuSession`)

> 路径修正：设计期写作 `algo_sdk::backend::NpuSession`，落地为 `algo_sdk::runtime::NpuSession`。
- 提供统一的推理会话模型：`NpuSession`。
- 封装物理显存输入绑定：
  - 自动识别输入 `CvBuffer` 的底层形态（DMA-BUF fd、Ascend DevicePtr、CUDA 指针）；
  - 屏蔽芯片间色彩空间转换与跨步对齐差异（如 Rockchip RGA 输出 RGB888 vs 昇腾 VPC 输出 128×16 对齐 NV12 并依托 NPU AIPP 硬件转换）；
- 提供统一张量输出视图 `UnifiedTensorOutput`，支持自动分发：
  - `Int8 { data, zp, scale }`（Rockchip RKNN 分支）；
  - `Fp16 / Fp32`（Ascend DaVinci / NVIDIA TensorRT 分支）。

### 3.3 通用检测器模板 (`algo_sdk::models::yolo`)

> 路径修正：设计期写作 `algo_sdk::cv::models::yolo`，落地为顶层模块 `algo_sdk::models::yolo`。
- 定义元配置 Trait `YoloSpec`（包含输入尺寸、类别数、类别标签列表、模型文件名等）；
- 提供开箱即用的通用结构体 `GenericYoloDetector<Spec>`，自动实现 `AlgoPlugin`；
- 内部完整闭环：配置注入 -> 硬件预处理 -> NPU 推理 -> INT8/FP16 DFL 解码 -> Fast-NMS -> 坐标反算 -> 结果发射；
- 让普通单模型检测包代码量收敛至 ~15 行。

> **口径澄清**：「~15 行」是**声明式交付心智模型**的度量，指开发者需要手写的声明式代码量级；
> 与「文件总行数」不是同一度量。`safetyhelmet_detection` 实测：文件 24 行、`impl YoloSpec` 块 11 行（含注释）。
> 引用该指标时必须标注口径，不得混用。

### 3.4 解码器策略解耦与魔改 YOLO 适配支持 (Decoupled YoloDecoder Strategy)
- 工业级定制模型往往包含魔改头（如 4 尺度 P2 微小目标层、Anchor 先验框机制、单张量融合输出 `[1, 84, 8400]` 等）；
- 将输出张量解析抽象为 `YoloDecoder` Trait（策略模式）：
  - 官方标准模型：默认使用 `Yolov8StandardDecoder`，零配置享受 15 行极简交付；
  - 非标/魔改模型：开发者只需自定义实现 `YoloDecoder`（负责定制分支反量化与解析），其余 2D 硬件预处理、DMA 映射、配置与 C ABI 导出仍 100% 自动继承。

### 3.5 模块化级联多模型流水线支持 (Composable Cascade Pipeline)

> **状态：SDK 层未落地，移出本任务（交接 H1）。**
> 级联能力已在 `algo-packages/rknn/rk3568/face_recognition` 与 `rk3588/face_recognition`（各 818 行）中以**包内手工编排**形态生产运行，
> 含「一份 640×384 RGA 输出 → 两个 RKNN session」的 DMA-BUF 复用验证。
> 缺口在于 SDK 契约抽象与自动化验证，而非能力本身。

- 规范化级联模型的中间显存流通契约：
  - 模型 1 检测出的 BoundingBox / 关键点，可直接送入 `Transform` 进行硬件级零拷贝抠图/仿射（DMA-to-DMA）；
    - 现状：抠图经 `cv::crop_rgb`（Rockchip 路径返回 DMA-BUF），仿射经 CPU 侧 `face::align`，非 `Transform` 算子；
  - 中间图像 `CvBuffer` 依托池化管理（Ring/Pool Buffer），杜绝高频 `malloc` / `mmap` / `cudaMalloc`；
    - 现状：❌ SDK 层**无**公共 `CvBuffer` 池。`CvBufferKind` 为 `pub(crate)` 密封，池是 `RgaCvEngine` 私有 `RgaBufferPool`；
  - 产出的中间显存块直接作为模型 2 的输入驱动二次推理。
- 提供时序状态机接口（如 `ByteTracker`、多帧平滑确认 `TemporalVerifier`）的标准挂载范式。
  - 现状：`ByteTracker` 已在 `algo_sdk::track` 提供并被 3 个包复用；`TemporalVerifier` 仍为 `fire-detections` 包内私有实现。

---

## 4. 非功能性约束与工程红线 (Non-Functional Requirements)

1. **绝对零拷贝红线（Zero-Copy Fast Path）**：
   - 稳态常驻推理路径上，图像数据严禁落入 CPU 内存进行 `memcpy` 或色彩格式转换；
   - 跨阶段（模型 1 -> 裁切 -> 模型 2）必须维持物理显存（DMA-BUF / Device Memory）直接流转。
2. **严苛的硬件步长对齐（Stride Alignment）**：
   - Rockchip 平台保持 16 字节行跨距对齐；
   - 华为昇腾平台严格遵守 DVPP 输入 16×2 宽高对齐与 VPC 输出 128×16 跨距约束；
   - 所有的边界补齐计算全部在底座 `CvEngine` 内部抚平，不向上层业务代码泄露硬件怪癖。
3. **严格的 FFI 与内存生命周期安全**：
   - 裸指针与底层驱动句柄必须受 RAII 守护，禁止内存泄漏与 Double-Free；
   - 所有导出的 C ABI 入口点必须拥有 `catch_unwind` Panic 防火墙，绝不允许跨 FFI 边界传播 Panic。
4. **单二进制交付与向后兼容**：
   - 改造不得破坏现有 C ABI 接口定义（`AvAlgoAbi` 布局、`AvFrameDesc` 120 字节规范）；
   - 保证宿主应用程序（`crates/infer`）对既有算法包的平滑兼容。

---

## 5. 验收标准 (Acceptance Criteria)

- [~] **M1: 核心 Transform 与 GenericYoloDetector 抽象落地**
  - 在 `crates/algo-sdk` 中实现 `algo_sdk::models::yolo::GenericYoloDetector` 及配套 `Transform`；
  - 编写覆盖全生命周期的单元测试（包含 CPU Fallback 模式模拟推理与后处理验证）。
  - **交付范围收窄（2026-10-01 校准）**：仅 `HwLetterbox` 一个算子落地；`HwCrop` / `HwAffineWarp5Points` 未交付，
    已交接至 H1。原措辞「配套 `Transform`」含三算子，现据实标注为部分满足。
    - 验证：`cargo test -p algo-sdk --lib` → 116 passed（含 `test_generic_yolo_detector_lifecycle`、`test_custom_decoder_strategy`、`test_hw_letterbox_basic`）
- [x] **M2: 现有检测算法包无感迁移验证**
  - 将 `algo-packages/rknn/rk3568/safetyhelmet_detection` 重构为基于 `GenericYoloDetector` 的极简实现；
    验收口径在实现期间由「源码行数压至 30 行以内」改写为「plugin 核心收敛至 21 行」（实测口径，
    两者均满足：`impl YoloSpec for SafetyHelmetSpec` 块实测 21 行），在此保留改写记录以便审计；
  - 运行并通过原算法包的全部集成测试与 Clippy 严格检查（`cargo test`、`cargo clippy`）。
  - **行数权威复核口径（2026-10-01）**：上述「21 行」与当前源码不符（块总计 11 行，含 2 行文档注释；去注释与空行 9 行）。
    上句为历史记录不作修改，**复核请以 `implement.md` §2 Step 6 的表格为准**。另：「~15 行」是声明式交付量级口径，
    与文件行数不可混用。
  - **迁移覆盖度**：「无感迁移」仅适用于本包 1/8；其余 7 个包（`fire-detections`、3× `general_detection`、3× `face_recognition`）
    仍为手工编排，其中 `fire-detections` 仍使用 `Box::leak` 标签泄漏（SDK 已提供 `Option<String>` 方案）。
- [ ] **M3: 级联多模型与时序拼装范式验证** — **未达成，已移出本任务（交接 H1）**
  - 提供标准的级联测试用例（如检测 -> 抠图 -> 分类 -> 时序确认），验证 DMA-to-DMA 中间显存流转正确性；
  - 验证在中间高频生成 Chip 图像时无内存泄漏、无多余 CPU 拷贝。
  - **未达成证据**：全仓 `grep CascadePipeline` 0 命中；`crates/algo-sdk` 无级联集成测试；SDK 层无公共 `CvBuffer` 池。
  - **降级理由**：级联能力已在 2 个 `face_recognition` 包（各 818 行）中生产运行并含 DMA-BUF 复用验证，
    缺的是抽象与测试而非能力；`CvBufferKind` 为 `pub(crate)` 密封，池化上提需评估 ABI 与跨平台影响，属独立工程量。
  - **不得视为已交付**：移出本任务不等于完成，H1 未立项前该能力仅以包内私有实现存在，无契约保障。
- [x] **M4: 跨平台架构钩子与规范文档更新**
  - 在 `crates/algo-sdk/GUIDE.md` 和 `.trellis/spec/algo-sdk/` 中正式收录该生态设计规范；
  - 确保四平台（`macos`, `rk3568`, `rk3576`, `rk3588`）及宿主根工作区门禁全绿通过。
  - **校准（2026-10-01）**：本仓无 CI workflow（`.github/workflows` 不存在），「CI 门禁」实指本地命令全部通过；
    「跨平台架构钩子」中 Ascend / NVIDIA 部分为空壳（`AscendVpcEngine` 0 行代码、无 `CudaDeviceMemory`），
    仅为预留扩展点，不得理解为已具备跨平台能力（见 H2）。

---

## 6. 交接与风险登记 (Handoff & Risk Register)

变更日期：2026-10-01。本任务范围收窄为「已交付部分的文档校准与验收口径对齐」，
以下三项不属该范围，独立承接，**不挂为本任务的 subtask**（归档会移动父目录，子任务链接将悬空）。
详见 `implement.md` §4。

| 编号 | 事项 | 性质 | 状态 |
|---|---|---|---|
| H1 | 级联多模型 + `CvBuffer` 池化 SDK 抽象（原 M3 / Phase 8） | 未交付能力 | 未立项 |
| H2 | Ascend DVPP / NVIDIA TensorRT 平台适配（原 Phase 7） | 未启动，需真机 | 未立项 |
| H3 | CPU fallback 静默降级不可观测 | **生产风险** | 未立项，建议定级 P1 |

### H3 风险摘要

`RuntimeSession::open` → `RknnSession::open_or_fallback` 在 `librknnrt` **加载失败**时静默降级为
`CpuFallbackSession` 并返回固定模拟检测框；`is_fallback()` 为 `pub` API，但 `crates/infer` 侧无任何消费者；
`RknnSessionOptions` 亦无「禁止降级」开关。触发条件比 AGENTS.md 定义的 `debug_cpu_fallback_path`
（「仅在物理上确无硬件加速单元时」）更宽——`package_root` 拼写错误、ABI 不匹配同样会走到该分支，
使算法包「看起来正常运行」并稳定输出固定假框。

### 关联任务

- `10-01-npu-core-allocation`：分核策略与宿主协同。注意 `RknnSessionOptions::core_mask` 已是 session 构造参数，
  但 `RuntimeSession::open` 未暴露 options 透传口，声明式路径固定使用 `RKNN_NPU_CORE_AUTO`；该任务如需改动此接口，
  应与本任务确立的 `NpuSession` 契约对齐。
