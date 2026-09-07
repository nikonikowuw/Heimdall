# C ABI 算法包宿主与沙箱加载体系 技术设计方案 (Subtask 1)

## 1. 架构目标与职责边界

本子任务聚焦于在 Rust 宿主内 1:1 承接原有 C ABI 规范（兼容 `sdk/include/argus/algo.h`、`types.h`、`result.h`），构建安全沙箱与动态加载器，支持现有 `algo-packages/{platform_id}/{algo_id}` 动态热插拔与自带图前向自测。

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                             crates/infer                                    │
│                                                                             │
│  ┌───────────────────────────────────────────────────────────────────────┐  │
│  │                     Safe Public Rust Interfaces                       │  │
│  │  - AlgoRegistry: 算法包注册中心（多平台扫描、动态管理、并发读）           │  │
│  │  - AlgoPackage: 已加载算法包句柄（拥有 Library 与 LibraryContext）        │  │
│  │  - AlgoInstance: 推理实例句柄（实现 InferenceBackend，RAII 生命周期）   │  │
│  └──────────────────────────────────┬────────────────────────────────────┘  │
│                                     │                                       │
│  ┌──────────────────────────────────▼────────────────────────────────────┐  │
│  │                        AlgoSandbox (沙箱校验器)                       │  │
│  │  1. 路径防穿透  2. SHA256完整性  3. Manifest与平台匹配  4. Schema校验  │  │
│  │  5. dlopen符号寻址与ABI尺寸断言  6. testimage.jpg 真实前向自检推理    │  │
│  │  7. 热注册至可用算法注册表                                             │  │
│  └──────────────────────────────────┬────────────────────────────────────┘  │
│                                     │                                       │
│  ┌──────────────────────────────────▼────────────────────────────────────┐  │
│  │                   crates/infer/src/c_abi (FFI 隔离层)                 │  │
│  │  - types.rs: #[repr(C)] 1:1 映射 (av_frame_desc, av_algo_abi 等)      │  │
│  │  - loader.rs: 基于 libloading 的动态库符号加载与封装                  │  │
│  │  - callbacks.rs: C 回调转 Rust 通道 (av_algo_result_cb)               │  │
│  │  - layout_tests.rs: static_assertions 与 offset_of! 双向布局硬核断言  │  │
│  └───────────────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────┬───────────────────────────────────────┘
                                      │ C ABI (extern "C")
                                      ▼
             ┌─────────────────────────────────────────────────┐
             │ algo-packages/macos-arm64-coreml/general_detect │
             │  ├── manifest.json                              │
             │  ├── testimage.jpg                              │
             │  ├── model/ (yolo26n.mlpackage)                 │
             │  └── lib/libgeneral_detection.dylib             │
             └─────────────────────────────────────────────────┘
```

---

## 2. C ABI 数据结构与虚表 1:1 映射 (`crates/infer/src/c_abi/types.rs`)

严格遵循 64 位 C ABI 内存布局（`#pragma pack(push, 8)`），每个结构体均修饰 `#[repr(C)]`：

### 2.1 状态码与枚举
- `av_algo_status`：`AV_OK = 0`，`AV_ERR_UNSUPPORTED_API = -1`，`AV_ERR_INVALID_ARG = -2`，`AV_ERR_INCOMPATIBLE_FRAME = -3`，`AV_ERR_CONFIG_INVALID = -4`，`AV_ERR_MODEL_LOAD_FAILED = -5`，`AV_ERR_INFERENCE_FAILED = -6`，`AV_ERR_OUT_OF_MEMORY = -7`，`AV_ERR_NOT_IMPLEMENTED = -8`，`AV_ERR_TIMEOUT = -9`，`AV_ERR_INTERNAL = -99`。
- `av_pixel_format`：`AV_PIX_UNKNOWN = 0`，`AV_PIX_NV12 = 1`，`AV_PIX_BGRA = 2`，`AV_PIX_RGB24 = 3`，`AV_PIX_I420 = 4`。
- `av_memory_type`：`AV_MEM_UNKNOWN = 0`，`AV_MEM_HOST = 1`，`AV_MEM_PLATFORM_SURFACE = 2`。
- `av_opaque_kind`：`AV_OPAQUE_NONE = 0`，`AV_OPAQUE_CVPIXELBUFFER = 0x1001`，`AV_OPAQUE_DMABUF = 0x2001`。
- `av_result_kind`：`AV_RESULT_ALARM = 1`，`AV_RESULT_SELF_TEST = 2`，`AV_RESULT_RECOGNITION = 3`。

### 2.2 核心结构体与固定尺寸要求
1. **`av_frame_desc` (152 字节)**：
   - 字段：`size: u32` (0), `api_version: u32` (4), `frame_id: u64` (8), `wall_time_ns: i64` (16), `pts_ns: i64` (24), `modifier: u64` (32), `offset: [u64; 4]` (40), `opaque: *mut c_void` (72), `frame_token: *mut c_void` (80), `platform_tag: u32` (88), `opaque_kind: u32` (92), `memory_type: u32` (96), `pixel_format: u32` (100), `layout: u32` (104), `width: u32` (108), `height: u32` (112), `alloc_width: u32` (116), `alloc_height: u32` (120), `stride: [i32; 4]` (124), `color_primaries: u16` (140), `color_transfer: u16` (142), `color_matrix: u16` (144), `color_range: u8` (146), `plane_count: u8` (147), `time_synced: u8` (148), `reserved: [u8; 3]` (149)。
2. **`av_algo_library_args` (48 字节)**：
   - `size: u32`, `api_version: u32`, `package_root: *const c_char`, `platform_id: *const c_char`, `platform_tag: u32`, `log: Option<av_log_fn>`, `log_user: *mut c_void`。
3. **`av_algo_library_info` (200 字节)**：
   - `size: u32`, `api_version: u32`, `algorithm_id: [c_char; 64]`, `version: [c_char; 32]`, `algorithm_type: [c_char; 32]`, `alarm_type_id: [c_char; 64]`。
4. **`av_algo_instance_args` (96 字节)**：
   - `size: u32`, `api_version: u32`, `mode: u32`, `reserved0: u32`, `instance_id: *const c_char`, `instance_run_id: *const c_char`, `config_json: *const c_char`, `config_json_len: u32`, `reserved1: u32`, `frame_ops: *const av_frame_ops`, `image_ops: *const av_image_ops`, `on_result: Option<av_algo_result_cb>`, `result_user: *mut c_void`, `rules: *const av_rule`, `rule_count: u32`。
5. **`av_algo_abi` 虚函数表 (96 字节)**：
   - `size: u32`, `api_version: u32`
   - `library_open: Option<unsafe extern "C" fn(...) -> i32>`
   - `library_query: Option<unsafe extern "C" fn(...) -> i32>`
   - `library_close: Option<unsafe extern "C" fn(...) -> i32>`
   - `instance_create: Option<unsafe extern "C" fn(...) -> i32>`
   - `instance_negotiate: Option<unsafe extern "C" fn(...) -> i32>`
   - `instance_update_config: Option<unsafe extern "C" fn(...) -> i32>`
   - `instance_set_rules: Option<unsafe extern "C" fn(...) -> i32>`
   - `instance_process: Option<unsafe extern "C" fn(...) -> i32>`
   - `instance_flush: Option<unsafe extern "C" fn(...) -> i32>`
   - `instance_destroy: Option<unsafe extern "C" fn(...) -> i32>`
   - `last_error: Option<unsafe extern "C" fn(...) -> i32>`
6. **`av_algo_result` (48 字节)** 与 **`av_algo_image_req` (32 字节)**。
7. **`av_face_extract_input` (40 字节)** 与 **`av_face_extract_output` (67892 字节)**。

### 2.3 内存布局双向断言 (`crates/infer/tests/c_abi_layout_tests.rs`)
通过 `static_assertions` 与 `std::mem::offset_of!` 建立 100% 覆盖的测试用例：
```rust
assert_eq!(std::mem::size_of::<av_frame_desc>(), 152);
assert_eq!(std::mem::offset_of!(av_frame_desc, frame_token), 80);
assert_eq!(std::mem::offset_of!(av_frame_desc, stride), 124);
assert_eq!(std::mem::offset_of!(av_frame_desc, color_primaries), 140);
assert_eq!(std::mem::size_of::<av_algo_abi>(), 96);
assert_eq!(std::mem::size_of::<av_algo_library_info>(), 200);
assert_eq!(std::mem::size_of::<av_algo_instance_args>(), 96);
```

---

## 3. 平台感知拓扑与七步沙箱校验器 (`AlgoSandbox`)

### 3.1 平台 ID 获取与精简规范
系统在编译/运行时自动获取当前平台的精炼代号：
- macOS Apple Silicon：`macos-arm64`（兼容别名：`macos-arm64-coreml`, `darwin-arm64`）
- Linux Rockchip RK3588：`linux-rknn`（兼容别名：`linux-arm64-rknn`, `rknn`）
- Linux Ascend CANN：`linux-ascend`（兼容别名：`linux-arm64-ascend`, `ascend`）
- 通用 CPU 回退：`linux-x64` / `darwin-x64`

系统通过 `normalize_platform_id` 函数进行别名归一化，既保持目录与配置短小规整，又 100% 兼容历史已打包算法包。

### 3.2 七步沙箱流程 (7-Step Validation Pipeline)
```
  [算法包目录/ZIP]
         │
  1. 路径与结构校验 (Path Traversal Check, 必须含 manifest.json 与 lib/)
         │
  2. SHA256 完整性 (可选验证 checksum)
         │
  3. Manifest 元数据核验 (manifest_version == 1, platform_id 必须匹配当前平台)
         │
  4. Config Schema 校验 (合法 JSON 格式)
         │
  ┌──────▼─────────────────────────────────────────────────────────────┐
  │ 🚨 物理子进程沙箱隔离执行 (Subprocess Worker Sandbox)              │
  │ (通过单二进制自调用: ./heimdall __verify-algo <pkg_dir> 隔离执行)     │
  │                                                                    │
  │ 5. dlopen 与符号寻址 (libloading 动态加载, 寻址 "av_algo_get_abi") │
  │ 6. 虚表尺寸与版本断言 (abi.size == 96, api_version == 1)           │
  │ 7. 真实前向自检 (Self-Test: 解码 testimage.jpg 跑前向推理)        │
  │                                                                    │
  │ 🛡️ 若第三方库发生 SIGSEGV / 内存越界 / 死锁，仅子进程挂掉，         │
  │    主服务毫发无损，超时 10s 强杀 (kill -9)，安全捕获退出状态码!    │
  └──────┬─────────────────────────────────────────────────────────────┘
         │
  [子进程安全返回 0 且输出结构化自检通过指标 -> 主进程热注册到 AlgoRegistry]
```

### 3.3 物理子进程沙箱隔离自检机制 (Subprocess Sandbox Worker)
- **故障域隔离 (Fault Domain Isolation)**：
  - Rust 语言的 `catch_unwind` 仅能捕获语言内部 panic，无法拦截 C/C++ 引发的段错误（`SIGSEGV`）、非法指令（`SIGILL`）或硬件总线错误（`SIGBUS`）。
  - 为确保主进程（实时流推流、已有摄像头 AI 分析任务）绝对不崩溃，安装校验环节强制派生子进程执行：
    ```rust
    let mut child = std::process::Command::new(std::env::current_exe()?)
        .arg("__verify-algo")
        .arg(package_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    ```
- **超时与看门狗 (Watchdog & Kill)**：
  - 主进程为子进程配置 10 秒超时看门狗；若超时未完成推理，直接调用 `child.kill()` 发送 `SIGKILL` 强杀，彻底阻断死循环。
- **真实前向自检推理 (Self-Test Execution)**：
  - 子进程内部载入 `testimage.jpg`（约 137KB 标准自检图片）；
  - 使用 `image` crate 解码为 RGBA/BGRA 或 NV12 内存平铺，构造有效的 `av_frame_desc`；
  - 配置 `av_algo_instance_args`，设置 `mode = AV_INSTANCE_INSTALL_SELF_TEST`；
  - 注入结果回调函数 `on_result`，收集解析出的 `av_algo_result` JSON；
  - 触发 `instance_process`，断言返回状态码为 `AV_OK`（0）；
  - 验证回调结果 JSON 能够反序列化为至少 1 个目标检测结果（例如 `person`、`dog` 等，置信度 $> 0.25$）；
  - 验证通过后向 stdout 打印 `{"status": "ok", "detections": N}` 并退出（exit code 0）。
  - 主进程解析 stdout，确认自检通过后在主进程内部正式注册该算法包。

---

## 4. 运行期 RAII 资源管理与线程安全抽象

### 4.1 安全封装体系
1. **`LoadedLib`**:
   - 持有 `libloading::Library` 以及指向 `av_algo_abi` 的引用；
   - 保证动态库在所有实例和上下文使用期间不被卸载。
2. **`AlgoPackage` (Arc<AlgoPackageInner>)**:
   - 代表已通过沙箱自检并被宿主打开的算法包；
   - 持有 `av_algo_library` 句柄；
   - 实现 `Drop`：在安全层触发 `abi.library_close(raw_lib)`。
3. **`AlgoInstance`**:
   - 代表某路摄像头运行的算法实例；
   - 持有 `av_algo_instance` 句柄以及对 `LoadedLib` 的生命周期依赖；
   - 实现 `InferenceBackend` trait：
     ```rust
     #[async_trait]
     impl InferenceBackend for AlgoInstance {
         fn name(&self) -> &'static str { "C-ABI-AlgoInstance" }
         async fn detect(&self, frame: &FrameRef) -> Result<Vec<Detection>, InferError>;
     }
     ```
   - 内部将 `FrameRef` 转换为 `av_frame_desc`，通过跨语言通道等待单帧推理回调结果；
   - 实现 `Drop`：调用 `abi.instance_destroy(raw_inst)`。

### 4.2 线程安全与并发保证
- `av_algo_abi` 的函数均为 C 导出，通常底层一个实例绑定一个推理线程（或内部线程安全）。
- `AlgoInstance` 实现 `Send`，在每次调用 `instance_process` 时，若底层不保证实例重入安全，安全层封装 `tokio::sync::Mutex` 或通过专用工作线程调度。
- C 回调转 Rust：通过 `user_data` 传入通道的发送端指针（或每个请求的 oneshot sender），结果由 Rust 接收后完成类型安全的反序列化与坐标变换。

---

## 5. 错误处理与 Unsafe 隔离准则

1. **Unsafe 极度收敛**：所有裸指针解引用、C 函数指针调用严格限定在 `crates/infer/src/c_abi/` 内部。
2. **Panic 屏障**：所有跨 FFI 的反向回调（如 `log` 回调、`on_result` 回调）必须用 `std::panic::catch_unwind` 严格包裹，坚决不允许 Rust panic 逃逸至 C 动态库。
3. **C 错误码映射**：封装 `check_algo_status(code, &abi, inst)` 函数，统一将负数状态码映射为详尽的 `InferError::CAbiError { code, message }`。
