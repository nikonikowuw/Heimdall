# 推理后端规范

> Argus 要在 Apple Silicon、Ascend、Rockchip 三套完全不同的 NPU SDK 上跑同一套业务逻辑。**平台差异全部收敛在本 crate 内**，上层代码不感知。

> ⚠️ **状态：立项约定（尚未经代码验证）**
> trait 签名是设计草案，首批后端实现落地后必须回填真实签名与示例，并删除本提示。

---

## 铁律

**`infer` 之外的任何 crate，代码里不允许出现平台分支。**

```rust
// ❌ 出现在 pipeline 里
#[cfg(feature = "backend-rknn")]
let out = rknn_infer(frame)?;

// ✅ 上层只看到 trait
let out = self.backend.infer(frame)?;
```

违反这条会导致每加一个平台就要改遍全仓库。发现这种代码要当成 bug 修，不是风格问题。

---

## 抽象层

```rust
// crates/infer/src/backend.rs

/// 后端工厂：负责加载模型，产出可推理的实例。
pub trait InferenceBackend: Send + Sync {
    fn name(&self) -> &'static str;
    fn capabilities(&self) -> BackendCapabilities;
    fn load(&self, spec: &ModelSpec) -> Result<Box<dyn LoadedModel>, InferError>;
}

/// 已加载的模型实例。绑定在固定线程上使用，见 concurrency-guidelines.md。
pub trait LoadedModel: Send {
    fn input_spec(&self) -> &TensorSpec;
    fn infer(&mut self, frame: FrameRef) -> Result<RawOutput, InferError>;
}

pub struct BackendCapabilities {
    /// 支持的输入像素格式（决定预处理路径）
    pub input_formats: &'static [PixelFormat],
    /// 是否支持零拷贝输入（接受平台原生 buffer 句柄）
    pub zero_copy_input: bool,
    /// 可并行推理的实例数（NPU core 数）
    pub parallel_slots: usize,
}
```

设计要点：

- **`infer` 是 `&mut self`**：NPU 会话有内部状态，不是无状态函数。这也强制了"一个模型实例绑一个线程"。
- **`infer` 是同步的**：SDK 就是阻塞的，包成 async 只会骗人。异步化由调用方通过通道完成，见 [concurrency-guidelines.md](./concurrency-guidelines.md)。
- **输入是 `FrameRef` 而非 `Vec<u8>`**：`FrameRef` 持有平台原生 buffer 句柄，让零拷贝成为可能，见 [media-pipeline.md](./media-pipeline.md)。
- **预处理内聚原则（Preprocess Encapsulated in Backend）**：
  - 硬件专属预处理（Rockchip RGA 2D 缩放、Ascend DVPP VPC、Apple CoreVideo/Metal、CPU SIMD）**必须内聚在对应的 Backend 实现内部**，严禁在 `pipeline` 层建立通用的全量 CPU 内存预处理抽象！
  - `pipeline` 只需直接调用 `backend.infer(&frame)`，Backend 内部自洽识别持有的 `FrameHandle`，调用专属 2D 硬件单元完成跨步对齐缩放与色彩转换，全链路显存不落地。
- **架构权衡：Trait 动态分发 (`Box<dyn InferenceBackend>`)**：
  - 为什么不用编译期泛型？AI 边缘推理单次耗时在 5ms ~ 30ms 级别，而 Trait 虚表寻址开销仅数纳秒（ns），可忽略不计；
  - 换来的是避免了泛型单态化导致的编译时长与二进制体积膨胀，且支持运行时根据配置动态按需切换后端或模型实例。

---

## feature flag 约定

```toml
# crates/infer/Cargo.toml
[features]
default = ["backend-cpu"]
backend-cpu    = ["dep:ort"]         # 开源官方 ONNX Runtime 跨平台回退
backend-rknn   = []                  # 瑞芯微平台：链接 native/rknn 极薄 C 垫片与 librknnrt
backend-ascend = []                  # 华为昇腾：链接 native/ascend 极薄 C 垫片与 libascendcl
backend-coreml = ["dep:coreml-rs"]   # 苹果生态：Core ML / ANE 绑定 (或 objc2 原生调用)
```

规则：

- **`backend-cpu` 必须始终可用**，且是默认 feature。开发机上没有 NPU，`cargo test` 和 `cargo clippy` 必须能全绿通过。
- 平台 feature **互不冲突**，允许同时编译多个（虽然实际部署通常只开一个）。
- **不允许"零后端"构建**：`lib.rs` 顶部加编译期断言，至少启用一个后端。
- feature 只控制**编译进哪些实现**，不控制业务行为。运行时选哪个后端由配置决定。

---

## 后端注册与选择

```rust
// crates/infer/src/registry.rs
pub fn available_backends() -> Vec<Box<dyn InferenceBackend>> {
    let mut v: Vec<Box<dyn InferenceBackend>> = Vec::new();
    #[cfg(feature = "backend-rknn")]   v.push(Box::new(RknnBackend::new()));
    #[cfg(feature = "backend-ascend")] v.push(Box::new(AscendBackend::new()));
    #[cfg(feature = "backend-coreml")] v.push(Box::new(CoreMlBackend::new()));
    #[cfg(feature = "backend-cpu")]    v.push(Box::new(CpuBackend::new()));
    v
}
```

约定：

- **`#[cfg]` 只允许出现在 `registry.rs` 和 `backends/mod.rs`**，其它文件里一律不出现。
- 配置里写后端名（`"rknn"`），启动时按名字查找。找不到就报明确错误：`BackendUnavailable { backend }`，消息要提示"该后端未编译进本次构建"。
- 不做"自动探测最优后端"的魔法 —— 边缘设备部署是确定的，配置里写死更可预期，排查也更容易。

---

## 模型规格

模型文件是平台专属的（`.rknn` / `.om` / `.mlpackage`），不能跨平台复用：

```
models/
├── yolov8n/
│   ├── model.onnx           # 中间产物，转换源
│   ├── rk3576/model.rknn
│   ├── ascend310/model.om
│   ├── coreml/model.mlpackage
│   └── manifest.toml        # 输入形状、归一化参数、类别表、量化信息
```

约定：

- **`manifest.toml` 是唯一的元数据来源**。输入尺寸、均值方差、类别名不允许硬编码在 Rust 代码里。
- **模型二进制文件不入 git**（体积大且是构建产物）。转换脚本入 git，模型走单独分发。
- 转换脚本放 `models/<name>/convert/`，每个平台一份，记录完整的转换命令与量化数据集来源。**转换参数变了必须改脚本，不允许只在本地手动跑一遍**。

平台专属的转换细节（ATC 参数、rknn-toolkit2 配置、coremltools 选项）不写在 spec 里 —— 分别由 `ascend-pro`、`rknn-pro` 技能与 Apple 官方 Core ML / VideoToolbox 文档承载。

---

## 后处理放哪

NMS、坐标还原、置信度过滤这类后处理是**平台无关**的，放在 `infer/src/postprocess/`，不要在每个后端实现里各写一份。

例外：部分 SDK 支持把 NMS 融进模型（如 RKNN 的部分算子）。这种情况下后端实现里做格式适配，把结果转成统一的 `Vec<Detection>` 再返回 —— **对外输出格式必须一致**。

---

## 测试策略

| 测试 | 怎么做 |
|------|--------|
| trait 契约测试 | 对 `backend-cpu` 跑完整流程，验证输入输出形状与数值范围 |
| 后处理单元测试 | NMS、坐标变换用固定输入测，不依赖任何后端 |
| 真机后端测试 | `#[ignore]` + `#[cfg(feature = "backend-rknn")]`，只在设备上手动跑 |
| 跨后端一致性 | 同一张图在 CPU 和 NPU 后端上的检测结果差异在阈值内（量化会有偏差，不要求逐位相等） |

**规则**：开发机上 `cargo test` 必须全绿。任何需要真实 NPU 的测试都要 `#[ignore]`。

---

## CoreML / Apple Silicon 专项优化与陷阱

在 Apple Silicon (macOS arm64) 下使用 CoreML 原生推理时，必须严格遵守以下契约（完整实现见 `algo-sdk-guidelines.md` 第 9 节）：

- **`MLComputeUnits` 枚举映射绝对契约**：
  Apple 原生枚举定义中：`MLComputeUnitsCPUOnly = 0`，`MLComputeUnitsCPUAndGPU = 1`，`MLComputeUnitsAll = 2`。必须显式配置为 `2`（或使用 `MLComputeUnitsAll` 常量），严禁传 `0`，否则会强制 CoreML 降级为 CPU 软件模拟，导致推理延迟从 ~2.5ms 骤升至 10ms+！
- **Objective-C Runtime 选择器热路径预缓存**：
  严禁在推理热路径调用 `objc_getClass` 与 `sel_registerName`（每帧 17 次字符串哈希与分配）。必须在模型加载初始化阶段一次性预缓存至常驻句柄。
- **Accelerate 框架 Float16 SIMD 向量化转换**：
  CoreML 输出的 Float16 半精度张量必须直接调用 Accelerate 框架的 `vImageConvert_Planar16FtoPlanarF` 硬件 NEON 指令进行批量反量化，禁止在 Rust 中写 CPU 标量位移循环。

---

## Rockchip RKNN 专项优化与工程实践

在 Rockchip (RK3576 / RK3588) Linux 下使用 RKNN 原生推理时，必须严格遵守以下契约（完整实现见 `algo-sdk-guidelines.md` 第 10 节）：

- **NPU 多核调度掩码强制激活**：
  RK3576 (双核) 必须显式配置 `RKNN_NPU_CORE_0_1` (掩码 3)，RK3588 (三核) 建议配置 `RKNN_NPU_CORE_0_1_2` (掩码 7)。严禁留空使用单核 AUTO 调度（单核推理 ~15.2ms vs 双核 ~8.4ms）。
- **零拷贝 DMA-BUF 虚拟内存常驻缓存**：
  `rknn_create_mem_from_fd` 在 Rockchip BSP 下要求必须传入有效映射的 `virt_addr`。必须通过常驻 `HashMap<fd, DmaMemEntry>` 缓存用户态 `mmap` 地址，禁止在逐帧推理热路径中频繁调用 `libc::mmap` / `libc::munmap` 引发内核 `mmap_lock` 锁竞争。
- **6 分支 INT8 DFL 原生解码与分支剪枝**：
  保持 `want_float = 0` 获取原生 INT8 特征图，避免驱动在 CPU 端进行耗时 ~10ms+ 的浮点反量化。通过类别置信度前置剪枝，只对有效网格执行 16-bin Softmax DFL 坐标还原，将后处理耗时压降至 1.60ms 内。
- **`rknn_outputs_release` RAII 生命周期托管**：
  使用 `RknnOutputsGuard` 封装 `RknnOutput`，确保即使后处理中途抛出 `AlgoError` 或捕获 Panic，驱动级显存与锁资源也能安全释放。
- **RGA 堆分配兼容性**：
  仅在显式指定 `RgaCore::Rga2` 时强制校验 DMA32 堆（4GB 物理地址限制）；在 `Auto` 或 `Rga3` 策略下（如 RK3576/RK3588），允许从 `/dev/dma_heap/system` 64 位物理地址堆分配。

---

## 禁止事项

- ❌ `infer` 之外出现平台 `#[cfg]`
- ❌ 每次推理重新加载模型（加载耗时是推理的几十倍）
- ❌ 把输入输出形状、类别表硬编码在代码里（放 `manifest.toml`）
- ❌ 把 `LoadedModel` 在多个线程间移动使用（SDK 上下文通常有线程亲和要求）
- ❌ 后端实现里返回平台原生的错误码类型（在绑定层就转成 `InferError`）

---

## 待验证事项

- [ ] `FrameRef` 的确切定义（各平台 buffer 句柄如何统一表达）
- [ ] `RawOutput` 是否需要支持多输出头（分割/姿态模型）
- [x] RKNN context 是否真的不可跨线程：已验证单个 `RknnContext` 非线程安全，不可并发调用；必须绑定在专用推理线程内使用；跨线程扩展需使用独立 context 或多进程实例。
- [ ] 三平台量化后的精度差异范围，据此定一致性测试阈值
