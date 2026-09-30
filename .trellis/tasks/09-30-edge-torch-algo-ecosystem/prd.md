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
  - `HwLetterbox`: 保持比例等比缩放并在四周补齐底色，返回 `(CvBuffer, PreprocessMode)`；
  - `HwCrop`: ROI 局部硬件切片裁切；
  - `HwAffineWarp5Points`: 针对人脸等五点相似变换的硬件加速仿射变换；
- 底层根据编译平台或运行期环境自动路由至 `RgaCvEngine`（Rockchip）、`AscendVpcEngine`（华为昇腾）、`CudaCvEngine`（NVIDIA）或 `CpuCvEngine`。

### 3.2 异构推理会话统一抽象 (`algo_sdk::backend::NpuSession`)
- 提供统一的推理会话模型：`NpuSession`。
- 封装物理显存输入绑定：
  - 自动识别输入 `CvBuffer` 的底层形态（DMA-BUF fd、Ascend DevicePtr、CUDA 指针）；
  - 屏蔽芯片间色彩空间转换与跨步对齐差异（如 Rockchip RGA 输出 RGB888 vs 昇腾 VPC 输出 128×16 对齐 NV12 并依托 NPU AIPP 硬件转换）；
- 提供统一张量输出视图 `UnifiedTensorOutput`，支持自动分发：
  - `Int8 { data, zp, scale }`（Rockchip RKNN 分支）；
  - `Fp16 / Fp32`（Ascend DaVinci / NVIDIA TensorRT 分支）。

### 3.3 通用检测器模板 (`algo_sdk::cv::models::yolo`)
- 定义元配置 Trait `YoloSpec`（包含输入尺寸、类别数、类别标签列表、模型文件名等）；
- 提供开箱即用的通用结构体 `GenericYoloDetector<Spec>`，自动实现 `AlgoPlugin`；
- 内部完整闭环：配置注入 -> 硬件预处理 -> NPU 推理 -> INT8/FP16 DFL 解码 -> Fast-NMS -> 坐标反算 -> 结果发射；
- 让普通单模型检测包代码量收敛至 ~15 行。

### 3.4 解码器策略解耦与魔改 YOLO 适配支持 (Decoupled YoloDecoder Strategy)
- 工业级定制模型往往包含魔改头（如 4 尺度 P2 微小目标层、Anchor 先验框机制、单张量融合输出 `[1, 84, 8400]` 等）；
- 将输出张量解析抽象为 `YoloDecoder` Trait（策略模式）：
  - 官方标准模型：默认使用 `Yolov8StandardDecoder`，零配置享受 15 行极简交付；
  - 非标/魔改模型：开发者只需自定义实现 `YoloDecoder`（负责定制分支反量化与解析），其余 2D 硬件预处理、DMA 映射、配置与 C ABI 导出仍 100% 自动继承。

### 3.5 模块化级联多模型流水线支持 (Composable Cascade Pipeline)
- 规范化级联模型的中间显存流通契约：
  - 模型 1 检测出的 BoundingBox / 关键点，可直接送入 `Transform` 进行硬件级零拷贝抠图/仿射（DMA-to-DMA）；
  - 中间图像 `CvBuffer` 依托池化管理（Ring/Pool Buffer），杜绝高频 `malloc` / `mmap` / `cudaMalloc`；
  - 产出的中间显存块直接作为模型 2 的输入驱动二次推理。
- 提供时序状态机接口（如 `ByteTracker`、多帧平滑确认 `TemporalVerifier`）的标准挂载范式。

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

- [x] **M1: 核心 Transform 与 GenericYoloDetector 抽象落地**
  - 在 `crates/algo-sdk` 中实现 `algo_sdk::cv::models::yolo::GenericYoloDetector` 及配套 `Transform`；
  - 编写覆盖全生命周期的单元测试（包含 CPU Fallback 模式模拟推理与后处理验证）。
- [ ] **M2: 现有检测算法包无感迁移验证**
  - 将 `algo-packages/rknn/rk3568/safetyhelmet_detection` 重构为基于 `GenericYoloDetector` 的极简实现（源码行数压至 30 行以内）；
  - 运行并通过原算法包的全部集成测试与 Clippy 严格检查（`cargo test`、`cargo clippy`）。
- [ ] **M3: 级联多模型与时序拼装范式验证**
  - 提供标准的级联测试用例（如检测 -> 抠图 -> 分类 -> 时序确认），验证 DMA-to-DMA 中间显存流转正确性；
  - 验证在中间高频生成 Chip 图像时无内存泄漏、无多余 CPU 拷贝。
- [x] **M4: 跨平台架构钩子与规范文档更新**
  - 在 `crates/algo-sdk/GUIDE.md` 和 `.trellis/spec/algo-sdk/` 中正式收录该生态设计规范；
  - 确保四平台（`macos`, `rk3568`, `rk3576`, `rk3588`）及宿主根工作区 CI 门禁全绿通过。
