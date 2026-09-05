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

### 3.3 Panic 绝对隔离屏障
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
