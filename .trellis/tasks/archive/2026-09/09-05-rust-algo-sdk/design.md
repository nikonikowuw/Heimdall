# Technical Design: Heimdall 独立 Rust 算法包开发套件 (algo-sdk)

## 1. 架构总览

`crates/algo-sdk` 遵循 **"HAL 隔离 + SPI 驱动 + 同一代码跨平台硬加速 + 独立仓库零宿主耦合"** 原则。

### 1.1 全链路调用序列

```
宿主 pipeline 调用 instance_process(AvFrameDesc*)
       │
       ▼ [C ABI 边界, export_algo! 生成的 catch_unwind 隔离层]
       │
  SafeFrame::from_raw(&desc)          ← 零拷贝只读借用
       │
  cv::letterbox(&frame, 640, 640)     ← CvEngine SPI 自动分派
       │  产出 CvBuffer (RAII 显存守卫)
       ▼
  session.infer(&buf)                 ← InferenceSession trait
       │  产出 RawOutput
       ▼
  fast_nms / unmap_box                ← 纯 Rust 后处理
       │
  emitter.emit_detections(&boxes)     ← 回调宿主 on_result
       │
       ▼ [C ABI 边界返回 AV_OK]
```

### 1.2 模块依赖拓扑

```
                export_algo! (宏, 展开为 C ABI 符号)
                     │
              ┌──────┴──────┐
              ▼              ▼
         plugin::AlgoPlugin   emitter::ResultEmitter
              │                    │
         ┌────┴─────┐             │
         ▼          ▼             │
    frame::SafeFrame  model::SharedWeights
         │              │         │
         ▼              ▼         │
    cv::CvEngine    cv::CvBuffer ─┘
         │
    ┌────┼────┐
    ▼    ▼    ▼
   Cpu  Host  (Rga/Apple 后续)
         │
         ▼
    fast_math (IoU, NMS, unmap_box)
         │
         ▼
    c_abi (结构体定义, 常量, 错误码)
         │
         ▼
    error::AlgoError
```

### 1.3 关键约束

| 约束 | 原因 |
|---|---|
| `algo-sdk` 零依赖 `types`/`db`/`media`/`infer`/`pipeline`/`api`/`app` | 算法工程师独立仓库可用 |
| `CvBufferKind` 枚举为 `pub(crate)` | 底层硬件句柄不逃逸到算法业务代码 |
| `SafeFrame<'a>` 生命周期绑定 C 回调栈帧 | 防止帧数据逃逸回调作用域 |
| `InferenceSession::infer` 为 `&mut self` | NPU 会话有内部状态，不可并发 |
| `export_algo!` 内部每个 C 入口必须 `catch_unwind` | 算法 panic 不能带崩宿主 |

---

## 2. 核心模块详细设计

### 2.1 C ABI 类型 (`c_abi`)

从 `infer/src/c_abi/types.rs` 精确复制（bit-for-bit），包含所有结构体、常量、函数指针类型。关键结构体尺寸：

| 结构体 | 字节数 | 关键约束 |
|---|---|---|
| `AvFrameDesc` | 152 | 8 字节对齐，`opaque` 在 offset 72 |
| `AvAlgoAbi` | 96 | 11 个函数指针 |
| `AvAlgoInstanceArgs` | 96 | `on_result` 在 offset 64 |
| `AvAlgoResult` | 48 | `json` 在 offset 24 |
| `AvAlgoImageReq` | 32 | `purpose` 在 offset 24 |
| `AvImageOps` | 48 | 4 个函数指针 |

必须包含与 `infer/tests/c_abi_layout_tests.rs` 完全一致的 `size_of` + `offset_of` 断言。

### 2.2 错误体系 (`error`)

```rust
#[derive(Debug, thiserror::Error)]
pub enum AlgoError {
    #[error("配置解析失败: {reason}")]
    ConfigParse { reason: String },
    #[error("模型加载失败: {reason}")]
    ModelLoad { reason: String },
    #[error("推理执行失败: {reason}")]
    Inference { reason: String },
    #[error("预处理失败: {reason}")]
    Preprocess { reason: String },
    #[error("不兼容的帧格式: {reason}")]
    IncompatibleFrame { reason: String },
    #[error("内存不足")]
    OutOfMemory,
    #[error("内部错误: {reason}")]
    Internal { reason: String },
}

impl AlgoError {
    /// 转换为 C ABI 错误码
    pub fn to_c_status(&self) -> c_int { ... }
}
```

### 2.3 安全帧视图 (`frame`)

```rust
/// 从 AvFrameDesc 裸指针构造的强类型安全只读视图。
/// 生命周期 'a 绑定到 C 回调栈帧，防止帧数据逃逸。
pub struct SafeFrame<'a> {
    desc: &'a AvFrameDesc,
}

/// 硬件句柄解包视图（匹配 opaque_kind）
pub enum FrameHandleView<'a> {
    DmaBuf { fd: i32 },
    ApplePixelBuffer { ptr: *mut c_void },
    AscendDeviceMemory { ptr: *mut c_void },
    Host { data: &'a [u8], len: usize },
}

impl<'a> SafeFrame<'a> {
    /// # Safety
    /// `desc` 必须指向有效的 AvFrameDesc，且在 'a 生命周期内不被修改。
    pub(crate) unsafe fn from_raw(desc: &'a AvFrameDesc) -> Self { ... }

    pub fn width(&self) -> u32 { self.desc.width }
    pub fn height(&self) -> u32 { self.desc.height }
    pub fn pixel_format(&self) -> u32 { self.desc.pixel_format }
    pub fn stride(&self, plane: usize) -> i32 { self.desc.stride[plane] }
    pub fn alloc_width(&self) -> u32 { self.desc.alloc_width }
    pub fn alloc_height(&self) -> u32 { self.desc.alloc_height }
    pub fn wall_time_ns(&self) -> i64 { self.desc.wall_time_ns }
    pub fn frame_id(&self) -> u64 { self.desc.frame_id }

    /// 解包底层硬件句柄
    pub fn handle_view(&self) -> FrameHandleView<'a> { ... }
}
```

### 2.4 HAL 与 CvBuffer (`cv::buffer`)

```rust
/// 抽象显存句柄（RAII 管理底层资源）
pub struct CvBuffer {
    inner: CvBufferKind,
    width: u32,
    height: u32,
    format: PixelFormat,
}

/// 底层异构句柄 — pub(crate) 密封，不逃逸到算法业务代码
pub(crate) enum CvBufferKind {
    DmaBuf { fd: i32, guard: Option<Box<dyn Any + Send>> },
    ApplePixelBuffer { ptr: *mut c_void, retain: bool },
    AscendDeviceMemory { ptr: *mut c_void },
    Host(Vec<u8>),
    /// 宿主 AvImageOps 分配的图像视图 — 由宿主 free 回调归还
    HostOps { view: AvImageView, ops_ctx: *mut c_void,
              free_fn: unsafe extern "C" fn(*mut c_void, *mut AvImageView) -> c_int },
}

impl CvBuffer {
    pub fn width(&self) -> u32 { ... }
    pub fn height(&self) -> u32 { ... }
    pub fn format(&self) -> PixelFormat { ... }

    /// 仅供底层推理引擎使用的零拷贝解包
    pub fn as_dma_buf_fd(&self) -> Option<i32> { ... }
    pub fn as_host_bytes(&self) -> Option<&[u8]> { ... }
}

impl Drop for CvBuffer {
    fn drop(&mut self) {
        match &mut self.inner {
            CvBufferKind::DmaBuf { fd, .. } => { /* close(fd) if owned */ }
            CvBufferKind::ApplePixelBuffer { ptr, retain: true } => { /* CVPixelBufferRelease */ }
            CvBufferKind::HostOps { view, ops_ctx, free_fn } => {
                // SAFETY: view/ctx 在 CvBuffer 生命周期内有效，free_fn 由宿主保证安全
                unsafe { free_fn(*ops_ctx, view as *mut _); }
            }
            _ => {}
        }
    }
}
```

### 2.5 CvEngine SPI (`cv::engine`)

```rust
/// 预处理结果描述
pub enum PreprocessMode {
    Letterbox(LetterboxLayout),
    Resize,
}

pub struct LetterboxLayout {
    pub scale: f32,       // 原图缩放比
    pub pad_left: u32,    // 左侧黑边像素
    pub pad_top: u32,     // 上侧黑边像素
    pub dst_w: u32,       // 目标宽
    pub dst_h: u32,       // 目标高
}

/// 驱动 SPI 接口
pub trait CvEngine: Send + Sync {
    fn letterbox(
        &self, frame: &SafeFrame<'_>,
        dst_w: u32, dst_h: u32, fill_color: [u8; 3],
    ) -> Result<(CvBuffer, PreprocessMode), AlgoError>;

    fn resize(
        &self, frame: &SafeFrame<'_>,
        dst_w: u32, dst_h: u32,
    ) -> Result<(CvBuffer, PreprocessMode), AlgoError>;
}

// ── 引擎选择 ──

/// 全局注入的 CvEngine 实例（由 export_algo! 在 library_open 时初始化）
static ACTIVE_ENGINE: OnceLock<Box<dyn CvEngine>> = OnceLock::new();

/// 统一门面：自动委派到当前环境的最佳引擎
pub fn letterbox(frame: &SafeFrame<'_>, dst_w: u32, dst_h: u32, fill: [u8; 3])
    -> Result<(CvBuffer, PreprocessMode), AlgoError>
{
    active_engine()?.letterbox(frame, dst_w, dst_h, fill)
}

pub fn resize(frame: &SafeFrame<'_>, dst_w: u32, dst_h: u32)
    -> Result<(CvBuffer, PreprocessMode), AlgoError>
{
    active_engine()?.resize(frame, dst_w, dst_h)
}
```

**引擎选择优先级** (在 `library_open` 时执行一次)：

1. 如果宿主通过 `AvAlgoInstanceArgs.image_ops` 注入了 `AvImageOps` 虚表 → `HostCvEngine`
2. `#[cfg(feature = "cv-rga")]` → `RgaCvEngine`
3. `#[cfg(target_os = "macos")]` → `AppleCvEngine`
4. 兜底 → `CpuCvEngine`

### 2.6 Model 子系统 (`model`)

```rust
/// 硬件核心调度描述符
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Core {
    #[default]
    Auto,
    Id(u8),
    All,
    Mask(u32),
}

/// 共享模型权重 — 物理显存只占 1 份
pub trait ModelWeights: Send + Sync + 'static {
    type Session: InferenceSession;
    fn session_on(&self, core: Core) -> Result<Self::Session, AlgoError>;
}

/// 独立推理会话 — 每通道独占，&mut self 强制不可并发
pub trait InferenceSession: Send + 'static {
    type Output;
    fn infer(&mut self, input: &CvBuffer) -> Result<Self::Output, AlgoError>;
}

/// 共享权重智能指针
#[derive(Clone)]
pub struct SharedWeights<W: ModelWeights> {
    inner: Arc<W>,
    core_count: u8,
    rr_counter: Arc<AtomicUsize>,
}

impl<W: ModelWeights> SharedWeights<W> {
    pub fn new(weights: W, core_count: u8) -> Self { ... }

    /// 自动轮询核心
    pub fn session(&self) -> Result<W::Session, AlgoError> {
        let idx = self.rr_counter.fetch_add(1, Ordering::Relaxed);
        let core_id = (idx % self.core_count as usize) as u8;
        self.inner.session_on(Core::Id(core_id))
    }

    /// 显式绑定核心
    pub fn session_on(&self, core: Core) -> Result<W::Session, AlgoError> {
        self.inner.session_on(core)
    }
}
```

### 2.7 后处理 (`fast_math`)

```rust
/// 归一化检测框 [0.0, 1.0]
#[derive(Debug, Clone, Copy)]
pub struct NormBox {
    pub x: f32, pub y: f32, pub w: f32, pub h: f32,
    pub confidence: f32,
    pub class_id: u32,
}

/// 快速 IoU 计算
pub fn calculate_iou(a: &NormBox, b: &NormBox) -> f32 { ... }

/// 非极大值抑制 — 按置信度降序排序后贪心抑制
pub fn fast_nms(boxes: &mut Vec<NormBox>, iou_threshold: f32) { ... }

/// 坐标反算 — 从模型输入空间映射回原始帧归一化坐标
pub fn unmap_box(b: &NormBox, mode: &PreprocessMode, src_w: u32, src_h: u32) -> NormBox {
    match mode {
        PreprocessMode::Letterbox(layout) => {
            // 减去黑边偏移 → 除以缩放比 → 归一化
        }
        PreprocessMode::Resize => {
            // 直接归一化（模型输入空间线性对应原图）
        }
    }
}

/// 截断到 [0.0, 1.0]
pub fn clamp_bbox(b: &mut NormBox) { ... }
```

### 2.8 ResultEmitter (`emitter`)

```rust
/// 安全包装宿主回调
pub struct ResultEmitter<'a> {
    frame_id: u64,
    on_result: AvAlgoResultCb,
    user_data: *mut c_void,
    _lifetime: PhantomData<&'a ()>,
}

impl<'a> ResultEmitter<'a> {
    /// 发射检测结果 — 自动构造告警 JSON + 全景抓拍请求
    pub fn emit_detections(&mut self, boxes: &[NormBox]) -> Result<(), AlgoError> {
        // 1. 构造 JSON: { "event_id": uuid, "objects": [...] }
        // 2. 构造 AvAlgoImageReq { x:0, y:0, w:1, h:1 } 全景大图
        // 3. 构造 AvAlgoResult { kind: AV_RESULT_ALARM, ... }
        // 4. 调用 on_result 回调
    }

    /// 自检模式发射
    pub fn emit_self_test(&mut self, detection_count: usize) -> Result<(), AlgoError> { ... }
}
```

**JSON 格式必须与 `infer/src/package.rs::parse_alarm_objects` 兼容**：

```json
{
  "event_id": "uuid-v4",
  "objects": [
    {
      "class_id": 0,
      "label": "person",
      "confidence": 0.87,
      "bbox": [0.12, 0.34, 0.15, 0.28]
    }
  ]
}
```

### 2.9 AlgoPlugin Trait (`plugin`)

```rust
/// 算法包实例化时由宿主传入的上下文
pub struct InitContext<'a> {
    pub package_root: &'a Path,
    pub platform_id: &'a str,
    pub instance_id: &'a str,
    pub is_self_test: bool,
    // 未来扩展：load_shared_model 等能力
}

/// 算法插件核心 Trait
pub trait AlgoPlugin: Sized + Send + 'static {
    type Config: serde::de::DeserializeOwned + Default;

    fn init(ctx: &InitContext<'_>, config: Self::Config) -> Result<Self, AlgoError>;
    fn process(&mut self, frame: SafeFrame<'_>, emitter: &mut ResultEmitter<'_>) -> Result<(), AlgoError>;
    fn flush(&mut self, _emitter: &mut ResultEmitter<'_>) -> Result<(), AlgoError> { Ok(()) }
    fn set_rules(&mut self, _rules: &[AvRule]) -> Result<(), AlgoError> { Ok(()) }
}
```

### 2.10 导出宏 (`export_algo!`)

```rust
/// 用法:
/// export_algo!(MyDetector, algo_id: "my_detector", version: "1.0.0",
///              algo_type: "object_detection", alarm_type_id: "intrusion");

#[macro_export]
macro_rules! export_algo {
    ($plugin_ty:ty, algo_id: $id:expr, version: $ver:expr,
     algo_type: $atype:expr, alarm_type_id: $alarm:expr) => {
        // 展开为:
        // 1. static ABI: AvAlgoAbi = { ... 11 个函数指针 ... };
        // 2. #[no_mangle] pub unsafe extern "C" fn av_algo_get_abi(ver: u32) -> *const AvAlgoAbi
        // 3. 内部 LibraryContext 与 InstanceContext<$plugin_ty>
        // 4. 每个 C 入口函数：
        //    unsafe extern "C" fn algo_library_open(...) -> c_int {
        //        std::panic::catch_unwind(|| { ... }).unwrap_or(AV_ERR_INTERNAL)
        //    }
        // 5. last_error 线程局部缓存
    };
}
```

**展开后的关键实现细节**：

- `LibraryContext` 持有 `package_root: PathBuf`, `platform_id: String`, 以及 `AvImageOps` 指针副本
- `InstanceContext<P: AlgoPlugin>` 持有 `plugin: P`, `on_result: AvAlgoResultCb`, `user_data: *mut c_void`
- 所有 C 入口 `catch_unwind`；捕获到 panic 时写入 `thread_local! { LAST_ERROR: RefCell<String> }`
- `last_error` 函数从 `LAST_ERROR` 读取最后一次错误信息

### 2.11 测试脚手架与硬件加速帧生成 (`testing`)

为避免“单测走软解 Host 内存通过、上真实硬件板子遇 DMA-BUF/Stride 对齐崩溃”，测试脚手架通过可选 feature 提供将本地图片文件直接构造成硬件加速帧的能力：

```rust
pub struct MockFrameBuilder { ... }

impl MockFrameBuilder {
    /// 基础内存构造
    pub fn new() -> Self { ... }
    pub fn dimensions(mut self, width: u32, height: u32) -> Self { ... }
    pub fn pixel_format(mut self, fmt: u32) -> Self { ... }
    pub fn host_data(mut self, data: Vec<u8>) -> Self { ... }

    /// 【feature = "testing-image"】从本地图片文件直接加载
    #[cfg(feature = "testing-image")]
    pub fn from_image_file(path: impl AsRef<Path>) -> Result<Self, AlgoError> { ... }

    /// RGB24 自动转 NV12，并严格按 hardware_stride 对齐行步长
    pub fn to_nv12(self, stride_alignment: u32) -> Self { ... }

    /// 【feature = "testing-hardware"】自动/显式挂载平台原生硬件显存句柄
    /// - macOS: 创建真实 CVPixelBuffer (AV_OPAQUE_CVPIXELBUFFER)
    /// - Linux: 创建 DMA-BUF fd (通过 dma-heap 或 memfd 导出，AV_OPAQUE_DMABUF)
    #[cfg(feature = "testing-hardware")]
    pub fn from_image_hardware(path: impl AsRef<Path>) -> Result<Self, AlgoError> { ... }

    /// 产出拥有完整生命周期的 MockFrame，可随时借用为 SafeFrame
    pub fn build(self) -> MockFrame { ... }
}
```

---

## 3. 文件结构

```
crates/algo-sdk/
├── Cargo.toml
├── src/
│   ├── lib.rs           # pub mod 声明 + 统一 re-export
│   ├── c_abi.rs          # C ABI 类型 1:1 映射
│   ├── error.rs          # AlgoError
│   ├── frame.rs          # SafeFrame + FrameHandleView
│   ├── cv/
│   │   ├── mod.rs        # 门面函数 + 引擎选择
│   │   ├── buffer.rs     # CvBuffer + CvBufferKind
│   │   ├── types.rs      # PixelFormat, PreprocessMode, LetterboxLayout
│   │   ├── layout.rs     # compute_letterbox_layout
│   │   └── platforms/
│   │       ├── mod.rs
│   │       ├── cpu.rs    # CpuCvEngine (纯 Rust)
│   │       └── host_ops.rs # HostCvEngine (AvImageOps 委托)
│   ├── model.rs          # Core, ModelWeights, InferenceSession, SharedWeights
│   ├── math.rs           # IoU, NMS, unmap_box, clamp_bbox
│   ├── emitter.rs        # ResultEmitter, DetectionBox
│   ├── plugin.rs         # AlgoPlugin trait, InitContext
│   ├── macros.rs         # export_algo! 宏
│   └── testing.rs        # MockFrameBuilder, MockEmitter, MockWeights
└── tests/
    ├── c_abi_layout_tests.rs  # size_of + offset_of 断言
    ├── math_tests.rs          # NMS / IoU / unmap 测试
    ├── cv_tests.rs            # CpuCvEngine 集成测试
    └── plugin_lifecycle.rs    # MockRustDetector 完整生命周期
```

---

## 4. Cargo.toml 设计

```toml
[package]
name = "algo-sdk"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
libc = "0.2"
static_assertions = "1.1"
uuid = { version = "1", features = ["v4"] }

# thiserror 仅在 std 环境使用
thiserror = "2.0"

# 测试与工具可选依赖：默认不编译，保证生产 cdylib 插件零冗余体积
image = { version = "0.25", default-features = false, features = ["jpeg", "png"], optional = true }

[dev-dependencies]
# algo-sdk 自身单测开启全部测试特性
image = { version = "0.25", default-features = false, features = ["jpeg", "png"] }

[features]
default = []

# 测试脚手架可选特性：算法工程师在独立仓 [dev-dependencies] 中引入
testing-image = ["dep:image"]
testing-hardware = ["testing-image"]

# 硬件加速驱动 (后续扩展)
# cv-rga = []
# cv-apple = []

[lints]
workspace = true
```

---

## 5. 兼容性与演进

### 5.1 ABI 稳定性

- `AvAlgoAbi.api_version` 当前为 `1`
- 添加新字段只能追加在结构体末尾，并增加 `api_version`
- 旧版 SDK 编译的算法包可以在新版宿主上运行（向前兼容）

### 5.2 未来演进路径

| 演进项 | 时机 |
|---|---|
| 抽取 `crates/algo-abi` 作为单一 C ABI 来源 | 当 SDK 需要独立发布到 crates.io |
| `RgaCvEngine` 硬件驱动 | 在 Rockchip 目标设备上验证时 |
| `AppleCvEngine` 硬件驱动 | macOS 开发机验证时 |
| ARM NEON intrinsics 优化 `fast_nms` | 性能剖析确认瓶颈后 |
| `ModelRegistry` 全局去重缓存池 | 多算法包共享模型时 |
