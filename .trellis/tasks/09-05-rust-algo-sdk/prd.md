# PRD: Heimdall 独立 Rust 算法包开发套件 (algo-sdk)

## 1. 目标与核心定位

为 **Argus 边缘智能多媒体分析生态** 打造完全独立、轻量、高能效的纯 Rust 算法包开发套件（`crates/algo-sdk`）。

### 1.1 问题陈述

当前 `crates/infer` 的宿主侧已完整实现 C ABI 加载（`LoadedLib`）、七步沙箱自检（`AlgoSandbox`）、实例生命周期管理（`AlgoInstance`）和推理调度（`AlgoRegistry`）。但**算法包开发者侧**缺乏配套 SDK，算法工程师在自己的仓库中需要手动：

1. **重新定义 C ABI 结构体布局** — 必须与宿主 `infer/src/c_abi/types.rs` 严格 bit-for-bit 一致（152 字节 `AvFrameDesc`、96 字节 `AvAlgoAbi` 等），一旦漂移即为 UB
2. **手写 `#[no_mangle] extern "C"` 虚函数表** — 逐个实现 `library_open`/`query`/`close`、`instance_create`/`negotiate`/`process`/`flush`/`destroy`/`last_error` 共 11 个回调
3. **手写 `catch_unwind` Panic 防护** — 每个 C 入口必须包裹，遗漏一个就是宿主进程级崩溃
4. **自行解包 `AvFrameDesc` 裸指针** — 需要在 `unsafe` 中手动判断 `opaque_kind`、提取 DMA-BUF fd / CVPixelBuffer / Host 内存
5. **自行实现平台预处理分派** — Letterbox / Resize 等硬件加速调用完全不可复用
6. **自行实现 NMS / IoU / 坐标反算** — 每个算法包各写一份
7. **自行构造符合宿主规则引擎的 JSON 和 `AvAlgoImageReq` 抓拍请求** — 格式不对则规则引擎静默丢弃

### 1.2 核心使命

**一行 `export_algo!(YourPlugin)` + 一套平台无关的统一 API 消灭以上全部重复劳动。**

算法工程师面对的代码在任何芯片上完全一模一样：

```rust
fn process(&mut self, frame: SafeFrame<'_>, emitter: &mut ResultEmitter<'_>) -> Result<(), AlgoError> {
    let (buf, mode) = cv::letterbox(&frame, 640, 640, [114, 114, 114])?;
    let outputs      = self.session.infer(&buf)?;
    let boxes         = yolo_nms(&outputs, 0.45, 0.5);
    let unmapped      = unmap_boxes(&boxes, &mode, frame.width(), frame.height());
    emitter.emit_detections(&unmapped)?;
    Ok(())
}
```

### 1.3 核心价值支柱

1. **C ABI 类型单一真实来源** — `algo-sdk` 包含权威的 C ABI 结构体定义，`infer` 改为 re-export/依赖同一套定义，从根源杜绝两侧布局漂移
2. **硬件抽象层 (HAL) 与驱动 SPI (`CvEngine`)** — 统一 `CvBuffer` 不透明显存句柄 + RAII 自动回收；`cv::letterbox` / `cv::resize` 一次调用，底层自动分派到 RGA / vImage / DVPP / CPU
3. **共享模型权重 + 独立推理会话 (`model`)** — `SharedWeights<W>` 物理显存只占 1 份，`weights.session()` 自动轮询多 NPU 核心
4. **向量化后处理 (`fast_math`)** — 纯 Rust IoU / NMS / 坐标反算，消灭跨算法包的代码重复
5. **结果发射器 (`ResultEmitter`)** — 一键 `emit_detections`，自动格式化为宿主可消费的告警 JSON + 全景大图抓拍请求
6. **一行声明宏 (`export_algo!`)** — 自动展开 C ABI 虚函数表 + Panic 隔离 + 错误缓存
7. **绝对独立自包含** — `algo-sdk` 零依赖 Argus 业务 crate，算法工程师在独立仓库中 `cargo test` 即可闭环

---

## 2. 功能模块划分

### 2.1 C ABI 规范映射 (`c_abi`)

- 1:1 严格对齐 Argus C ABI 布局（64 位平台 8 字节对齐）
- 核心结构体：`AvAlgoAbi`, `AvFrameDesc`, `AvAlgoInstanceArgs`, `AvAlgoResult`, `AvAlgoImageReq`, `AvImageOps`, `AvFrameCaps`, `AvRule` 等
- **归属决策**：`algo-sdk/src/c_abi.rs` 为权威定义。宿主 `infer` 侧当前的 `c_abi/types.rs` 保持不动（本期不强制迁移），但两侧必须通过 CI `size_of`/`offset_of` 断言测试保证零漂移。未来可抽为独立 `algo-abi` crate 作为单一来源。

### 2.2 错误体系 (`error`)

- `AlgoError` 枚举 — 覆盖配置解析、模型加载、推理执行、预处理、I/O 等场景
- 与 C ABI 错误码 (`AV_OK`, `AV_ERR_*`) 的双向映射

### 2.3 安全帧视图 (`frame`)

- `SafeFrame<'a>` — 从 `AvFrameDesc` 裸指针构造的强类型只读借用视图（生命周期绑定到 C 回调栈帧）
- `FrameHandleView<'a>` — 精准解包原生硬件句柄：
  - `DmaBuf { fd: i32 }` (Rockchip RGA / RKNN)
  - `ApplePixelBuffer(ptr)` (Apple Silicon)
  - `AscendDeviceMemory(ptr)` (Huawei Ascend DVPP)
  - `Host(bytes)` (开发/测试)
- 暴露宽高、格式、时间戳、对齐跨度

### 2.4 视觉加速硬件抽象层 (`cv` - HAL & SPI)

- **`CvBuffer`** — 不透明显存句柄容器，RAII 守卫。封装 DMA-BUF fd / CVPixelBuffer / DeviceMemory / Host 内存
- **`PreprocessMode`** — `Letterbox(LetterboxLayout)` | `Resize`，用于坐标反算对齐
- **`CvEngine` Trait (SPI)** — `letterbox()` / `resize()` 标准接口
- **统一门面函数** — `cv::letterbox()` / `cv::resize()` 自动委派到当前环境最佳引擎
- **内置驱动**：
  - `CpuCvEngine` — 纯 Rust 双线性软缩放保底，确保 `cargo test` 通过
  - `HostCvEngine` — 委托宿主注入的 `AvImageOps` 虚表
  - `AppleCvEngine` — 基于 Apple Accelerate vImage / CoreVideo CVPixelBuffer 硬件加速驱动（`#[cfg(target_os = "macos")]`）
  - `RgaCvEngine` — Rockchip RGA 2D 硬件 DMA（后续扩展）

### 2.5 共享模型与核心调度 (`model`)

- **`Core`** 枚举 — `Auto`（Round-Robin）, `Id(u8)`, `All`, `Mask(u32)`
- **`ModelWeights` Trait** — `fn session_on(&self, core: Core) -> Result<Self::Session, AlgoError>`
- **`InferenceSession` Trait** — `fn infer(&mut self, input: &CvBuffer) -> Result<Self::Output, AlgoError>`
- **`SharedWeights<W>`** — `Arc<W>` + `AtomicUsize` Round-Robin 计数器
  - `weights.session()` — 自动轮询
  - `weights.session_on(Core::Id(0))` — 显式绑核

### 2.6 向量化后处理 (`fast_math`)

- `calculate_iou(a, b)` — 快速交并比
- `fast_nms(boxes, iou_threshold)` — 高性能类别感知 NMS
- `fast_nms_agnostic(boxes, iou_threshold)` — 类别无关 NMS
- `unmap_box(box, mode, src_w, src_h)` — Letterbox 扣黑边 / Resize 线性归一化，双模式自适应
- `clamp_bbox(bbox)` — 坐标截断到 `[0.0, 1.0]`

### 2.7 结果发射器 (`emitter`)

- `DetectionBox` — 归一化检测框 + 类别 + 置信度
- `ResultEmitter<'a>` — 持有 C 回调函数指针的安全包装
  - `emit_detections(&[DetectionBox])` — 自动序列化为 Argus 标准告警 JSON + 全景大图抓拍请求
  - `emit_self_test(count)` — 自检模式合格报告

### 2.8 插件 Trait 与导出宏 (`plugin` & `macros`)

- `AlgoPlugin` Trait — `init` / `process` / `flush` / `set_rules`
- `export_algo!` 宏 — 生成 C ABI 虚函数表 + `av_algo_get_abi` 符号 + `catch_unwind` 隔离 + `last_error` 缓存

### 2.9 测试脚手架 (`testing`)

- **`MockFrameBuilder`**：
  - 基础模式：从内存切片构建 `SafeFrame`；
  - **图片直转硬件加速格式 (feature = `testing-image` / `testing-hardware`)**：
    - 直接从本地 `.jpg` / `.png` 文件解码并自动完成色彩空间与步长转换（RGB24 -> 平台特定 Stride 对齐的 NV12 格式，如 16/64 字节步长对齐）；
    - 自动/按需构造底层真实硬件加速显存句柄：
      - macOS 环境：分配 CoreVideo `CVPixelBuffer`，打上 `AV_OPAQUE_CVPIXELBUFFER` 标签；
      - Linux 环境：通过 DMA-Heap / `memfd` 构造 `dma_buf` 文件描述符 `fd`，打上 `AV_OPAQUE_DMABUF` 标签；
    - 确保算法工程师在独立仓库单测中，能 100% 真实覆盖零拷贝快速推理通路（`FrameHandleView::DmaBuf` / `ApplePixelBuffer`）与硬件步长对齐边界，彻底杜绝“单测走软解、上机绿屏崩”的测试漂移。
- **`MockEmitter`**：捕获并校验算法包输出。
- **`MockWeights` / `MockSession`**：纯 CPU 推理桩。

---

## 3. C ABI 类型归属策略

这是本任务最关键的架构决策。

### 3.1 约束分析

| 约束 | 来源 |
|---|---|
| `algo-sdk` 必须零依赖主工程业务 crate | PRD §1.3.7 |
| 两侧 C ABI 结构体必须 bit-for-bit 一致 | FFI spec + UB 规则 |
| 宿主 `infer` 已有完整类型定义 + 测试 | `infer/src/c_abi/types.rs` + `tests/c_abi_layout_tests.rs` |

### 3.2 本期方案：双副本 + CI 断言对齐

1. `algo-sdk/src/c_abi.rs` 内含完整的 C ABI 类型定义（作为算法开发者侧的权威来源）
2. `infer/src/c_abi/types.rs` 保持不动（宿主侧现有代码零改动）
3. 两侧各自包含 `size_of` + `offset_of` 静态断言测试，值必须完全相同
4. CI 中 `cargo test --workspace` 同时覆盖两侧断言，任一漂移即红灯

### 3.3 演进方向

后续可抽取为独立的 `crates/algo-abi` (`no_std`, 零依赖)，两侧统一 re-export。本期不执行此重构，避免改动 `infer` 现有代码。

---

## 4. 与现有代码的边界关系

### 4.1 与 `infer` 的关系

| 维度 | `infer`（宿主侧） | `algo-sdk`（算法开发者侧） |
|---|---|---|
| 角色 | 加载 `.so/.dylib`、沙箱校验、C ABI 调用方 | 提供 C ABI 实现的脚手架 |
| C ABI 类型 | 消费者（构造 `AvFrameDesc` 传入） | 消费者（接收 `AvFrameDesc` 解包） |
| 依赖关系 | **不依赖** `algo-sdk` | **不依赖** `infer` |
| 集成验证 | `infer/tests/` 加载 `algo-sdk` 编译产物做端到端测试 | `algo-sdk/tests/` 独立运行 |

### 4.2 与 `types::FrameRef` 的关系

- `FrameRef` + `FrameHandle` 是**宿主管线内**的帧流转单元（media → pipeline → infer）
- `SafeFrame<'a>` 是 SDK 侧从 `AvFrameDesc` 构造的**只读借用视图**，生命周期绑定 C 回调栈帧
- `CvBuffer` 是 SDK 侧 HAL 产出的**预处理后显存句柄**
- 三者层级清晰，不交叉：`AvFrameDesc → SafeFrame → cv::letterbox → CvBuffer → session.infer(&buf)`

### 4.3 与 `InferenceBackend` 的关系

- 宿主侧 `InferenceBackend::detect(&self, frame: &FrameRef)` 面向 pipeline 层
- SDK 侧 `InferenceSession::infer(&mut self, input: &CvBuffer)` 面向算法包内部
- 宿主通过 C ABI 调 `instance_process` → SDK 内部走 `AlgoPlugin::process → cv::letterbox → session.infer`

---

## 5. 验收标准

### 5.1 结构与依赖
- [ ] `crates/algo-sdk` 成功创建，在 workspace `Cargo.toml` 注册
- [ ] Cargo 依赖纯净：仅 `serde`, `serde_json`, `libc`, `static_assertions`，零主工程业务 crate 依赖

### 5.2 C ABI 与帧
- [ ] C ABI 类型定义完整，`size_of` + `offset_of` 静态断言与 `infer` 侧完全一致
- [ ] `SafeFrame` 成功从 `AvFrameDesc` 构造，正确解包 4 种硬件句柄

### 5.3 HAL & SPI
- [ ] `CvBuffer` RAII 生命周期正确（Drop 测试覆盖）
- [ ] `CvEngine` Trait 定义完整
- [ ] `CpuCvEngine` 通过 `cargo test`，产出正确的 `PreprocessMode`
- [ ] `cv::letterbox` / `cv::resize` 门面函数可调用

### 5.4 Model
- [ ] `SharedWeights<W>` 支持 `session()` (Round-Robin) 与 `session_on(Core::Id(n))`
- [ ] 不同核心数下的分配行为有单元测试覆盖

### 5.5 后处理
- [ ] `fast_nms` 对极端重叠、空输入、单框等边界条件有测试覆盖
- [ ] `unmap_box` 对 Letterbox 与 Resize 双模式的坐标反算精度有测试覆盖

### 5.6 发射器
- [ ] `ResultEmitter` 产出的 JSON 能被 `infer/src/package.rs::parse_alarm_objects` 正确解析
- [ ] 全景大图抓拍请求 `AvAlgoImageReq { x:0, y:0, w:1, h:1 }` 自动挂载

### 5.7 宏与安全
- [ ] `export_algo!` 展开后导出唯一符号 `av_algo_get_abi`
- [ ] Panic 隔离测试：模拟 panic 后返回 `AV_ERR_INTERNAL`，宿主不崩溃
- [ ] 所有 `unsafe` 块均有 `// SAFETY:` 注释

### 5.8 集成与测试脚手架
- [ ] `algo-sdk/tests/` 中 `MockRustDetector` 完成完整生命周期验证
- [ ] 测试脚手架支持通过 feature 加载真实图片并构造硬件加速帧（NV12 + Stride 步长对齐 + 平台原生句柄 CVPixelBuffer/DMA-BUF），能被算法包解包消费
- [ ] `infer/tests/` 中集成测试能加载 `algo-sdk` 编译的纯 Rust 动态库（可标记 `#[ignore]`）

### 5.9 门禁
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
