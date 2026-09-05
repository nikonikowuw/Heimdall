# Implementation Plan: Heimdall 独立 Rust 算法包开发套件 (algo-sdk)

## API Contract Exemption Note
本任务为纯后端与插件 SDK 架构工程（`crates/algo-sdk`），不涉及外部 HTTP/WebSocket API 接口变更，故根据 Trellis 规范跳过 `api.md`。

---

## 关键前置决策

| 决策 | 选定方案 | 理由 |
|---|---|---|
| C ABI 类型归属 | 双副本 + CI 断言对齐 | 零改动现有 `infer` 代码，通过断言防漂移 |
| MVP 引擎范围 | `CpuCvEngine` + `HostCvEngine` | 保证 `cargo test` 全平台通过；硬件驱动后续 feature-gated |
| `fast_math` 优化策略 | 先标量正确性，后 NEON/AVX | 正确性优先，SIMD 路径按需追加 |
| 集成测试策略 | `algo-sdk/tests/` 内编译 `cdylib` + `infer/tests/` 加载 | 端到端验证 C ABI 兼容性 |

---

## Phase 1: 创建纯净 crate 骨架 + C ABI + 错误体系

### 步骤

- [x] 1.1 创建 `crates/algo-sdk/Cargo.toml`
  - 核心依赖：`serde`, `serde_json`, `libc`, `static_assertions`, `uuid`, `thiserror`
  - 可选依赖：`image = { version = "0.25", default-features = false, features = ["jpeg", "png"], optional = true }`
  - 特性支持：`testing-image = ["dep:image"]`, `testing-hardware = ["testing-image"]`
  - 零主工程业务 crate 依赖，生产 cdylib 默认 0 冗余体积
- [x] 1.2 在根 `Cargo.toml` 的 `workspace.members` 中添加 `"crates/algo-sdk"`
  - 在 `workspace.dependencies` 中添加 `algo-sdk = { path = "crates/algo-sdk" }`
- [x] 1.3 创建 `crates/algo-sdk/src/lib.rs` — 模块声明骨架
- [x] 1.4 实现 `crates/algo-sdk/src/c_abi.rs`
  - 从 `infer/src/c_abi/types.rs` 精确复制所有结构体、常量、函数指针类型
  - 在文件顶部添加注释说明"权威来源：本文件与 infer/src/c_abi/types.rs 必须 bit-for-bit 一致"
- [x] 1.5 创建 `crates/algo-sdk/tests/c_abi_layout_tests.rs`
  - 从 `infer/tests/c_abi_layout_tests.rs` 精确复制所有 `size_of` + `offset_of` 断言
  - 值必须完全一致，任一偏差即编译失败
- [x] 1.6 实现 `crates/algo-sdk/src/error.rs`
  - `AlgoError` 枚举：`ConfigParse`, `ModelLoad`, `Inference`, `Preprocess`, `IncompatibleFrame`, `OutOfMemory`, `Internal`
  - `to_c_status()` — 映射到 `AV_OK`, `AV_ERR_*` 常量
  - `from_c_status()` — 反向映射

### 验证

```bash
cargo fmt --all
cargo clippy -p algo-sdk --all-targets -- -D warnings
cargo test -p algo-sdk
```

---

## Phase 2: 安全帧视图 + CvBuffer HAL + CvEngine SPI

### 步骤

- [x] 2.1 实现 `crates/algo-sdk/src/frame.rs`
  - `SafeFrame<'a>` — 从 `&'a AvFrameDesc` 构造
  - `FrameHandleView<'a>` — 4 种硬件句柄解包
  - 访问器：`width()`, `height()`, `pixel_format()`, `stride(plane)`, `alloc_width()`, `alloc_height()`, `wall_time_ns()`, `frame_id()`
  - `handle_view()` — 根据 `opaque_kind` 分派
- [x] 2.2 创建 `crates/algo-sdk/src/cv/` 目录结构
  - `mod.rs` — 门面函数 + 引擎选择 (`OnceLock<Box<dyn CvEngine>>`)
  - `buffer.rs` — `CvBuffer` + `CvBufferKind` + RAII `Drop`
  - `types.rs` — `PixelFormat`, `PreprocessMode`, `LetterboxLayout`
  - `layout.rs` — `compute_letterbox_layout(src_w, src_h, dst_w, dst_h)` 纯计算
- [x] 2.3 实现 `CvEngine` Trait (`cv/mod.rs` 或 `cv/engine.rs`)
  - `letterbox()` / `resize()` 签名
  - `active_engine()` 函数 — 从 `OnceLock` 读取
  - `cv::letterbox()` / `cv::resize()` 门面函数
- [x] 2.4 实现 `CpuCvEngine` (`cv/platforms/cpu.rs`)
  - 纯 Rust 双线性插值缩放
  - Letterbox 模式：先缩放再贴到灰色画布
  - 产出 `CvBuffer { inner: Host(Vec<u8>) }`
  - 单元测试：验证输出尺寸、`PreprocessMode` 参数
- [x] 2.5 实现 `HostCvEngine` (`cv/platforms/host_ops.rs`)
  - 委托宿主注入的 `AvImageOps` 虚表
  - `alloc` → `convert` (缩放贴图) → 产出 `CvBuffer { inner: HostOps { view, free_fn } }`
  - 产出的 `CvBuffer` 在 Drop 时调用 `AvImageOps.free` 归还
- [x] 2.5.1 实现 `AppleCvEngine` (`cv/platforms/apple.rs`)
  - 深度绑定 Apple `Accelerate.framework` (vImage SIMD 硬件指令) 与 `CoreVideo` (`CVPixelBuffer`)
  - 零拷贝锁定并读取 `CVPixelBuffer` 双平面 NV12
  - 调用 `vImageConvert_420Yp8_CbCr8ToARGB8888` 向量化硬件转色彩空间
  - 调用 `vImageScale_ARGB8888` 硬件 SIMD 缩放至目标 Letterbox 画布
  - 调用 `vImageConvert_RGBA8888toRGB888` 打包紧凑 RGB24 输出
  - `active_engine()` 在 macOS 环境下默认优先调度 `AppleCvEngine`
- [x] 2.6 编写 `cv` 模块测试 (`tests/cv_tests.rs`)
  - `CpuCvEngine` letterbox: 输入 640x480 → 640x640，验证黑边参数
  - `CpuCvEngine` resize: 输入 640x480 → 320x320，验证输出尺寸
  - `AppleCvEngine` letterbox + resize: 真实 `CVPixelBuffer` 硬件显存输入 + SIMD 缩放黑边验证
  - `CvBuffer` Drop 行为：验证 Host 模式不泄漏

### 验证

```bash
cargo test -p algo-sdk -- --test cv_tests
```

---

## Phase 3: Model + 后处理 + Emitter

### 步骤

- [x] 3.1 实现 `crates/algo-sdk/src/model.rs`
  - `Core` 枚举：`Auto`, `Id(u8)`, `All`, `Mask(u32)`，`#[derive(Default)]` 默认 `Auto`
  - `ModelWeights` Trait — `session_on(&self, core: Core) -> Result<Session, AlgoError>`
  - `InferenceSession` Trait — `infer(&mut self, input: &CvBuffer) -> Result<Output, AlgoError>`
  - `SharedWeights<W>` — `Arc<W>` + `AtomicUsize` Round-Robin
  - 单元测试：
    - 1 核设备：`session()` 始终返回 `Core::Id(0)`
    - 3 核设备 (RK3588)：连续 6 次 `session()` 依次分配 0,1,2,0,1,2
    - `session_on(Core::Id(2))` 始终返回核心 2
- [x] 3.2 实现 `crates/algo-sdk/src/math.rs`
  - `NormBox` 结构体 — `x, y, w, h, confidence, class_id`
  - `calculate_iou(a, b) -> f32` — 无分支安全除法
  - `fast_nms(boxes: &mut Vec<NormBox>, iou_threshold: f32)` — 按置信度降序排序 + 贪心抑制
  - `unmap_box(b, mode, src_w, src_h) -> NormBox` — Letterbox 扣黑边 / Resize 线性
  - `clamp_bbox(b: &mut NormBox)` — 截断到 `[0.0, 1.0]`
  - 单元测试 (`tests/math_tests.rs`)：
    - IoU: 完全重叠 → 1.0，无重叠 → 0.0，部分重叠 → 已知值
    - NMS: 空输入、单框、高度重叠 3 框应抑制为 1 框
    - unmap_box Letterbox: 模型空间 (160, 0, 320, 640) 在 640x640 目标上反算回原始归一化坐标
    - unmap_box Resize: 线性缩放验证
    - clamp: 负值截断为 0，>1 截断为 1
- [x] 3.3 实现 `crates/algo-sdk/src/emitter.rs`
  - `ResultEmitter<'a>` — 持有 `on_result` 回调指针
  - `emit_detections(&mut self, boxes: &[NormBox]) -> Result<(), AlgoError>`
    - 构造 JSON: `{ "event_id": "uuid", "objects": [{ "class_id", "label", "confidence", "bbox": [x, y, w, h] }] }`
    - 构造 `AvAlgoImageReq { x:0, y:0, w:1, h:1, purpose: 1 }` 全景大图
    - 构造 `AvAlgoResult { kind: AV_RESULT_ALARM, ... }`
    - 调用 `on_result` 回调
  - `emit_self_test(&mut self, count: usize) -> Result<(), AlgoError>`
    - `kind: AV_RESULT_SELF_TEST`
  - **兼容性验证**：确保产出的 JSON 能被 `parse_alarm_objects` 正确解析
    - 在单元测试中内联 `parse_alarm_objects` 的解析逻辑做验证（不依赖 `infer` crate）

### 验证

```bash
cargo test -p algo-sdk -- --test math_tests
cargo test -p algo-sdk
```

---

## Phase 4: 插件 Trait + `export_algo!` 宏

### 步骤

- [x] 4.1 实现 `crates/algo-sdk/src/plugin.rs`
  - `InitContext<'a>` 结构体
  - `AlgoPlugin` Trait — `init`, `process`, `flush`, `set_rules`
- [x] 4.2 实现 `crates/algo-sdk/src/macros.rs`
  - `export_algo!` 声明宏
  - 展开产出：
    - `static ABI: AvAlgoAbi` — 填充 11 个函数指针
    - `#[no_mangle] pub unsafe extern "C" fn av_algo_get_abi(ver: u32) -> *const AvAlgoAbi`
    - `LibraryContext` — 持有 `package_root`, `platform_id`
    - `InstanceContext<P>` — 持有 `plugin: P`, 回调指针
    - 每个 C 入口函数外层 `std::panic::catch_unwind`
    - `thread_local! { LAST_ERROR: RefCell<String> }` 错误缓存
  - 关键实现细节：
    - `library_open`: 初始化 `LibraryContext`，设置 `CvEngine`
    - `library_query`: 填充 `AvAlgoLibraryInfo`
    - `library_close`: Drop `LibraryContext`
    - `instance_create`: 调用 `AlgoPlugin::init`
    - `instance_process`: 构造 `SafeFrame` + `ResultEmitter`，调用 `AlgoPlugin::process`
    - `instance_flush`: 调用 `AlgoPlugin::flush`
    - `instance_set_rules`: 调用 `AlgoPlugin::set_rules`
    - `instance_destroy`: Drop `InstanceContext<P>`
    - `last_error`: 从 `LAST_ERROR` 读取
    - `instance_negotiate`: 返回 `AV_OK`（本期简化处理）
    - `instance_update_config`: 返回 `AV_OK`（本期简化处理）

### 验证

```bash
cargo clippy -p algo-sdk --all-targets -- -D warnings
cargo test -p algo-sdk
```

---

## Phase 5: 测试脚手架 + 示例算法包 + 集成测试

### 步骤

- [x] 5.1 实现 `crates/algo-sdk/src/testing.rs`
  - `MockFrameBuilder` 构造器：
    - 基础模式：从 Host 内存构建 `SafeFrame`；
    - 图像加载与色彩空间转换：`to_nv12(stride_alignment)` 实现纯 Rust RGB24 转 NV12 并计算硬件 Stride 步长对齐（16/64 字节）；
    - `#[cfg(feature = "testing-image")] from_image_file(path)`：一键从本地 `.jpg`/`.png` 加载；
    - `#[cfg(feature = "testing-hardware")] from_image_hardware(path)`：自动挂载平台原生硬件显存句柄：
      - macOS: `CVPixelBufferCreate` + `AV_OPAQUE_CVPIXELBUFFER`；
      - Linux: DMA-Heap / `memfd` 构造 `dma_buf_fd` + `AV_OPAQUE_DMABUF`；
  - `MockEmitter` — 替代 `ResultEmitter`，将产出捕获到 `Vec<String>` 供断言
  - `MockWeights` / `MockSession` — 简单返回固定输出的推理桩
- [x] 5.2 编写 `tests/plugin_lifecycle.rs`
  - 定义 `MockRustDetector` 实现 `AlgoPlugin`
  - 测试场景：
    - 正常生命周期：`init → process × N → flush → Drop`
    - 硬件加速格式输入：通过 `MockFrameBuilder` 输入 NV12 + DMA-BUF/CVPixelBuffer，验证 `handle_view()` 零拷贝解包成功
    - Panic 隔离：process 中故意 panic，验证 `catch_unwind` 捕获并返回 `AV_ERR_INTERNAL`
    - `set_rules` 接收空规则列表
    - 自检模式 (`is_self_test = true`)
- [ ] 5.3 [可选] 在 `crates/algo-sdk/examples/` 创建 `dummy_detector` cdylib 示例
- [ ] 5.4 [可选, 可标记 `#[ignore]`] 在 `infer/tests/` 添加集成测试

### 验证

```bash
cargo test -p algo-sdk -- --test plugin_lifecycle
cargo test -p algo-sdk
```

---

## Phase 6: 全量质量门禁

- [x] 6.1 格式化
  ```bash
  cargo fmt --all
  ```
- [x] 6.2 Lint
  ```bash
  cargo fmt --all -- --check
  cargo clippy --all-targets -- -D warnings
  ```
- [x] 6.3 测试
  ```bash
  cargo test --workspace
  ```
- [x] 6.4 检查禁止项
  - 无 `dbg!`, `println!`, `todo!()`, `@ts-ignore`
  - 所有 `unsafe` 块均有 `// SAFETY:` 注释
  - 无未记录的 `#[allow(...)]`

---

## 风险与缓解

| 风险 | 缓解 |
|---|---|
| `export_algo!` 宏展开复杂度高，难调试 | 先手写展开验证正确性，再抽象为宏；宏内保持最小逻辑 |
| C ABI 类型两侧漂移 | CI 断言测试 + 文件顶部注释提醒 |
| `CvBuffer::Drop` 调用 C 回调时宿主已释放上下文 | `HostCvEngine` 的 `CvBuffer` 生命周期严格短于 `InstanceContext`，由 `export_algo!` 保证 |
| `CpuCvEngine` 双线性插值实现精度不足 | 对标 OpenCV 参考结果做数值测试 |
