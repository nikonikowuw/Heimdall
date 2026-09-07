# 算法开发套件 (algo-sdk) 与 C ABI 规范

> `crates/algo-sdk` 是专为算法开发者提供的独立开发套件，
> 负责打通标准 C ABI 虚拟方法表、帧安全借用、硬件视觉预处理 (HAL)、向量化后处理与结果发射。
> 动态库对宿主业务 crate（`types`/`infer`/`pipeline`/`db`）**零依赖**。

---

## 1. 核心导出与 C ABI 虚表规范

算法插件编译为 `.so` / `.dylib` 动态库，必须导出唯一的标准入口：

```rust
// 算法作者在 crate 根目录直接调用
export_algo!(MyAlgorithmPlugin);

// C ABI 虚表结构体（定义于 crates/algo-sdk/src/c_abi.rs）
#[repr(C)]
pub struct AvPluginVTable {
    pub size: u32,             // 必须 == size_of::<AvPluginVTable>() (40)
    pub api_version: u32,      // 必须 == AV_ALGO_API_VERSION (1)
    pub init: unsafe extern "C" fn(json_config: *const c_char, out_lib: *mut *mut c_void) -> i32,
    pub destroy: unsafe extern "C" fn(lib: *mut c_void) -> i32,
    pub create_instance: unsafe extern "C" fn(lib: *mut c_void, stream_id: *const c_char, rules: *const AvRule, rule_count: u32, out_instance: *mut *mut c_void) -> i32,
}

#[repr(C)]
pub struct AvPluginInstanceVTable {
    pub size: u32,             // 必须 == size_of::<AvPluginInstanceVTable>() (32)
    pub api_version: u32,      // 必须 == AV_ALGO_API_VERSION (1)
    pub process: unsafe extern "C" fn(instance: *mut c_void, frame: *const AvFrameDesc, on_result: AvResultCallback, user_data: *mut c_void) -> i32,
    pub destroy: unsafe extern "C" fn(instance: *mut c_void) -> i32,
}
```

```rust
pub trait AlgoPlugin: Sized + 'static {
    type Instance: AlgoInstance;
    fn init(config_json: &str) -> Result<Self, AlgoError>;
    fn create_instance(&self, stream_id: &str, rules: &[SafeRule]) -> Result<Self::Instance, AlgoError>;
}

pub trait AlgoInstance: 'static {
    fn process(&mut self, frame: &SafeFrame, emitter: &mut ResultEmitter) -> Result<(), AlgoError>;
}
```

---

## 2. 帧描述符与内存契约 (`AvFrameDesc`)

内存布局权威信源定义于 `crates/algo-sdk/src/c_abi.rs`：

- **尺寸与版本硬性约束**：`size == 152` 字节，`api_version == 1`；
- **平台句柄与类型 (`opaque`)**：
  - `opaque_kind = 0`：Host 内存裸指针；
  - `opaque_kind = 1`：Linux `DMA-BUF fd`；
  - `opaque_kind = 2`：macOS `CVPixelBufferRef`；
- **步长虚宽虚高**：算法必须使用 `stride[plane]` 计算跨度，严禁假设 `stride[0] == width`；
- **Panic 绝对隔离屏障**：所有跨 FFI 导出的函数入口内部必须通过 `std::panic::catch_unwind` 隔离，严禁 Rust panic unwind 逃逸到宿主 C 栈（panic 时统一返回 `AV_STATUS_ERR_PANIC` -5）。

---

## 3. 硬件预处理 HAL 与显存池契约

### 3.1 预处理 SPI 抽象 (`CvEngine`)

```rust
pub trait CvEngine: Send + Sync {
    fn letterbox(&self, frame: &SafeFrame, dst_w: u32, dst_h: u32, fill_color: [u8; 3]) -> Result<(CvBuffer, PreprocessMode), AlgoError>;
    fn resize(&self, frame: &SafeFrame, dst_w: u32, dst_h: u32) -> Result<(CvBuffer, PreprocessMode), AlgoError>;
}
```

### 3.2 Rockchip RGA 硬件加速与显存池契约

- **预热句柄池 (`RgaBufferPool`)**：初始化时预分配 DMA-BUF 并单次执行 `importbuffer_fd` 绑定 `rga_buffer_handle_t`；后续仅复用 handle，严禁每帧重复 import/release 触发内核 IOMMU 映射震荡；
- **输入源生命周期安全**：输入源 DMA-BUF 由 `RgaHandleGuard` 执行单帧 RAII 保护，处理完毕立即释放；输出端则由受控显存池统一长期复用；
- **多规格硬顶**：单个 `RgaCvEngine` 最多缓存 16 个不同输出分辨率规格的显存池，超限主动拒绝防 OOM；
- **DMA-BUF 堆选取**：优先分配 `/dev/dma_heap/system-dma32` 与 `/dev/dma_heap/system`；仅在显式指定 `Rga2` 核心调度时前置强制要求 DMA32 堆（4GB 寻址限制），在 `Auto`/`Rga3` 策略下支持 64 位物理地址堆。

---

## 4. 告警事件序列化与坐标反算

- **零分配序列化契约**：通过 `BoxesSerializer` 流式写入字节向量并在末尾原地追加 `\0` NUL 字节（`json_bytes.push(0)`），零中间中间字符串分配；
- **坐标归一化约束**：`[x, y, w, h]` 必须严格归一化至 `[0.0, 1.0]` 浮点区间；
- **坐标反算 (`unmap_box`)**：经 Letterbox 缩放的模型输出框，必须通过 `unmap_box(b, &mode, frame.width(), frame.height())` 精确扣除 padding 边框并还原至原图相对坐标。

---

## 5. 算法包工程交付契约 (Packaging & Local Runner)

算法包为独立交付单元，根目录下必须包含标准 Makefile 与自治验证能力：

### 5.1 标准 Makefile 生命周期（收敛为 7 项指令）

| 命令 | 行为与副作用约束 |
| --- | --- |
| `make` / `make build` | 编译高度优化的 Release 动态库并同步至 `lib/lib<algo>.so`（或 `.dylib`） |
| `make test` | 运行纯净单元测试（`cargo test`），不产生图片与日志污染 |
| `make run` | 本地真实图像检测与可视化，读取 `.env`，单行美化打印检测结果并保存 `result.jpg` |
| `make benchmark` | 性能剖析（5 轮预热 + 100 轮循环），输出 Preprocess / Inference / Postprocess 分阶段耗时与 FPS |
| `make stress` | **满载耐力压测**（默认 30 秒，支持 `DURATION=60`），验证零崩溃、零内存泄漏与热稳定性 |
| `make package` | 自动编译并打包为纯净的 `<algo>.tar.gz`（含 `manifest.json`, `config.schema.json`, `README.md`, `testimage.jpg`, `model/`, `lib/`；严格排除源码、构建缓存与 `.env`） |
| `make clean` | 彻底清理构建产物、临时文件与 `result.jpg` |

### 5.2 交付文件树与体积弹性配置

- **标准归档结构**：包含 `manifest.json`、`config.schema.json`、`README.md`、`testimage.jpg`、`model/` 与 `lib/`；
- **单包体积上限**：默认最大支持 **1024 MB (1 GB)**，支持在 `config.toml` 中通过 `max_package_size_mb` 扩展（环境变量 `ARGUS_MAX_PACKAGE_SIZE_MB` 覆盖）。

---

## 6. Apple Silicon CoreML 原生推理工程避坑指南

- **`MLComputeUnits` 映射陷阱**：原生 Objective-C 枚举中 `MLComputeUnitsCPUOnly = 0`，**必须显式传入 `2` (`MLComputeUnitsAll`)** 才能激活 ANE (Apple Neural Engine) 协处理器，传 `0` 会导致性能骤降 4 倍；
- **选择器热路径预缓存**：严禁在推理热路径调用 `objc_getClass` 与 `sel_registerName`，必须在模型加载初始化阶段一次性预缓存至结构体；
- **Accelerate NEON 向量化反量化**：CoreML 输出的 Float16 半精度浮点张量必须调用 Accelerate 框架底层 `vImageConvert_Planar16FtoPlanarF` 硬件 NEON 指令进行批量反量化，禁止 CPU 逐元素标量位移循环。

---

## 7. Rockchip RKNN (RK3576 / RK3588) 原生推理工程避坑指南

- **NPU 多核调度强制激活**：RK3576 (双核) 必须配置 `RKNN_NPU_CORE_0_1` (掩码 3)，RK3588 (三核) 配置 `RKNN_NPU_CORE_0_1_2` (掩码 7)。默认 `AUTO (0)` 仅被调度至单核（RK3576 单核推理 ~15.2ms vs 双核 ~8.4ms）；
- **DMA-BUF 虚拟内存长期缓存 (`dma_mem_cache`)**：
  - `rknn_create_mem_from_fd` 要求 `virt_addr` 必须为有效映射内存（不可为 NULL）；
  - 必须通过 `HashMap<i32, DmaMemEntry>` 缓存用户态 `mmap` 地址，严禁每帧调用 `libc::mmap`/`libc::munmap` 引发内核 `mmap_lock` 锁竞争；
- **6 分支 INT8 DFL 原生解码与前置剪枝**：
  - 保持 `want_float = 0` 直接获取驱动原生 INT8 张量，避免驱动在 CPU 端进行耗时 >10ms 的浮点反量化；
  - 遍历分类分支时，校验 `(raw_cls - zp) * scale >= conf_thresh`，未达到阈值的背景网格直接跳过，仅对候选网格执行 16-bin Softmax DFL 坐标还原，将后处理耗时压降至 **1.60ms** 以内；
- **`RknnOutputsGuard` RAII 显存托管**：
  驱动通过 `rknn_outputs_get` 分配的输出缓冲区，必须由 RAII 结构体在 `Drop` 中自动调用 `rknn_outputs_release`，杜绝错误路径或 Panic 时驱动句柄泄漏。

---

## 8. 禁止事项 (Iron Rules)

- ❌ 在算法插件中引入 `types`/`infer`/`pipeline`/`db` 等宿主业务 crate
- ❌ 让 Rust panic unwind 逃逸跨越 C ABI 虚表函数边界（必须 `catch_unwind`）
- ❌ 忽略 `stride` 步长，直接将 `width` 当作扫描行字节跨度（会导致硬解图像斜切花屏或崩溃）
- ❌ 在逐帧推理路径中反复调用 `libc::mmap` / `libc::munmap`
- ❌ 将包含源码、构建缓存或 `.env` 文件的脏目录打包至分发 `.tar.gz` 中
- ❌ 在 CoreML 推理中将 `MLComputeUnits` 误设为 `0`（强制降级为 CPU 模拟）
