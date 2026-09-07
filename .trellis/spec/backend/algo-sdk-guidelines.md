# 算法开发套件 (algo-sdk) 与 C ABI 规范

> `crates/algo-sdk` 是专为 Rust 算法开发者提供的轻量独立开发套件，
> 负责打通标准 C ABI 虚拟方法表、帧安全视图、硬件视觉预处理 (HAL)、向量化后处理与结果发射器。
> 算法插件在编译为 `.so` / `.dylib` 动态库时，对宿主业务 crate（`types`/`infer`/`pipeline`/`db`）零依赖。

---

## 1. Scope / Trigger

### 触发条件
- 开发或接入新的 Rust/C++ 动态检测/识别算法包
- 修改 `AvFrameDesc`、`AvPluginVTable`、`AvDetectResult` 等跨 FFI C ABI 结构体
- 调整硬件图像预处理链路（`CvEngine` / `CvBuffer`）或新增平台加速后端（如 RGA/VPC）
- 优化算法检测框反算（`unmap_box`）、NMS 算子或宿主告警事件发射器（`ResultEmitter`）

---

## 2. Signatures

### C ABI 核心导出与虚表定义
```rust
// crates/algo-sdk/src/c_abi.rs
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

### 算法插件导出宏与入口
```rust
// 算法作者在算法 crate 根目录直接调用
export_algo!(MyAlgorithmPlugin);

pub trait AlgoPlugin: Sized + 'static {
    type Instance: AlgoInstance;
    fn init(config_json: &str) -> Result<Self, AlgoError>;
    fn create_instance(&self, stream_id: &str, rules: &[SafeRule]) -> Result<Self::Instance, AlgoError>;
}

pub trait AlgoInstance: 'static {
    fn process(&mut self, frame: &SafeFrame, emitter: &mut ResultEmitter) -> Result<(), AlgoError>;
}
```

### 硬件图像预处理 SPI 与安全视图
```rust
// 硬件预处理引擎抽象
pub trait CvEngine: Send + Sync {
    fn letterbox(&self, frame: &SafeFrame, dst_w: u32, dst_h: u32, fill_color: [u8; 3]) -> Result<(CvBuffer, PreprocessMode), AlgoError>;
    fn resize(&self, frame: &SafeFrame, dst_w: u32, dst_h: u32) -> Result<(CvBuffer, PreprocessMode), AlgoError>;
}

// 帧内存安全借用视图
impl<'a> SafeFrame<'a> {
    pub unsafe fn from_raw_checked(raw: *const AvFrameDesc) -> Result<Self, AlgoError>;
    pub fn width(&self) -> u32;
    pub fn height(&self) -> u32;
    pub fn stride(&self, plane: usize) -> i32;
    pub fn plane_offset(&self, plane: usize) -> usize;
    pub fn handle_view(&self) -> FrameHandleView<'a>;
}
```

---

## 3. Contracts

### 3.1 AvFrameDesc 内存布局契约
```text
Offset | Field           | Type       | Value / Constraint
-------+-----------------+------------+---------------------------------------------
0      | size            | uint32_t   | 必须固定为 152 字节
4      | api_version     | uint32_t   | 必须为 1 (AV_ALGO_API_VERSION)
8      | frame_id        | uint64_t   | 帧全局单调自增序号
16     | wall_time_ns    | int64_t    | 墙上时间戳（纳秒）
24     | pts_ns          | int64_t    | PTS 视频呈现时间戳（纳秒）
32     | modifier        | uint64_t   | DRM 格式修饰符
40     | offset[4]       | uint64_t*4 | 各平面相对于 opaque 的起始内存偏移
72     | opaque          | void*      | 平台句柄（DMA-BUF fd / CVPixelBufferRef / Host 指针）
80     | frame_token     | void*      | 帧生命周期 Token
88     | platform_tag    | uint32_t   | 平台标识 (0=Unknown, 1=Apple, 2=RK, 3=Ascend)
92     | opaque_kind     | uint32_t   | 0=Host, 1=DmaBuf, 2=CVPixelBuffer
96     | memory_type     | uint32_t   | 内存类型枚举
100    | pixel_format    | uint32_t   | 0=NV12, 1=I420, 2=BGR24, 3=RGB24, 4=BGRA
104    | layout          | uint32_t   | 内存排布
108    | width           | uint32_t   | 图像可见宽度
112    | height          | uint32_t   | 图像可见高度
116    | alloc_width     | uint32_t   | 硬件对齐/步长虚宽（如 16 字节对齐宽度）
120    | alloc_height    | uint32_t   | 硬件对齐/步长虚高（如 16 字节对齐高度）
124    | stride[4]       | int32_t*4  | 各平面物理步长（字节）
140    | color_primaries | uint16_t   | 色彩空间元数据
142    | color_transfer  | uint16_t   | 传输特性元数据
144    | color_matrix    | uint16_t   | 转换矩阵 (BT.601 / BT.709)
146    | color_range     | uint8_t    | 0=Limited, 1=Full
147    | reserved        | uint8_t*5  | 保留字段补齐至 152 字节
```

### 3.2 告警事件与检测框序列化契约
- **信封格式**：`{"event_id": "<uuid-v4>", "objects": [{"class_id": 0, "label": "person", "confidence": 0.95, "bbox": [x, y, w, h]}]}`。
- **坐标归一化约束**：`x, y, w, h` 必须归一化至 `[0.0, 1.0]` 浮点区间，左上角原点，尺寸非负。
- **零分配序列化**：通过 `BoxesSerializer` 直接流式写入字节向量并在末尾补充 `\0` NUL 字节，避免中途为每个对象分配堆内存与中间字符串。
- **抓拍请求挂载**：`ResultEmitter` 在发射告警时，默认自动请求主码流全景大图抓拍 (`image_type = 0`)，特写抓拍 (`image_type = 1`) 必须带 10% 扩边框。

### 3.4 Rockchip RGA 硬件加速与显存池契约
- **动态库隔离与线程安全**：Linux 平台运行时动态加载 `librga.so` / `librga.so.2`，不产生链接期强绑定。因底层驱动线程安全限制，所有进入 `librga` 的 API 调用必须由内部互斥锁序列化保护。
- **预热句柄复用与陈旧句柄防御**：`RgaBufferPool` 在初始化时分配 DMA-BUF 并单次调用 `importbuffer_fd` 绑定 `rga_buffer_handle_t`；后续租借归还仅复用 handle 与 `wrapbuffer_handle_t`，严禁每帧频繁 import/release 造成 IOMMU 映射震荡与内核锁争用。输入源 DMA-BUF 由 `RgaHandleGuard` 执行基于单帧同步生命周期的 RAII 保护，处理完成后立即调用 `releasebuffer_handle`，坚决避免跨帧非受控缓存导致内核 fd 轮转复用时命中陈旧句柄（Stale Handle）；输出端缓冲区则由受控 `RgaBufferPool` 统一预热并复用长期句柄。
- **多输出规格安全硬顶**：单个 `RgaCvEngine` 实例最多缓存 16 个不同输出分辨率几何规格的 `RgaBufferPool`，超限时主动拒绝，杜绝异常动态分辨率打爆物理显存。
- **纯设备路径防御**：输入 DMA-BUF 必须为无 format modifier (`modifier == 0`) 的线性连续帧；NV12 必须满足 `stride[0] == stride[1]`；I420 必须满足 `stride[0] == 2 * stride[1] == 2 * stride[2]`；单平面的最大扫描行步长硬限制为 32,768 (`RGA_MAX_STRIDE`)。非法排布直接返回 `AlgoError::IncompatibleFrame`，严禁在 DMA-BUF 路径悄悄执行 CPU 软解或 readback 伪装为硬件加速。
- **DMA-BUF 堆选取安全**：Linux dma_heap 分配优先尝试 `/dev/dma_heap/system-dma32`（规避 RGA2 4GB 寻址限制）与 `/dev/dma_heap/system`；对于 `Auto` 与 `Rga2` 调度策略，直接前置过滤非 DMA32 堆，防止非法物理地址进入硬件加速器。分配失败时记录包含候选路径与系统 errno 的详细告警日志。

### 3.5 Panic 绝对隔离屏障
- `export_algo!` 导出的所有 FFI 函数入口（`init` / `destroy` / `create_instance` / `process`）必须通过 `std::panic::catch_unwind` 隔离。
- 严禁任何 Rust panic unwind 逃逸到宿主 C ABI 栈，发生 panic 时记录错误并向宿主返回 `AV_STATUS_ERR_PANIC` (-5) 或 `AV_STATUS_ERR_INTERNAL` (-1)。

---

## 4. Validation & Error Matrix

| 输入条件 / 场景 | 校验位置 | 产生的错误 / 返回状态码 | 处理动作 |
|----------------|---------|-----------------------|---------|
| 传入空指针 (`raw.is_null()`) | `validate_abi_header` | `AV_STATUS_ERR_INVALID_PARAM` (-2) | 立即拦截，拒绝解引用 |
| 指针未按结构体 ABI 对齐 | `validate_abi_header` | `AlgoError::Preprocess("未按 ABI 对齐")` | 立即拦截，杜绝未对齐读取 UB |
| `size < 152` 字节 | `SafeFrame::from_raw_checked` | `AlgoError::Preprocess("大小不足")` | 拒绝读取未分配的尾部字段 |
| `api_version != 1` | `SafeFrame::from_raw_checked` | `AlgoError::Preprocess("不支持的 ABI 版本")` | 拒绝跨大版本加载 |
| `stride[0] < alloc_width` | `SafeFrame::from_raw_checked` | `AlgoError::Preprocess("步长非法")` | 拒绝处理破损步长描述 |
| 显式 `offset` 平面区间重叠 | `SafeFrame::from_raw_checked` | `AlgoError::Preprocess("平面内存非法重叠")` | 杜绝 Y/UV 数据交叉读写越界 |
| 内存计算溢出 (`checked_mul`) | `SafeFrame` / `CvEngine` | `AlgoError::OutOfMemory` | 抛出内存超限，不分配脏内存 |
| 算法逻辑触发 `panic!` | `export_algo!` 外层 | `AV_STATUS_ERR_PANIC` (-5) | 捕获 panic，保护宿主进程不崩溃 |

---

## 5. Good/Base/Bad Cases

### Good Case: Rockchip RGA 硬件零拷贝预处理 (Linux + rga)
```rust
// DMA-BUF -> DMA-BUF (RGB888)，全链路位于 Linux DMA-BUF 显存池，零 CPU 拷贝
// 使用预热句柄池 (RgaBufferPool) 消除每帧 importbuffer_fd 的 IOMMU 映射开销
let engine = RgaCvEngine::new();
let (buffer, mode) = engine.letterbox(&safe_frame, 640, 640, [114, 114, 114])?;
let dma_fd = buffer.as_dma_buf().ok_or(AlgoError::Preprocess { reason: "无 DMA-BUF".into() })?;
// 下游直接交付 RKNN NPU 零拷贝输入；buffer 析构时自动通过 RAII 归还池槽位，不释放 RGA handle
```

### Good Case: Apple vImage 硬件零拷贝预处理
```rust
// NV12 CVPixelBuffer -> BGRA CVPixelBuffer，全链路位于 Apple 统一显存，零 CPU 拷贝
let (buffer, mode) = AppleCvEngine.letterbox(&safe_frame, 640, 640, [114, 114, 114])?;
let raw_output = my_coreml_model.infer(&buffer)?;
let mut boxes = my_postprocess(raw_output)?;
fast_nms(&mut boxes, 0.45);
for b in &mut boxes {
    *b = unmap_box(b, &mode, safe_frame.width(), safe_frame.height());
}
emitter.emit_detections(&boxes)?;
```

### Base Case: CPU 回退预处理 (Host NV12/I420 -> RGB24)
```rust
// 物理无硬件 2D 加速器时保底，严格遵循 stride 对齐计算
let (buffer, mode) = CpuCvEngine::new().letterbox(&safe_frame, 640, 640, [114, 114, 114])?;
let rgb_slice = buffer.as_host_bytes().ok_or(AlgoError::Preprocess { reason: "无 Host 数据".into() })?;
```

### Bad Case: 凭经验硬编码内存偏移与全量读回
```rust
// ❌ 错误：忽略 stride 与 alloc_height，强制假设密集排列；且在常驻路径执行 Host readback
let y_plane = unsafe { std::slice::from_raw_parts(desc.opaque as *const u8, (desc.width * desc.height) as usize) };
// ❌ 错误：未对齐和未计算 padding 会导致在 RK3588/昇腾等硬解码帧上发生斜切花屏或内存越界 Segfault！
```

---

## 6. Tests Required (with Assertion Points)

1. **C ABI 字段偏移与尺寸绝对一致性** (`tests/c_abi_layout_tests.rs`)：
   - 断言 `size_of::<AvFrameDesc>() == 152`
   - 断言 `offset_of!(AvFrameDesc, pts_ns) == 24`
   - 断言 `offset_of!(AvFrameDesc, opaque) == 72`
   - 断言 `offset_of!(AvFrameDesc, stride) == 124`
2. **描述符防御性边界检验** (`crates/algo-sdk/src/frame.rs`)：
   - 验证空指针返回 `None` / `Err`
   - 验证 ABI `size < 152` 拒绝加载
   - 验证 NV12 平面偏移重叠 (`offset[0] == offset[1]`) 立即报错
   - 验证 `stride[0] < width` 立即报错
3. **坐标映射双向还原与黑边剔除** (`tests/math_tests.rs`)：
   - 验证 16:9 输入在 1:1 Letterbox 下，四周填充黑边区域坐标被准确剔除，有效物体中心坐标精确还原至 `[0.0, 1.0]` 原图比例。
4. **插件 Panic 隔离验证** (`tests/plugin_lifecycle.rs`)：
   - 构造在 `init` 或 `process` 中主动 `panic!("boom")` 的恶意/缺陷插件，验证宿主捕获状态码为负数且主进程正常继续工作。

---

## 7. Wrong vs Correct

### 7.1 结构体对齐校验

#### ❌ Wrong (依赖隐式转换或取模)
```rust
if (raw as usize) % std::mem::align_of::<T>() != 0 {
    return Err(...);
}
```

#### ✅ Correct (现代标准库方法，意图明确且无分支惩罚)
```rust
if !(raw as usize).is_multiple_of(std::mem::align_of::<T>()) {
    return Err(AlgoError::Preprocess {
        reason: "指针未按 ABI 对齐".to_string(),
    });
}
```

### 7.2 结果发射与 JSON 序列化

#### ❌ Wrong (中间大量分配 String 与动态 Vector)
```rust
let objects: Vec<serde_json::Value> = boxes.iter().map(|b| {
    json!({
        "class_id": b.class_id,
        "label": b.label.clone().unwrap_or_default(),
        "bbox": [b.x, b.y, b.w, b.h]
    })
}).collect();
let json_str = serde_json::to_string(&json!({ "objects": objects })).unwrap();
let c_str = CString::new(json_str).unwrap(); // 存在双重拷贝与内存分配
```

#### ✅ Correct (零分配流式序列化切片与原地追加 NUL)
```rust
struct BoxesSerializer<'a>(&'a [NormBox]);
impl<'a> Serialize for BoxesSerializer<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where S: Serializer {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for b in self.0 {
            seq.serialize_element(&JsonAlarmObject {
                class_id: b.class_id,
                label: b.label.unwrap_or(""),
                confidence: b.confidence,
                bbox: [b.x, b.y, b.w, b.h],
            })?;
        }
        seq.end()
    }
}

let mut json_bytes = serde_json::to_vec(&envelope)?;
let json_len = json_bytes.len() as u32;
json_bytes.push(0); // 原地追加 C 字符串终止符，零额外堆拷贝
let c_json = CStr::from_bytes_with_nul(&json_bytes)?;
```

---

## 8. 算法包本地工程交付与调试规约 (Local Runner & Packaging)

算法包作为独立交付单元，必须具备完整自治的本地验证能力，且必须严格遵循以下契约：

### 8.1 Makefile 标准生命周期（严格收敛为 7 项指令）
严禁在算法包 Makefile 内塞入私货指令或破坏标准接口，必须保持极简与纯粹：

| 命令 | 行为与副作用约束 |
|---|---|
| `make` / `make build` | 编译高度优化的 Release 动态库并自动同步至 `lib/lib<algo>.dylib`（或 `.so`） |
| `make test` | **纯净单元测试**：仅运行 `cargo test`，绝不允许输出图片或侵入业务日志 |
| `make run` | 运行单机真实图像检测与可视化，支持 `.env` 调试覆盖，输出美化 JSON 并保存 `result.jpg` |
| `make benchmark` | 运行基准性能剖析（默认 5 轮预热 + 100 轮循环），输出分阶段性能指标报告 |
| `make stress` | **满载压力与稳定性测试**：在指定时长内（默认 30 秒，支持 `DURATION=60`）满载不间断推理，验证零崩溃、零内存泄漏与长时间热稳定性 |
| `make package` | 自动编译 Release 产物并打包为纯净的 `<algo>.tar.gz`（必须包含 `README.md`、`manifest.json`、`config.schema.json`、`testimage.jpg`、`model/` 与 `lib/`；严格排除源码、构建缓存、`.env` 与测试输出） |
| `make clean` | 彻底清理 `target/` 缓存、动态库、打包文件与 `result.jpg` |

### 8.2 规范性文档要求 (`README.md` 强制契约)
**所有算法包根目录必须提供完整的 `README.md` 说明文档，且打包时必须同动态库一起归档分发**。
`README.md` 必须涵盖以下结构：
1. **规格信息**：`algorithm_id`、`version`、`platform_id`、`alarm_type_id`、模型输入输出与类别说明；
2. **目录排布**：各文件功能定位与交付物结构树；
3. **快速上手**：`make build`、`make test`、`make run`、`make benchmark`、`make package` 操作指导；
4. **本地调试配置**：`.env` 变量覆盖指南；
5. **硬件基准数据**：真机 NPU/ANE 推理延迟、FPS 吞吐实测指标。

### 8.3 `.env.example` 与 `.env` 调试配置协议
算法包根目录必须提供 `.env.example` 模板文件，并在 `.gitignore` 中隔离本地 `.env` 与 `result.jpg`：

```env
CONF_THRESH=0.5
IOU_THRESH=0.45
INPUT_IMAGE=testimage.jpg
OUTPUT_IMAGE=result.jpg
MODEL_PATH=model/yolo26n.mlpackage
TARGET_CLASSES=
LOOPS=100
WARMUP=5
```
- 本地单机 runner 必须优先解析本地 `.env` 文件，不存在时优雅回退至进程环境变量与默认常数；
- 禁止将算法私有 `.env` 读取逻辑泄漏至宿主生产流水线。

### 8.4 终端输出与检测 JSON 美化规范
在单机执行 `make run` 时，终端输出格式必须严格对齐如下格式：

```text
[Config] confidence=0.5 iou=0.45 input=testimage.jpg output=result.jpg model=model/yolo26n.mlpackage loops=1 warmup=0
[Detection Result]
{
  "event_id": "bec13422-bdab-41ee-9e82-2f8d5167b698",
  "objects": [
    {
      "class_id": 5,
      "label": "bus",
      "confidence": 0.9053,
      "bbox": [0.0030, 0.2113, 0.9961, 0.4762]
    },
    {
      "class_id": 0,
      "label": "person",
      "confidence": 0.8706,
      "bbox": [0.0621, 0.3633, 0.2383, 0.4811]
    }
  ]
}
[Visualizer] Saved result image to result.jpg
```
- **缩进规范**：根对象 2 空格缩进，目标列表项 4 空格缩进；
- **浮点数定点四位截断**：`confidence` 与坐标归一化 `bbox: [x, y, w, h]` 统一格式化为 `0.0000` 格式，杜绝 `0.0030381945` 冗余浮点尾数；
- **坐标单行紧凑**：`bbox` 必须在单行内展开（`[0.0030, 0.2113, 0.9961, 0.4762]`），杜绝因跨 6 行换行缩进导致列表纵向严重拉伸。

### 8.5 阶段级性能分析基准规范 (`make benchmark`)
运行性能分析时，必须统计并输出阶段级基准报告，精准剖析硬件耗时瓶颈：

```text
--- Benchmark Report (100 iterations, 5 warmup) ---
  Preprocess:  Avg 1.30 ms | P50 1.28 ms | P99 1.70 ms
  Inference:   Avg 3.52 ms | P50 3.54 ms | P99 5.21 ms
  Postprocess: Avg 0.00 ms | P50 0.00 ms | P99 0.00 ms
  End-to-end:  Avg 4.82 ms | P50 4.80 ms | P99 6.81 ms | FPS: 207.5
  ABI process: Avg 4.82 ms | P50 4.73 ms | P99 6.41 ms | FPS: 207.4
```
- **Preprocess**：HAL 硬件 Letterbox / 缩放对齐耗时；
- **Inference**：NPU 驱动硬件前向推理耗时；
- **Postprocess**：张量反序列化、阈值过滤与坐标反算耗时；
- **End-to-end**：单帧纯算法阶段合计耗时与对应理论上限 FPS；
- **ABI process**：包含 C ABI 栈参数拆装与回调发射的完整端到端耗时。

### 8.6 满载持续压力测试规约 (`make stress`)
算法包必须支持无人值守的满载耐力压测，验证长时间持续高并发吞吐下的鲁棒性：
- **可配置时长**：支持命令行参数 `make stress DURATION=60` 或 `.env` 中的 `DURATION`，默认 30 秒；
- **心跳监控**：压测过程中每 5 秒打印一次实时处理帧数与瞬时吞吐心跳；
- **硬性验收契约**：
  1. **零 Panic、零崩溃、零段错误 (Segfault)**；
  2. **内存稳定性**：压测周期内进程驻留内存 (RSS) 必须保持平稳，验证 CVPixelBuffer、DMA-BUF 租约与 Objective-C `AutoreleasePool` 的成对释放，严禁内存或句柄泄漏；
  3. **吞吐衰减判定**：观察是否存在因 NPU/CPU 发热过大导致的严重降频（Thermal Throttling）。压测结束必须完整输出总帧数、总时长、平均 FPS 与单帧平均延迟。

### 8.7 算法包分发与上传体积弹性配置 (Configurable Max Package Size)
由于边缘端算法包覆盖轻量检测模型（几十 MB）到多模态大模型/视觉语言模型（数百 MB 乃至数 GB），系统**严禁硬编码限制上传体积**，必须提供分层级可配置机制：

1. **默认工业级上限**：代码内置默认值为 **1024 MB (1 GB)**；
2. **配置文件设定 (`config.toml`)**：
   ```toml
   [server]
   host = "0.0.0.0"
   port = 8000
   # 算法包单文件上传上限 (MB)，可按需扩展至 2048、4096 等
   max_package_size_mb = 1024
   ```
3. **环境变量无缝覆盖**：
   - 支持 `ARGUS_SERVER__MAX_PACKAGE_SIZE_MB=2048`
   - 支持快捷变量 `ARGUS_MAX_PACKAGE_SIZE_MB=2048`
4. **友好超限防御提示**：当上传包体超出设定阈值时，网关拦截并返回包含当前限制值与配置调整指南的友好提示，杜绝抛出底层原始断流异常。

---

## 9. Apple Silicon CoreML 原生极速推理工程陷阱与加速指南

在 Apple Silicon (M 系列芯片) 下实现极致 CoreML 推理效率，必须严格遵守以下底层硬件约束：

### 9.1 `MLComputeUnits` 枚举映射陷阱（致命级）
在 Apple 原生 Objective-C `MLModelConfiguration.h` 中，计算单元枚举定义如下：
```objc
typedef NS_ENUM(NSInteger, MLComputeUnits) {
    MLComputeUnitsCPUOnly = 0,           // 👈 0 强制纯 CPU 计算！
    MLComputeUnitsCPUAndGPU = 1,
    MLComputeUnitsAll = 2,               // 👈 2 才是真正的 ANE (Neural Engine) + GPU + CPU 全开！
    MLComputeUnitsCPUAndNeuralEngine = 3
};
```

#### ❌ Wrong (误将 0 当作 All，导致 ANE 协处理器掉线)
```rust
// 错误：传入 0 会将模型锁定在纯 CPU 计算模式，单帧耗时从 2.5ms 骤增至 10ms+！
unsafe { msg_send_void!(config, set_compute_units_sel, 0isize) };
```

#### ✅ Correct (准确映射 2，激活 Apple Neural Engine 硬件协处理器)
```rust
// 正确：传入 2isize 才能激活 Apple Neural Engine (ANE)
unsafe { msg_send_void!(config, set_compute_units_sel, 2isize) };
```

### 9.2 Objective-C Runtime 热路径零分配预缓存
在 Rust 中通过 `objc_msgSend` 调用 CoreML 时，严禁在 `predict_pixelbuffer` 前向热路径中动态调用 `objc_getClass` 与 `sel_registerName`。

#### ❌ Wrong (每帧重复执行字符串哈希与堆内存分配)
```rust
pub unsafe fn predict(&self, pixelbuffer: *mut c_void) -> Result<...> {
    let cls = get_class("MLFeatureValue")?;          // ❌ 每帧分配 CString 并在全局表加锁查类
    let sel = register_sel("featureValueWithPixelBuffer:")?; // ❌ 每帧注册选择器
    ...
}
```

#### ✅ Correct (在模型初始化阶段一次性预缓存至结构体)
```rust
#[derive(Clone, Copy)]
pub struct CachedSelectors {
    pub feat_val_cls: *mut c_void,
    pub feat_pixel_sel: *mut c_void,
    pub dict_cls: *mut c_void,
    pub dict_sel: *mut c_void,
    pub predict_sel: *mut c_void,
    ...
}

pub struct CoreMlRunner {
    model: *mut c_void,
    selectors: CachedSelectors, // 前向热路径直接解引用结构体字段发起 objc_msgSend
}
```

### 9.3 Accelerate 框架 SIMD/NEON 向量化反量化
CoreML 输出的 `MLMultiArray` 张量通常为 Float16 半精度浮点数（`dataType == 65552`）。严禁使用 CPU 逐元素标量位运算循环进行转换。

#### ❌ Wrong (标量位运算循环，未利用 CPU 向量单元)
```rust
let u16_ptr = data_ptr as *const u16;
for i in 0..1800 {
    output[i] = half_to_float(*u16_ptr.add(i)); // ❌ 1800 次标量浮点位运算分支
}
```

#### ✅ Correct (调用 Apple Accelerate 框架底层 NEON 向量指令)
```rust
#[link(name = "Accelerate", kind = "framework")]
extern "C" {
    fn vImageConvert_Planar16FtoPlanarF(
        src: *const VImageBuffer,
        dest: *const VImageBuffer,
        flags: u32,
    ) -> isize;
}

// 单次硬件 SIMD 向量调用即刻完成 1800 维 Float16 -> Float32 转换，耗时 < 1µs
let ret = unsafe { vImageConvert_Planar16FtoPlanarF(&src_buf, &dest_buf, 0) };
if ret != 0 {
    return Err(AlgoError::Internal { reason: format!("vImageConvert 失败: {ret}") });
}
```

