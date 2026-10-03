# 算法 SDK 与 C ABI

适用于 `crates/algo-sdk`、`crates/infer` 插件适配层和 `algo-packages/`。插件不依赖 `types/infer/pipeline/db` 等宿主业务 crate。

## 构建边界

- Heimdall 根 workspace 只构建宿主 crate 与 `algo-sdk`；根 `Cargo.toml` 不列出任何具体算法包。
- `algo-packages/` 按平台和硬件运行时拆分为 `macos`、`rknn/rk3568`、`rknn/rk3576`、`rknn/rk3588` 四个独立 workspace，各自维护成员、算法侧依赖版本、锁文件和构建缓存。算法包通过各自 workspace 的相对路径依赖使用 `crates/algo-sdk`，不通过宿主业务 crate 反向依赖运行时。
- 算法包构建、测试、格式化和交叉编译均以目标平台 workspace 的 manifest 为入口；交付前生成的 `.so/.dylib` 复制到包内 `lib/`，再由 Makefile 打成归档。
- 包内 `lib/` 的插件库是本地构建产物，**不入版本库**（`algo-packages/` 各级 `.gitignore` 已覆盖 `*.so` / `*.dylib`）：版本库只承载 manifest、config schema、模型与源码。从干净检出开始时必须先在该包 workspace 执行 `make` / `make package` 生成 `lib/`（归档同样不入库）；否则宿主启动自愈扫描会因缺少插件库而无法自检装载该内置包，依赖真实包的沙箱测试也只会静默跳过。
- 多个平台 workspace 可以共享 SDK 源码，但不共享 Cargo 的成员集合、锁文件或构建缓存。宿主只在运行时通过 manifest 和 C ABI 动态加载算法制品。

## 权威定义

| 内容             | 源文件                                                                                                                                                          |
| -------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| SDK / 宿主 C ABI | [c_abi.rs](../../../../crates/algo-sdk/src/c_abi.rs)、[types.rs](../../../../crates/infer/src/c_abi/types.rs)                                                       |
| 插件 trait / 导出宏 | [plugin.rs](../../../../crates/algo-sdk/src/plugin.rs)、[macros.rs](../../../../crates/algo-sdk/src/macros.rs)                                                      |
| 插件日志桥接 | [logging.rs](../../../../crates/algo-sdk/src/logging.rs)（`HostLogBridgeLease` RAII 持有权 + 自装 `tracing` Subscriber；级别映射与宿主 `default_c_logger` 双向绑定） |
| 配置宏与三级优先级 | [config.rs](../../../../crates/algo-sdk/src/config.rs)（`algo_config!`、`FromEnvValue`，自动实现显式参数追踪与三级优先级覆盖） |
| Composable 模型生态 | [models](../../../../crates/algo-sdk/src/models/mod.rs)（`YoloSpec`、`YoloDecoder` 解码策略、`GenericYoloDetector` 15行开箱即用） |
| 硬件预处理流水线 | [cv::transforms](../../../../crates/algo-sdk/src/cv/transforms.rs)（`Transform`、`HwLetterbox` 纯设备侧零拷贝变换） |
| 异构推理运行时与零拷贝 | [runtime](../../../../crates/algo-sdk/src/runtime/mod.rs)（`NpuSession`、`RuntimeSession` 跨芯片抽象，Rockchip RKNN 平台驱动位于 `runtime/platforms/rockchip.rs`） |
| 硬件回退策略与可用性判定 | [runtime/fallback.rs](../../../../crates/algo-sdk/src/runtime/fallback.rs)（`FallbackPolicy`、`HardwareStatus`/`HardwareAvailability`、`resolve_fallback_policy`、`platform_requires_hardware`） |
| 本地调测与基准评测 | [testing.rs](../../../../crates/algo-sdk/src/testing.rs)（`LocalPluginRunner`、`BenchmarkStats`、`MockEmitter`） |
| 帧 / 预处理 / 模型会话 | [frame.rs](../../../../crates/algo-sdk/src/frame.rs)、[cv](../../../../crates/algo-sdk/src/cv/mod.rs)、[model.rs](../../../../crates/algo-sdk/src/model.rs)             |
| 后处理工具库 | [cv::postprocess](../../../../crates/algo-sdk/src/cv/postprocess/mod.rs)（quantize / dfl / yolov8_rknn）                                                  |
| 目标跟踪算法库 | [track::bytetrack](../../../../crates/algo-sdk/src/track/bytetrack.rs)（纯 Rust ByteTrack、卡尔曼滤波与匈牙利最优二分图匹配）                                     |
| 人脸视觉几何库 | [face](../../../../crates/algo-sdk/src/face/mod.rs)（Umeyama 相似变换、ArcFace 模板对齐、五点质量姿态拓扑几何）                     |
| 结果 / 向量数学库   | [emitter.rs](../../../../crates/algo-sdk/src/emitter.rs)、[math.rs](../../../../crates/algo-sdk/src/math.rs)（IoU、NMS、余弦相似度、L2 归一化、Base64 编码）          |
| 加载 / 沙箱 / 注册表  | [loader.rs](../../../../crates/infer/src/c_abi/loader.rs)、[sandbox.rs](../../../../crates/infer/src/sandbox.rs)、[package.rs](../../../../crates/infer/src/package.rs) |

完整声明以双侧源码和布局测试为准；以下保留调用约束，不复制结构体实现。

## 状态与生命周期

- 一路流可绑定多个算法实例；同一实例任一时刻只服务一路流，固定绑定独立 Worker，线程数量受启动配置约束。
- 插件可维护局部 ByteTrack、BestShot、时序缓存；Pipeline 维护全局航迹和规则。两者内部状态/track ID 互不耦合。
- 帧和结果经有界通道传递，同一实例串行调用；内部状态只在所属线程访问。

```text
get_abi → library_open/query → instance_create → process/update/negotiate
        → flush（重置、停止或销毁前）→ instance_destroy → library_close
```

`instance_flush(inst) -> c_int` 可选：有状态插件应排空滞留结果并重置时序，无状态插件可使用默认空实现；宿主对未提供的能力跳过调用。
关闭库前销毁全部实例，库句柄与回调资源必须覆盖实例有效期。禁止把“提供 flush 函数”当作“宿主已接入所有 flush 时机”，见文末差异。

## 导出与插件接口

插件以 `cdylib` 交付（`.so/.dylib`），使用宏导出标准 `av_algo_get_abi`：

```rust
export_algo!(
    MyPlugin,
    algo_id: "my_algorithm",
    version: "1.0.0",
    algo_type: "object_detection",
    alarm_type_id: "object_detect"
);
```

可选 `library_open_hook` / `library_close_hook` 管理库级资源；导出 `algorithm_id` 必须与 manifest 一致。

| `AvAlgoAbi` 方法                                                  | 要求              |
| --------------------------------------------------------------- | --------------- |
| `library_open/query/close`、`instance_create/process/destroy`    | 必填，缺任一函数指针拒绝加载  |
| `instance_negotiate/update_config/set_rules/flush`、`last_error` | 按能力处理，不假定可选方法存在 |

- ABI 版本为 `AV_ALGO_API_VERSION = 1`，64 位 `AvAlgoAbi` 大小为 96 字节。
- 每个 C ABI 入口隔离 panic，失败返回 `AV_ERR_INTERNAL`（panic 隔离规则见 [全局约定](../../guides/conventions.md#防御性错误处理)）。
- `AlgoPlugin: Sized + Send + 'static`，配置为 `DeserializeOwned + Default`；必需实现 `init(ctx, config)` 和同步 `process(SafeFrame, &mut ResultEmitter)`。
- `InitContext` 提供 `package_root/platform_id/instance_id/is_self_test/fallback_policy_override`。默认 `flush/set_rules` 返回成功，`update_config` 返回 `NotImplemented`，不能误报配置已应用。
  `fallback_policy_override` 在 `export_algo!` 展开处**恒为 `None`**，硬门与优先级契约见 [硬件回退策略](#硬件回退策略-fallbackpolicy)。

## 插件日志桥接 (`logging`)

宿主在 `library_open` 时通过 `AvAlgoLibraryArgs.log` / `log_user` 提供 C 日志回调（宿主实现见 `crates/infer/src/c_abi/loader.rs` 的 `default_c_logger`）。`algo-sdk` 负责把插件内的 `tracing` 事件转发过去。

### 为何必须由插件自装 subscriber

算法包以 `cdylib` 交付，拥有**独立 workspace 与独立 lock**，`tracing-core` 被**静态链入每个 `.so` 的私有副本**且不导出任何 tracing 符号。

导出符号实测（`nm -gU`，release 构建）：

| 算法包 | `av_algo_*` 导出符号 | 导出的 tracing 符号 |
| --- | --- | --- |
| `face_recognition`（rk3568 / rk3588） | `av_algo_get_abi`、`av_algo_extract_face`、`av_algo_get_gallery_abi`、`av_algo_gallery_bulk` | 0 |
| `face_recognition`（rk3576） | `av_algo_get_abi`、`av_algo_extract_face` | 0 |

> 人脸包额外导出 gallery 两符（`macros.rs` 的 `export_face_gallery!`）；**不得**把符号清单写成“仅两个”，那只对未导出 gallery 的包成立。结论只依赖「tracing 符号数为 0」这一点。

因此：

- 宿主的 `tracing_subscriber::registry().init()` 只作用于**宿主自己的** dispatcher，**不可**被插件复用；
- 插件若不自行安装 subscriber，其 `tracing` 事件会回落到 no-op sink 并被静默丢弃。

**隔离性已实测**（2026-10-01）：两个各自独立 workspace 构建的 cdylib，各自调用 `install_host_log_subscriber()` **均返回成功**（若全局状态共享，第二个必失败），各发 3 条事件后 `alpha=3, beta=3`，**无串流**。因此一个进程内加载多个算法包时，每个包只能看到自己的事件。

### 契约

| 项 | 契约 |
| --- | --- |
| 触发 | `library_open` 且 `AvAlgoLibraryArgs.log` 为 `Some` 时登记并安装；`None` 时保持既有行为（不安装、不登记） |
| 级别映射 | TRACE→0 / DEBUG→1 / INFO→2 / WARN→3 / ERROR→4。**与宿主 `default_c_logger` 是双向契约**，改任一侧必须同步另一侧与其表驱动测试 |
| 消息格式 | 必须是 **NUL 结尾** 缓冲：宿主用 `CStr::from_ptr` 读取并**忽略 `len`**；`len` 仍按契约填不含终止符的字节数 |
| 内部 NUL | 必须替换（当前用 U+FFFD），否则宿主 `CStr` 读取会静默截断后半段 |
| 字段 | `message` 之外的结构化字段以 `key=value` 追加。算法包诊断普遍形如 `warn!(reason = ?e, "...")`，丢弃字段会失去诊断价值 |
| 共存 | `set_global_default` 失败（已安装全局 subscriber、插件自身 `fmt().init()`）必须**忽略而非 panic**，不得覆盖已有 dispatcher |
| 线程安全 | `AvLogFn` 必须可在**任意线程**上被调用且可重入并发；插件转发可能发生在推理 Worker 等任意工作线程。宿主须自行加锁——`default_c_logger` 据此设计（无共享可变状态） |
| 不做 target 过滤 | cdylib 拥有**私有** `tracing-core` 副本，本订阅器只能看到本插件自身发出的事件，宿主事件不会进入该 dispatcher，因此「劫持宿主事件」不成立（`design.md` §3.3 的过滤项因此无对象） |
| 依赖 | 不得为此引入 `tracing-subscriber` 或新增任何依赖：`tracing` 已 re-export 全部所需 core 类型（`Subscriber`/`Event`/`Metadata`/`Level`/`span::Id`/`field::Visit`） |

### 交付体积（实测）

rk3588 人脸包 release 构建，`nm -gU` 导出符号集完全一致（含 `av_algo_get_gallery_abi` / `av_algo_gallery_bulk`），导出 tracing 符号数为 0：

| 版本 | `.dylib` 字节数 | 差值 |
| --- | --- | --- |
| 引入桥接前 | 2212048 | — |
| 引入桥接后 | 2240368 | **+28320（+27.7 KiB，+1.28%）** |

> 不得再写“体积零变化”。零变化的是**依赖图**（`Cargo.lock` 无改动），不是产物大小：`logging.rs` 的转发实现会链入每个 cdylib。

### 生命周期：持有权必须是 RAII，禁止在 `library_close` 手工配对

> **Warning**：`library_close` 在**每次** `RawAlgoLibrary::drop` 都触发，而 `RawAlgoLibrary::open` 有长期与短命两类调用点：常驻 Worker **长期持有**一个句柄（`package.rs` 的 `create_worker`），`extract_face` / `create_gallery` 则另开短命句柄、用完即关。
>
> 若 `library_close` 无条件停用日志桥接，一次 `extract_face` 就会把常驻 Worker 的后续日志**静默掐断**——正是本能力要消灭的缺陷模式。

但不能因此把释放逻辑写在 `library_close` 里：

> **Warning**：`library_open` 存在**不产生库句柄**的失败返回路径——`open_hook` 返回 `Err`（如 `shared_models()` 的 `ModelLoad` 失败）或在其内部 panic。这些路径下宿主拿不到句柄，因此**不会**调用 `library_close`（`crates/infer/src/c_abi/loader.rs` 只在 `raw_lib` 非空时关闭）。手工配对会让每次失败加载永久泄漏一个计数。

→ **持有权必须是 RAII 的**（[`HostLogBridgeLease`](../../../../crates/algo-sdk/src/logging.rs)）：`register_host_log_sink` 返回持有权，随 `LibraryContext` 一同析构，正常返回、失败返回与 unwind 三条路径全部自动配对。持有权是**零尺寸**类型，不复制回调负载。

契约：

1. `library_open`（且 `log` 为 `Some`）登记并取得一个持有权；`log` 为 `None` 时不登记、不取持有权；
2. 持有权随 `LibraryContext` 析构自动释放——**不得**在 `library_close` 里手工递减；
3. 活跃性由计数**单一派生**（`count > 0`），**不另设 `AtomicBool`**：两个独立状态源之间存在丢失更新窗口（`release` 减到 0 与 `register` 并发时，`store(false)` 可能落在 `store(true)` 之后），会造成「计数为 1 但桥接已关」；
4. 计数为 0 时递减必须**饱和**（`checked_sub`），否则会回绕成天文数字而永不归零；
5. 回调永不释放（所有权属于宿主，且宿主始终传同一 `default_c_logger`）。

该手法与上文「进程级默认引擎必须显式回收」及 `DefaultEngineLease` 同源。**通用规则**：凡是挂在 `library_open` 上的进程级资源，都必须按**库句柄**引用计数，且必须用 RAII 而非 `library_close` 钩子配对——因为并非每次成功的登记都会走到 `library_close`。

### 验证与错误矩阵

| 条件 | 期望行为 |
| --- | --- |
| `log = None` | 不安装、不登记、不计数；其 `library_close` 也不得递减（否则会扣掉另一个存活句柄的配额） |
| `library_open` 重复调用 | 幂等（`OnceLock` 首次写入生效）；计数按调用次数递增 |
| **`open_hook` 返回 `Err`** | 持有权随 `LibraryContext` 析构释放；**不得**留下计数（这些路径没有 `library_close`） |
| **`open_hook` 内部 panic** | 同上一行：`catch_unwind` 展开时持授权随栈帧析构 |
| 全局 subscriber 已被占用 | 安装返回 `false`，不 panic，不覆盖 |
| 宿主回调 panic | **进程 abort（SIGABRT），不可隔离**。`AvLogFn` 是 `extern "C"`，回调内 panic 会在其自身的不可 unwind 守卫处直接终止进程，外层 `catch_unwind` 无法介入。宿主侧的 `default_c_logger` 已将函数体包在 `catch_unwind` 内，隔离责任在宿主 |
| 消息含内部 NUL | 替换为 U+FFFD，不 panic、不使宿主越界读 |
| 未配对的释放 | 计数饱和于 0，不回绕、不 panic |
| 常驻句柄 + 短命句柄 | 短命句柄关闭后桥接**继续活跃**，直到最后一个句柄释放 |
| 先 `log = Some` open、后 `fmt().init()` | 后者 panic（全局订阅器已被占用）。仓库内无此类二进制；探针走 rlib、不调 `library_open` |

### 必测项（断言点）

单元测试（`crates/algo-sdk/src/logging.rs`）：

- `registered_callback_receives_event_content` — 事件内容与结构化字段真实到达回调，且宿主读取位置为终止符；
- `level_mapping_matches_host_contract` — 五级映射表驱动；
- `message_is_nul_terminated_and_interior_nul_is_sanitized` — NUL 语义与内部 NUL 替换；
- `inactive_bridge_is_a_noop` — 停用后退化为无操作；
- `none_callback_registers_nothing` — `log = None` 不登记、不计数；
- `install_is_idempotent_and_never_panics` — 共存不 panic；
- `lease_carries_no_callback_payload` — 持有权零尺寸（不复制回调负载）；
- `lease_drop_releases_exactly_one_holding` — **回归锁**：持有权在析构时释放，不依赖 `library_close`；
- `bridge_stays_active_until_last_handle_released` — 模拟「常驻句柄 + 短命句柄」，短命句柄关闭不得停用桥接；
- `release_without_register_saturates_at_zero` — 未配对释放不回绕。

端到端（走真实 C ABI 虚表）：

- `plugin_lifecycle.rs::test_library_open_bridges_host_log_callback` — 经 `library_open` 注册后插件内 `tracing::warn!` 到达回调；`library_close` 后不再回调；
- `plugin_open_failure.rs::failed_open_does_not_leak_log_bridge` — **P1 回归锁**：`open_hook` 失败后不得留下持有权（含反复失败不累积）；
- `plugin_open_failure.rs::none_log_close_does_not_decrement_other_handles` — `log = None` 的 `close` 不得递减其他句柄的配额。

> **判别力（已实测变异验证）**：
>
> - 把 `register_host_log_sink` 的返回值 `mem::forget`（精确复现修复前的手工配对语义），`failed_open_does_not_leak_log_bridge` 以「失败 open 不得留下持有权」`left: 1, right: 0` 失败，`none_log_close_does_not_decrement_other_handles` 以「全部句柄关闭后必须停用」失败；
> - 把 `release_host_log_bridge` 改为无条件停用，`bridge_stays_active_until_last_handle_released` 会以「还有库句柄存活时不得停用桥接」失败；
> - 去掉 `library_open` 中的登记，`test_library_open_bridges_host_log_callback` 会以 `left: 0, right: 1` 失败。
>
> 新增同类用例时，请优先断言**具体取值**而不是「没有崩溃」。

### 测试编写约束（踩过的坑）

> **Warning**：`HOST_LOG_SINK` 是 `OnceLock`（set-once，与生产语义一致）。**同一测试二进制内所有用例必须登记同一个回调**——若某个用例登记了别的回调，谁先跑到就永久胜出，其余用例的断言静默失真且难以复现。
>
> 实测过一次违反此约束的后果：一个登记「会 panic 的回调」的用例，约 1/190 概率抢先写入槽位，使后续任意用例在转发时进入该回调 → 因 `extern "C"` 不可 unwind → **SIGABRT 崩掉整个测试进程**（退出码 134，且没有任何断言失败信息，极易误判为环境问题）。
>
> 因此**不要**再添加「回调 panic 是否被隔离」这类用例：它在 `extern "C"` 下不可断言（见上表）。

### 常见错误

**错误**：宿主提供了 log 回调，就以为插件日志自动可观测。

**症状**：`crates/infer` 侧收不到任何插件诊断，降级/失败/硬件异常全部无声。

**根因**：`LibraryContext` 早期版本从不读取 `AvAlgoLibraryArgs.log`，且 `algo-sdk` 内 0 处 `tracing_subscriber`——宿主想收、插件有得发，中间的桥是断的。

**修复**：由 `library_open` 登记回调并自装 subscriber（见上）。

**预防**：插件侧出现「应当可见但看不见」的日志时，先确认 `logging` 桥接是否登记成功，而不是先怀疑日志级别。

## 帧契约

64 位 `AvFrameDesc` 为 **120 字节、8 字节对齐**，包含版本头、frame ID、PTS、有效/分配尺寸、格式/句柄、stride 和 offset。
当前颜色矩阵与范围编码在 `color_space`，没有旧示例的独立 `color_range` 字段；字段顺序和偏移只引用布局测试。

- 宿主 `FrameRef.timestamp` 为 UTC 毫秒，ABI `pts_ns` 为纳秒，换算只发生在适配边界（1ms = 1,000,000ns），校验范围，不能改 ABI 单位。
- `SafeFrame::from_raw_checked` 是 unsafe 边界：调用方先保证 ABI 头可读且对齐，声明尺寸通过后完整结构体/底层内存在调用期有效。
- 校验 `size >= sizeof(AvFrameDesc)`、版本匹配、非零有效宽高、非零分配尺寸不小于有效尺寸、支持的像素格式和句柄。
- stride 不得为负；非零 stride 满足像素格式行宽，offset/平面范围不重叠且长度计算不溢出。零值按现有默认布局语义处理。
- 算法按 stride/offset/alloc 尺寸寻址，不以可见 width 替代行跨度。
- `SafeFrame` 只借用当前调用期的帧；需要延长生命周期时走 `AvFrameOps.retain/release`，不保存裸指针。`frame_token` 由宿主管理。

| `opaque_kind`                    | 值        | 载体                         |
| -------------------------------- | -------- | -------------------------- |
| `AV_OPAQUE_NONE`                 | `0`      | Host 像素指针，调用方保证内存范围有效      |
| `AV_OPAQUE_CVPIXELBUFFER`        | `0x1001` | CVPixelBufferRef           |
| `AV_OPAQUE_DMABUF`               | `0x2001` | fd 编码到指针宽度整数；fd 0 不等于无效 fd |
| `AV_OPAQUE_ASCEND_DEVICE_MEMORY` | `0x3001` | Ascend 设备指针                |

像素枚举为 `NV12=1`、`BGRA=2`、`RGB24=3`、`I420=4`，使用 `AV_PIX_*` 常量。
`AvFrameCaps` 的格式/内存数组最多 8/4 项；默认 `instance_negotiate` 透传 offered→accepted，不代表已校验所有硬件约束。

## 状态码

| 状态                          | 值   | 含义         |
| --------------------------- | --- | ---------- |
| `AV_OK`                     | 0   | 成功         |
| `AV_ERR_UNSUPPORTED_API`    | -1  | 版本不支持      |
| `AV_ERR_INVALID_ARG`        | -2  | 参数/预处理输入无效 |
| `AV_ERR_INCOMPATIBLE_FRAME` | -3  | 帧不兼容       |
| `AV_ERR_CONFIG_INVALID`     | -4  | 配置错误       |
| `AV_ERR_MODEL_LOAD_FAILED`  | -5  | 模型加载失败     |
| `AV_ERR_INFERENCE_FAILED`   | -6  | 推理失败       |
| `AV_ERR_OUT_OF_MEMORY`      | -7  | 资源不足       |
| `AV_ERR_NOT_IMPLEMENTED`    | -8  | 能力未实现      |
| `AV_ERR_TIMEOUT`            | -9  | 超时         |
| `AV_ERR_RETRY`              | -10 | 可重试        |
| `AV_ERR_INTERNAL`           | -99 | 内部错误/panic |

`AlgoError` 映射见 [error.rs](../../../../crates/algo-sdk/src/error.rs)；`last_error` 使用线程局部错误缓存提供详情，安全层不传播 C 整数错误码。

> **Warning：`last_error` 的实际可见长度受宿主读取缓冲限制**。宿主 `check_c_status` 用 **512 字节**栈缓冲
> （`crates/infer/src/c_abi/loader.rs`），`copy_last_error` 按 `min(len, cap - 1)` 截断，超长尾段会被静默丢弃。
> 因此写入 `last_error` 的错误消息必须把**可操作指引排在前面**、最易截断的底层原因放最后；
> 详细契约与回归锁见 [硬件回退策略](#硬件回退策略-fallbackpolicy)。

## 可选人脸提取

`av_algo_extract_face` 是独立可选符号，不扩展 `AvAlgoAbi` 虚表。宿主通过 `libloading` 探测，缺失时视为不支持。

```rust
unsafe extern "C" fn(
    lib: AvAlgoLibrary,
    input: *const AvFaceExtractInput,
    output: *mut AvFaceExtractOutput,
) -> c_int;
```

| 参数            | 契约                                                                                                  |
| ------------- | --------------------------------------------------------------------------------------------------- |
| `lib`         | 来自本插件 `library_open`，同步调用期有效                                                                        |
| input（24 字节）  | `size/api_version/image_bytes/image_bytes_len`；当前接受压缩图像字节，非裸 RGB/实时帧描述符；上限 32 MiB                   |
| output（56 字节） | 调用方提供完整可写 ABI 结构并初始化版本头；返回 `status_code`、embedding 指针/维度、aligned JPEG 指针/长度、quality/detection score |

- 输出向量和 JPEG 借用插件缓存，宿主不释放；在下次提取、线程结束或卸载前复制需要保留的结果，不能把缓存指针交给异步消费者。
- 成功 `status_code=0`；失败遵循返回码，不解引用错误结果。空指针、尺寸/版本错误、不可解码图像必须明确失败。
- 阈值属于插件配置职责，ABI 只传数据；实例配置与库级离线提取的关联不能靠假设。
- `quality` score 是**包内**融合与筛选用的原始加权分（尺寸/姿态/关键点/清晰度），语义、尺度与阈值全部归算法包；宿主只把它当排序键，**不得**以其绝对尺度决定证据留存或记录落库（证据存在性优先于画质）。检测器换代或参数调整导致分数整体漂移时，宿主侧行为不得随之变化。
- 这是低频离线能力，允许 CPU 解码/读回，不作为常驻推理的 CPU 回退借口。
- 权责边界：本接口专门服务于人员管理与底库录入（`PersonnelService`）及全景图特征重提取，严禁作为视频流实时抓拍对账的逆向降级路径。视频流实时分析由包内全景检测、映射裁剪与时域特征融合独立闭环。
- 当前实现：[macOS](../../../../algo-packages/macos/arm64/face_recognition/src/lib.rs)、[RK3576](../../../../algo-packages/rknn/rk3576/face_recognition/src/lib.rs)。

## 共享人脸底库 C ABI (`AvAlgoGalleryAbi`)

`av_algo_get_gallery_abi` 是独立可选虚表符号，不扩展 `AvAlgoAbi` 基础虚表。宿主通过 `libloading` 动态探测，支持该能力的算法包导出该符号；缺失时宿主回退至内存兜底。

```rust
unsafe extern "C" fn(api_version: u32) -> *const AvAlgoGalleryAbi;
```

### 虚表与数据契约

- ABI 版本为 `AV_ALGO_API_VERSION = 1`，64 位 `AvAlgoGalleryAbi` 大小为 **64 字节、8 字节对齐**。
- 虚表包含 `gallery_create`、`gallery_destroy`、`gallery_clear`、`gallery_insert`、`gallery_remove`、`gallery_search`、`gallery_count` 函数指针。
- 检索候选人 `AvFaceCandidate` 为 **32 字节、8 字节对齐** 的固定布局 POD 结构体（已彻底剥离业务元数据，仅承载纯向量计算输出）：
  - `id` (`u64`) 为样本的全局唯一数字 ID（由宿主生成或对应 SQLite 主键）；
  - `similarity` (`f32`) 为算法包**内部直接标定完成的标准置信分**（`[0.0, 1.0]`），严禁宿主与前端二次标定；
  - `raw_score` (`f32`) 为算法底层原始度量分（如原生余弦相似度）；
  - `rank` (`u32`) 为 Top-K 排序名次（从 1 开始）；
  - `reserved0` (`u64`) 为对齐与未来扩展保留字段。
  - 人员姓名、照片路径等业务元数据全部保留在宿主（`RegisteredFace` / SQLite）侧，通过 `id` 在检索后由宿主做人员聚合与去重，彻底消除 FFI 边界的业务双写一致性负担。

### 并发与内存模型（短读锁与不可变快照）

- [FaceGallery](../../../../crates/algo-sdk/src/face/gallery.rs) 使用 `RwLock<Arc<GallerySnapshot>>` 保存不可变底库快照。
- **计算阶段不持锁，不等于 Wait-Free**：写操作（`insert` / `remove` / `clear`）持排他写锁构造并替换快照；检索操作（`search`）取得读锁、克隆 `Arc` 后释放读锁，再执行点积与 Top-K 排序。已有快照的检索可与后续写入并行，但新检索获取快照时可能等待写锁，不能承诺整个调用无锁或无等待。
- **内存预算包含在途快照**：10,000 张 512D FP32 特征的原始数据约为 20.48 MB，不含容器、检索临时数据与快照副本。写入期间旧快照可能仍被检索持有，不能把单份特征大小当作进程内存上限。
- **宿主单一真实信源（Single Source of Truth）**：持久化数据以宿主 SQLite 为准，算法包底库仅作为运行期内存索引；全量同步使用 `clear` + 批量 `insert`，重建耗时须按底库规模与目标设备测量，不承诺固定毫秒级完成。

### 批量写入符号 `av_algo_gallery_bulk`

全量重建时逐条调 `gallery_insert` 会让包内 RCU 快照被复制 N 次（O(N²) 拷贝），千人级底库重建的 CPU 开销压到宿主启动路径上。因此额外导出一个**独立可选符号**：

```rust
unsafe extern "C" fn(
    gallery: AvAlgoGallery,
    op: u32,               // AV_GALLERY_BULK_INSERT = 1 / AV_GALLERY_BULK_REMOVE = 2
    entries: *const AvGalleryBulkEntry,
    entry_count: u32,
) -> c_int;
```

- **加法式演进，不动虚表**：不修改 `AvAlgoGalleryAbi` 布局（仍为 64 字节），旧宿主忽略该符号、新宿主 `libloading` 探测。缺失时宿主回退到逐条 `insert`/`remove`，语义等价、只是慢；`supports_bulk_write()` 供观测。
- `AvGalleryBulkEntry` 为 **24 字节、8 字节对齐**的固定布局 POD：`id` (0) / `feature_bytes` (8) / `feature_len` (16) / `reserved0` (20)。宿主与算法包**双侧**都要有尺寸、对齐与偏移断言（宿主侧见 `crates/infer/tests/c_abi_layout_tests.rs`）：两侧对同一片内存解引用，对齐不一致会导致读到的 `id` 错位半个指针。

## NPU 放置与权重复用可选扩展 (`AvAlgoPlacementExtensionV1`)

为了支撑宿主协同 NPU 多核分配与卡亲和架构（D1 架构：独占实例 Worker、独立会话与 IO、宿主统筹分核、板端真实物理权重复用），定义了加法式可选扩展虚表符号 `av_algo_get_placement_extension`。

```rust
pub const AV_ALGO_PLACEMENT_EXTENSION_SYMBOL: &[u8] = b"av_algo_get_placement_extension\0";

pub type AvAlgoGetPlacementExtensionFn =
    unsafe extern "C" fn(requested_api_version: u32) -> *const AvAlgoPlacementExtensionV1;
```

### 虚表与 POD 结构布局

- **虚表不破坏基础 `AvAlgoAbi`**：基础 `AvAlgoAbi` 严格保持 64 位 96 字节固定不变；宿主通过 `libloading` 动态查找该符号，缺失时视为该包仅支持单卡/默认运行，维持完全向后兼容。
- `AvAlgoPlacementExtensionV1` 大小为 **64 字节、8 字节对齐**：
  - `size` (`u32`): 结构体自身大小（64 字节）；
  - `api_version` (`u32`): 扩展 API 版本（当前为 1）；
  - `query_capabilities`: 查询算法包对各硬件平台（RKNN/Ascend）的核心绑定与权重复用能力支持；
  - `query_instance_receipt`: 查询实例应用放置策略后的硬件回执（绑定核心掩码、真实分配设备等）；
  - `query_cleanup_receipt`: 查询实例销毁时硬件资源的释放状态；
  - `reserved0..reserved3`: 未来扩展保留函数指针。
- **扩展 POD 结构体**：
  - `AvAlgoPlacementCapsPod` (大小 32 字节，4 字节对齐)：包含 `core_pinning_supported`、`shared_weights_supported`、`preferred_weight_sharing_route` (1=RouteA, 2=RouteB) 等标志；
  - `AvAlgoInstanceReceiptPod` (大小 72 字节，8 字节对齐)：包含 `applied_core_mask`、`runtime_device_index`、`status`、`reservation_id` (32B) 等；
  - `AvAlgoCleanupReceiptPod` (大小 72 字节，8 字节对齐)：包含 `cleanup_status`、`generation`、`reservation_id` (32B) 等；
  - 宿主与 SDK 双侧均有 `size_of`、`align_of` 及关键字段 offset 的单元测试断言。

### Wire 协议与业务插件隔离 (`__heimdall_placement`)

- 宿主在下发给 `instance_create` 的实例 JSON 配置顶层注入私有字段 `__heimdall_placement`；
- **配置剥离保护机制**：`export_algo!` 宏在调用业务插件配置反序列化前，先通过 `deserialize_config_stripping_placement` 将 `__heimdall_placement` 剥离并留存，业务插件无论是否标记 `#[serde(deny_unknown_fields)]` 均可安全解析，杜绝未知字段反序列化失败崩溃；
- 算法包如需获取放置元数据，可由 SDK 内部机制消费或传递给底层 InitContext Builder。
- `AV_GALLERY_BULK_REMOVE` 只读 `id`，`feature_bytes` / `feature_len` 置空；`entry_count == 0`、任一 `feature_bytes` 为空或 `feature_len == 0`（INSERT）均返回 `AV_ERR_INVALID_ARG`。
- 任一条失败即整体返回错误：调用方不得当成「已同步」。宿主在批量失败时按自己的快照走全量重建自愈（见 [API 规范](../../api/backend/api-guidelines.md#真人脸检索降级状态)）。
- 算法包侧由 `export_face_gallery!` 宏展开实现，同样包在 `catch_unwind` 中并设置 `last_error`。

### 导出宏

算法包通过 `export_face_gallery!` 宏一键导出该虚表与批量写入符号，内置 panic unwind 隔离防崩溃保护：

```rust
algo_sdk::export_face_gallery!(FaceRecognizer);
```

## 硬件预处理与会话

宿主向算法实例提供解码后的原生 `FrameRef`/`AvFrameDesc`，不为所有算法强制设定统一模型输入尺寸。当前默认由算法包自行选择预处理尺寸、裁切、色彩格式和归一化方式；若后续启用宿主预处理，算法包必须先通过 `instance_negotiate` 声明可接受的帧能力，宿主再按实例约束执行，不能用单一目标尺寸覆盖所有模型。

- `CvEngine::letterbox/resize` 返回 `(CvBuffer, PreprocessMode)`；宿主 `AvImageOps` 注入优先，未注入时使用平台引擎。
- 平台默认：macOS→AppleCvEngine，Linux+rga→RgaCvEngine，其他→CpuCvEngine；生产 CPU 回退仍受 [三路径边界](../../media/backend/media-pipeline.md#三条路径) 限制。
- `with_engine` 在线程局部 scope 绑定引擎，RAII Guard 在正常/错误/panic 路径恢复栈，不在实例间共享回调表。
- `CvBuffer` 统一管理 Host、DMA-BUF、CVPixelBuffer、Ascend 显存与宿主视图；宿主视图析构调用相应 free，外部句柄必须有明确 guard/释放责任。
- `compute_letterbox_layout` 提供 scale、padding 和缩放尺寸，`unmap_box` 复用同一布局完成逆变换。
- RGA 输出池在初始化时 import handle 并复用；输入 handle 由 `RgaHandleGuard` 单帧管理，禁止每帧重复 import/release 输出池。
- **当前实现与预算差异**：单 `RgaCvEngine` 的缓存上限由 [engine.rs](../../../../crates/algo-sdk/src/cv/platforms/rockchip/engine.rs) 的 `MAX_CACHED_POOLS` 定义，当前为 **64**；满额时已有规格仍可复用，新增规格返回 `AlgoError::Preprocess`。旧规范按 **16** 个规格讨论部署预算，与当前代码不一致；记录 64 仅说明实现上限，不代表已批准扩大资源预算或通过目标板内存验证。现有 `pool_for_enforces_maximum_cached_pools` 测试覆盖满额后拒绝新增规格。堆选择仍优先 system-dma32/system，只有显式 Rga2 强制 DMA32，Auto/Rga3 可使用 64 位物理地址堆。
- **进程级默认引擎必须显式回收**：`cv::default_engine` 把引擎存放在 `static OnceLock`，而 Rust 静态变量永不执行 `Drop`；若不显式回收，RGA 池持有的 DMA-BUF 导入句柄会一直存活到进程退出，由内核强制回收并在 dmesg 留下 `rga_mm: [tgid:N] Destroy handle[M] when the user exits`。契约：
  1. `CvEngine::release_hardware` 负责释放引擎持有的全部硬件资源（RGA 引擎清空并丢弃池表，池引用归零后各 `PoolResource` 析构即归还句柄），必须幂等且对无硬件资源的平台为空实现；
  2. `export_algo!` 展开的 `instance_create` 为每个实例登记 `DefaultEngineLease`，字段声明在 `InstanceContext` **末尾**，确保在 `plugin`/`engine` 之后析构；
  3. 最后一个实例销毁时计数归零并自动调用 `release_default_engine`（饱和递减，计数为 0 时拒绝递减而非回绕）；
  4. 引擎本体保留在静态中，后续实例按需重建缓冲池。
  不要改用 `library_close_hook` 做此事：`library_close` 在**每次** `RawAlgoLibrary::drop` 都触发（含 `extract_face` 等高频短操作），挂在彼处会造成池反复销毁/重建的句柄抖动。同类进程级资源的通用 RAII 持有权契约见 [插件日志桥接 · 生命周期](#生命周期持有权必须是-raii禁止在-library_close-手工配对)。
- **规格预算按引擎实例计数，部署内存按进程汇总**：缓存表属于单个 `RgaCvEngine`，共用该引擎的算法实例共享槽位，运行期不自动淘汰规格；不能据此假定所有动态算法库共用一张缓存表。算法包请求的 RGA 输出几何必须收敛到固定集合，**禁止**把随帧变化的 ROI 尺寸直接作为裁剪尺寸；耗尽槽位后新增规格会持续失败，已有规格仍可复用，直到显式释放缓存。超出档位集合时必须采用已定义的固定尺寸退化策略，不得静默新增规格。
- **几何预算的真实不变量是「单分辨率内有界」，不是「与分辨率无关」**：仅靠「超出档位就退化为整帧该轴尺寸」的策略，每种分辨率仍会贡献与帧尺寸绑定的规格，因此必须核算跨分辨率并集。旧规范记录的 4 种分辨率产生 9 种几何、7 种分辨率产生 12 种几何，仅是当时按 16 槽预算讨论的样例，不能外推为当前 64 槽的容量验证。落地要求：
  1. 档位集合与退化策略必须用**跨分辨率并集**回归测试钉住上限（参考 RK3568 人脸包 `plugin::tests::snapshot_roi_geometry_union_stays_within_process_budget`，含「扫描确实到达退化分支」的饱和校验），不能只断言单分辨率内的档位数；
  2. 部署核对时按「分辨率集合 × 档位并集 + 各算法包 letterbox 规格 + 退化档」核算，为未来算法包预留余量；
  3. 需要彻底解耦时应改用固定画布 + 几何逆变换（源图降采样到定长画布后再采样），但会引入重采样误差，必须先做 embedding 一致性验证，不得为省槽位牺牲识别精度。
  参考 RK3568 人脸包 `SNAPSHOT_ROI_TIERS`（128/192/256/384）及其档位有界性回归测试。
- 单个规格的池 `max_size` 默认 4（`min_idle=2`、`acquire_timeout_ms=50`）：第 5 个并发租约会阻塞至多 50 ms 后失败，调用方不得假设 `acquire` 永不阻塞。
- `SharedWeights<W>` 用 `Arc<W>` 共享权重，`session()` 用 `AtomicUsize` Round-Robin 分配，`session_on(Core)` 显式绑定。
- `Core` 支持 Auto、Id、All、Mask；各实例 session 独占。逻辑共享不代替具体 SDK 的物理内存验证。

## 硬件回退策略 (`FallbackPolicy`)

### 1. 范围 / 触发

任何由算法包自行创建推理会话的路径都受本契约约束：声明式骨架
[`GenericDetector::init`](../../../../crates/algo-sdk/src/models/yolo.rs) 与直连 `RknnSession::open_or_fallback`
的包（当前为 `rk3568/fire-detections`、`rk3576/general_detection`）。

**背景缺陷**：任一平台运行时加载失败（不只是“物理上确无加速单元”，也包括 `package_root` 拼写错误、
ABI/架构不匹配、`.so` 缺失）都会静默降级为 `debug_cpu_fallback_path` 模拟会话，
该会话产出**写死的模拟检测框**。后果是“安装自检通过”无法证明模型真的加载到了硬件——
宿主已有的 `platform_id` / `algorithm_id` 校验与六步沙箱自检全部覆盖不到这一点。
本契约把降级从“隐式默认”改为“显式策略”，并让失败响亮且可定位。

### 2. 签名

```rust
// crates/algo-sdk/src/runtime/fallback.rs
pub const ALLOW_CPU_FALLBACK_ENV_KEY: &str = "ALLOW_CPU_FALLBACK";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FallbackPolicy {
    #[default]
    Allow,             // 无硬件 → 模拟会话（开发/调试；既有默认行为）
    RequireHardware,   // 无硬件 → Err，绝不返回模拟会话
}

pub enum HardwareStatus { Hardware, Simulated }          // 成功构造域内两态
pub enum HardwareAvailability { Hardware, Simulated, Unavailable }  // 含构造失败的归约三态

pub fn classify_session_availability(
    result: &Result<HardwareStatus, AlgoError>,
) -> HardwareAvailability;

pub fn normalize_platform_id(id: &str) -> &str;
pub fn platform_requires_hardware(platform_id: &str) -> bool;

/// 策略解析唯一入口（优先级见 §3）
pub fn resolve_fallback_policy(
    is_self_test: bool,
    platform_id: &str,
    env: Option<&PackageEnv>,
    explicit: Option<FallbackPolicy>,   // 调用方显式声明；生产路径恒为 None
) -> FallbackPolicy;

/// “要求硬件但不可用”的可定位错误（`cause` 为底层根因描述）
pub fn hardware_unavailable_error(package_root: &Path, model_path: &Path, cause: &str) -> AlgoError;
/// 剥掉 `ModelLoad` 的外层 `Display` 前缀，避免嵌套重复占用宿主错误缓冲
pub fn describe_load_failure(error: &AlgoError) -> String;
```

```rust
// crates/algo-sdk/src/runtime/platforms/rockchip.rs
#[non_exhaustive]
pub struct RknnSessionOptions {
    pub core_mask: c_int,
    pub fallback_policy: FallbackPolicy,   // 加法式新增；Default = Allow
}
impl RknnSessionOptions {
    pub fn with_fallback_policy(self, fallback_policy: FallbackPolicy) -> Self;  // 链式，保留 core_mask
}

impl RknnSession {
    pub fn hardware_status(&self) -> HardwareStatus;
    pub fn is_fallback(&self) -> bool;   // 内部委托 hardware_status，外部行为逐位不变
}
```

```rust
// crates/algo-sdk/src/runtime/mod.rs
impl RuntimeSession {
    pub fn open(package_root, model_rel_path) -> Result<Self, AlgoError>;          // 既有签名，委托 Allow
    pub fn open_with_policy(package_root, model_rel_path, policy) -> Result<Self, AlgoError>;
    pub fn hardware_status(&self) -> HardwareStatus;   // Fallback 变体 → Simulated
    pub fn resolve_policy(is_self_test, platform_id, env, explicit) -> FallbackPolicy;
}
```

```rust
// crates/algo-sdk/src/plugin.rs
pub struct InitContext<'a> {
    pub package_root: &'a Path,
    pub platform_id: &'a str,
    pub instance_id: &'a str,
    pub is_self_test: bool,
    pub fallback_policy_override: Option<FallbackPolicy>,   // 加法式新增
}
impl<'a> InitContext<'a> {
    pub fn with_fallback_policy_override(self, policy: FallbackPolicy) -> Self;
}
```

### 3. 契约

**策略解析优先级（高 → 低）**，实现唯一落点在 `resolve_fallback_policy`：

| # | 条件 | 结果 |
| --- | --- | --- |
| 1 | `is_self_test == true` | `RequireHardware` —— **硬门，不可被任何下层依据翻越** |
| 2 | `explicit = Some(p)`（调用方显式声明） | `p` |
| 3 | `.env` 中 `ALLOW_CPU_FALLBACK` 为真值（`1`/`true`/`yes`/`on`） | `Allow` |
| 4 | `platform_requires_hardware(platform_id)` | `RequireHardware` |
| 5 | 其他（macOS / x86 开发机） | `Allow` |

**平台判定只读宿主自报的 `platform_id` 字符串，`algo-sdk` 内不得新增按目标 SoC / 宿主 OS 的 `cfg` 分支**：
同一进程可能同时装载多平台算法包，`cfg` 只能表达“本包编译成什么”，无法表达“宿主是什么”。
`normalize_platform_id` 与宿主 [`crates/infer/src/sandbox.rs`](../../../../crates/infer/src/sandbox.rs) 的同名函数是
**最小必要重复**（SDK 不能反向依赖宿主 crate）：只做 4 个别名族归一化，
两侧一致性由 `algo-sdk` 的表驱动单测与 `infer::algo_sandbox_tests::test_platform_alias_table_stays_in_sync_with_algo_sdk_copy` 双向锁定，任一侧漂移即失败。

**生产路径必须封死显式声明**：`export_algo!` 展开的 `instance_create` 构造 `InitContext` 时
恒定写 `fallback_policy_override: None`。显式声明仅供**不走 C ABI** 的本地开发工具使用。

**`RequireHardware` 的失败语义**：返回 [`AlgoError::ModelLoad`](../../../../crates/algo-sdk/src/error.rs)
→ `AV_ERR_MODEL_LOAD_FAILED = -5`。**零新增状态码、`AV_ALGO_API_VERSION` 不变、零 ABI 变更**。
沿既有链路自然变响：`init` 返回 `Err` → `macros.rs` 的 `set_last_error` + `to_c_status()` →
`instance_create` 返回 `-5` → 宿主 `check_c_status` 取回 `last_error` →
`sandbox.rs` 的 `create_code != AV_OK` 判定自检失败（**宿主无需新增校验代码**）。

**错误消息字段顺序是契约，不可随意重排**：宿主 `check_c_status` 用 **512 字节**栈缓冲读取
`last_error`（`crates/infer/src/c_abi/loader.rs`），`copy_last_error` 按 `min(len, cap - 1)` 截断。
底层 `libloading` 的失败原因（含候选路径 + dlopen 报错）本身就可达 300+ 字节，
因此必须按 **结论 → 操作指引 → `package_root` → 模型路径 → 底层原因（最易截断，放最后）** 排列。
把操作指引排到末位会让运维只能读到“模型加载失败”——恰好退回本契约要消灭的不可诊断状态。
另外 `RknnRuntime::load` 失败返回的已是 `ModelLoad`，直接 `{e}` 插值会嵌套出重复的 `模型加载失败: ` 前缀、
白耗缓冲配额，必须经 `describe_load_failure` 剥壳后再嵌入。

### 4. 验证与错误矩阵

| 条件 | 期望行为 |
| --- | --- |
| 自检模式 + 无运行时 | `instance_create` → `-5`，实例句柄为 null，`last_error` 含 `librknnrt` / `package_root` / `ALLOW_CPU_FALLBACK` |
| 自检模式 + `explicit = Some(Allow)` | **仍为 `RequireHardware`**（硬门不可翻越） |
| 自检模式 + `.env` `ALLOW_CPU_FALLBACK=1` | **仍为 `RequireHardware`**（硬门优先于 `.env`） |
| 硬件平台 + 常规实例 + 无声明 | `RequireHardware` —— 不是只在自检阶段设门；生产常规实例静默降级同属部署错误 |
| 硬件平台 + `explicit = Some(Allow)` | `Allow`（本地开发工具逃生口） |
| 硬件平台 + `.env` `ALLOW_CPU_FALLBACK=1` | `Allow`（运维显式覆盖；但自检不受影响） |
| 非硬件平台（macOS / x86） | `Allow`，回退会话可用且能完成 `instance_process` 闭环 |
| `ALLOW_CPU_FALLBACK` 为 `0`/`no`/缺失 | 不放开回退（只有真值才生效） |

### 5. Good / Base / Bad Cases

- **Good**：rknn 包在目标板上缺失 `librknnrt.so` → 自检第 5 步 `instance_create` 返回 `-5`，错误给出
  `package_root`、模型路径、底层 dlopen 原因与放开回退的操作指引，安装被拒绝。
- **Base**：macOS 开发机（`platform_id` 归一化为 `macos-arm64`）默认 `Allow`，无驱动回退照常可用。
- **Bad**：在自检路径上允许显式声明或 `.env` 翻越硬门 → 安装自检又能以“2 个假框”通过，
  缺陷原样回归。

### 6. 必测项（断言点）

单元（`crates/algo-sdk/src/runtime/fallback.rs`）：

- `allow_is_the_default_policy` / `test_session_options_default_keeps_existing_behavior` —— 默认即既有行为；
- `test_require_hardware_fails_without_runtime` —— 断言错误**变体**为 `ModelLoad` 且 `to_c_status() == -5`，
  并校验 4 个定位信息子串（不是仅 `is_err()`）；
- `self_test_gate_cannot_be_overridden_by_explicit_declaration` —— **回归锁**：硬门忽略显式声明；
- `self_test_gate_cannot_be_overridden_by_env` —— 硬门优先于 `.env`；
- `explicit_declaration_allows_simulation_on_hardware_platform` —— 开发工具逃生口；
- `platform_id_normalization_matches_host_contract` —— 别名族表驱动，锁定与宿主一致；
- `hardware_unavailable_error_keeps_actionable_hint_within_host_error_buffer` —— **宿主 512 字节窗口回归锁**：
  用与宿主相同的截断规则断言三个关键定位项均落在前 511 字节内（含超长 `package_root`）；
- `describe_load_failure_strips_redundant_model_load_prefix` —— 不嵌套 `模型加载失败: ` 前缀。

集成（走真实 C ABI 虚表，`crates/algo-sdk/tests/fallback_policy_gate.rs`）：

- `self_test_mode_without_hardware_fails_instance_create` —— 自检 + 无硬件 → `-5` + 句柄 null + `last_error` 三要素；
- `normal_mode_on_hardware_platform_also_requires_hardware` —— 常规实例同样受约束；
- `non_hardware_platform_keeps_simulation_available` —— 回退会话可完成 `instance_process`；
- `explicit_env_override_restores_simulation_on_hardware_platform` —— 保证错误里的操作指引不是空头承诺；
- `self_test_gate_cannot_be_overridden_by_env`。

SDK 侧 `models/yolo.rs`：`test_self_test_init_requires_real_hardware`、
`test_init_keeps_fallback_on_non_hardware_platform`、
`test_init_explicit_allow_enables_local_dev_fallback_on_hardware_platform`、
`test_init_self_test_gate_ignores_explicit_allow`。

> **判别力（已实测变异验证）**：把 `RequireHardware` 分支改回 `new_fallback(model_path)` → 6 个用例失败
> （含集成的 3 个）；把解析顺序改成 `explicit` 优先于 `is_self_test` → 2 个硬门回归锁失败；
> 把 `package_root` 移到消息末尾 → 512 字节窗口锁等 3 个用例失败。

### 7. Wrong vs Correct

#### Wrong

```rust
// 宿主装载的实例竟然可以声明“我允许模拟回退”，或把硬门排到显式声明之后
let policy = resolve_fallback_policy(ctx.is_self_test, ctx.platform_id, Some(&env),
                                     ctx.fallback_policy_override);
// 若实现把 explicit 判断写在 is_self_test 之前，安装自检就又可以用假框通过

// 本地开发工具依赖部署期文件才能跑通
// (在 run_local 里什么都不声明，指望运维去建 .env)
```

#### Correct

```rust
// 1) 生产路径：export_algo! 展开处恒定 None，宿主无法注入声明
let ctx = InitContext { /* ... */, fallback_policy_override: None };

// 2) 本地开发工具：意图写在代码里，不依赖 .env
let init_ctx = InitContext::new(Path::new("."), "linux-rknn", "standalone_local", false)
    .with_fallback_policy_override(FallbackPolicy::Allow);

// 3) 解析顺序：硬门永远第一
if is_self_test { return RequireHardware; }
if let Some(p) = explicit { return p; }
```

## 结果发射

结果字段、坐标、告警/证据职责与迁移要求统一见 [检测结果、告警与证据契约](../../pipeline/backend/detection-alarm-contract.md)。下表描述当前实现；`emit_detections` 尚未按新检测协议迁移。

| 方法                                     | 当前行为                          |
| -------------------------------------- | ----------------------------- |
| `emit_detections(&[NormBox])`          | 发射 `AV_RESULT_ALARM` 并附全景抓拍请求 |
| `emit_recognition_json(&[u8])`         | 发射识别结果，不自动附图片请求               |
| `emit_self_test(count)`                | 自检信号                          |
| `emit_json_result(kind, json, images)` | 通用结果与图片请求                     |

`BoxesSerializer` 直接写字节序列，避免中间字符串；当前 emitter 仍分配 JSON Vec/CString，不能声称完全零分配。
结果、JSON 和图片请求指针仅在同步回调期有效；长度不含尾部 NUL，JSON 内嵌 NUL 返回错误，消费者不能保存裸指针。

### 运行状态边界

- `instance_process` 返回 `AV_OK` 只表示插件报告本次同步调用成功返回；它不证明模型确实执行，也不证明输出结果在语义上正确。
- 结果回调及其载荷只服务于推理结果管线，不能作为算法语义正确性或运行状态的证据。
- 宿主侧监测口径、指标与在途心跳见 [推理运行时健康](../../infer/backend/inference-runtime-health.md)；插件只需保证调用按期返回并按自身契约发射结果。

## 包与沙箱

交付包含 `manifest.json`、`config.schema.json`、`testimage.jpg`、`README.md`、`lib/`、`model/`，排除源码、构建缓存和 `.env`。
实际工程可按平台/架构多层组织，发现逻辑复用 `discover_package_dirs`，不另写目录猜测。

- Manifest 必填：`manifest_version=1`、`algorithm_id`、`version`、`name`、`algorithm_type`、`alarm_type_id`、`platform_id`；description/min_adapter_version 可选。
- `algorithm_id` 与库元数据一致，version 使用语义化版本；模型文件按平台交付。
- 标准平台与别名：

| 标准 ID          | 兼容别名                                |
| -------------- | ----------------------------------- |
| `macos-arm64`  | `macos-arm64-coreml`、`darwin-arm64` |
| `linux-rknn`   | `linux-arm64-rknn`、`rknn`           |
| `linux-ascend` | `linux-arm64-ascend`、`ascend`       |
| `linux-x64`    | `generic-x86_64-cpu`、`linux-x86_64` |

归档由 [archive.rs](../../../../crates/api/src/algo/archive.rs) 识别 `.tar.gz/.tgz/.tar/.zip`，拒绝绝对路径、`..` 和逃逸目标目录的条目。
生产/上传必须启用子进程自检，失败拒绝加载。算法包必须完整通过以下**六步沙箱物理自检**：

| 检查               | 失败条件                                       |
| ---------------- | ------------------------------------------ |
| 1. 路径与结构         | canonicalize 失败，缺 manifest/lib/testimage   |
| 2. Manifest 与平台  | 解析/版本/平台匹配失败                               |
| 3. Config Schema | 存在但不是合法 JSON                               |
| 4. 子进程隔离         | 派生或监控失败                                    |
| 5. 元数据一致性        | library_query 的 algorithm_id 与 manifest 不同 |
| 6. 真实前向自检        | 原生测试帧处理失败，回调结果不合格，超时/异常退出                  |

子进程使用当前可执行文件的 `__verify-algo <package_dir>`，看门狗 **10000ms**；超时终止，非零退出或 SIGSEGV 等信号均失败。
库查找顺序：`lib/lib{algorithm_id}.{本机扩展名}` → 异构扩展名 → lib 目录按扩展名扫描；不能因此跳过平台匹配。
`AlgoRegistry` 的 `scan_and_register/load_and_register/open_and_register/get/list/unregister` 复用 canonical 路径去重，具体 async 签名以 `package.rs` 为准。
- **冷启动与运行时热重载守卫**：已在数据库中完成准入持久化的受信任算法包，冷启动与运行时版本切换统一使用轻量 `open` / `open_and_register`（仅执行动态链接与 C ABI 虚表握手，耗时 < 1ms），严禁在冷启动或已入库热切换时无条件重跑六步沙箱前向推理自测，避免触发边缘端冷启动推理风暴与 CMA 显存争抢。

## Apple Silicon

- `MLComputeUnitsAll=2`，`CPUOnly=0`；要求 ANE/GPU 能力的实现不误设为 0，实际设备调度需验证。
- 模型初始化时缓存 `objc_getClass/sel_registerName`，热路径不重复查选择器。
- Float16 输出转换使用 Accelerate `vImageConvert_Planar16FtoPlanarF` 批量处理，不逐元素标量位移。

## 后处理工具库 (cv::postprocess)

算法包通用后处理逻辑集中在 `algo_sdk::cv::postprocess`，避免各包重复实现量化、DFL 解码和多分支解析。

| 模块 | 职责 | 复用范围 |
|------|------|----------|
| `quantize` | `dequant_i8` / `quant_f32` INT8 量化反量化 | 任何量化模型 |
| `dfl` | `decode_dfl` DFL softmax 加权求和解码 | YOLOv8 系列（支持任意 bin 数） |
| `yolov8_rknn` | `parse_yolov8_int8` 多分支 INT8 解析 + score_sum 快筛 + logits 还原 + NMS | RKNN 优化版 YOLOv8 |

- 算法包通过 `Yolov8RknnConfig::from_spec` 参数驱动（输入尺寸、DFL bins、类别数、score_sum 开关、`cls_is_logits`），不需要为每个模型重写后处理。
- **分类分支激活语义必须与导出图一致**：`use_score_sum` 决定 9-tensor（含 score_sum 快筛）还是 6-tensor（官方标准）结构；`cls_is_logits` 声明 sigmoid 是否已被移出计算图。
  - 未声明 `cls_is_logits` 而模型实际为 logits 时，负值 logit 会被当作置信度直接与阈值比较，导致低分尺度（如全负的 `score_32`）**零检出**、其余尺度置信度系统性偏低。
  - `cls_is_logits = true` 时解码器按 `sigmoid(dequant(raw))` 还原置信度，并将阈值换算到 logit 空间（`ln(p/(1-p))`）后量化比较——因 sigmoid 单调，`sigmoid(x) > p ⟺ x > ln(p/(1-p))`，INT8 极值快速路径不变。换算统一收敛在 `ClassActivation::effective_threshold` / `to_confidence` 两个入口，不在解析器内散落条件分支。
  - 判定依据只能是**模型量化元数据**：分类分支范围含负值即为 logits（如 `score_32: [-9.72, -0.08]`）；非负区间（如 `[0, 0.75]`）则为图内已激活。不得凭文件名或 ONNX 输入 dtype 推断。
  - **`use_score_sum` 与 `cls_is_logits` 互斥**：score_sum 预筛依赖 `score_sum >= max_class_score >= 阈值`，该不等式仅在概率语义下成立，logits 语义下其余类别的负 logit 会把总和压低并过滤掉本该通过的网格（静默漏检）。`from_spec` 对该组合返回 `ConfigParse` 错误，算法包应在 `init` 阶段即失败而非每帧回退。
  - `YoloSpec::CLS_IS_LOGITS` 默认 `false`，存量 9-tensor 模型无需改动。
  - `debug_cpu_fallback_path` 的单张量桩（`InferenceOutput::SingleFloat`）分类通道直接是概率，不受 `CLS_IS_LOGITS` 影响；该标志只描述真实 RKNN 量化图。算法包不得仅凭单浮点回退用例覆盖 activation 语义。
- 扩展新模型（YOLOv11、RT-DETR 等）在 `postprocess/` 下新增文件，组合现有原语或实现新的解码逻辑。
- `RknnTensorOutput` 类型定义在 `postprocess::yolov8_rknn`，各算法包通过 re-export 使用，不在本地重复定义。
- 自定义标签覆盖在算法包 plugin 层完成（`parse_yolov8_int8` 返回后 `.label = Some(custom)`），不耦合到通用解析器。

## Rockchip RKNN

- **无硬件时不得静默降级**：`RknnSession::open_or_fallback` 由 `RknnSessionOptions::fallback_policy`
  控制（`Allow` 降级为 `debug_cpu_fallback_path` 模拟会话：2 个写死的目标；`RequireHardware` 返回
  `AlgoError::ModelLoad` → `-5`）。策略、优先级与自检硬门契约见 [硬件回退策略](#硬件回退策略-fallbackpolicy)。
  新增会话入口时必须接入策略，不得自行决定降级。
- RK3576 双核用 `RKNN_NPU_CORE_0_1=3`，RK3588 三核用 `RKNN_NPU_CORE_0_1_2=7`；不把 AUTO 当已启用多核。
- 本项目 BSP 的 `rknn_create_mem_from_fd` 需要有效 `virt_addr`；`dma_mem_cache` 持有映射，禁止逐帧 mmap/munmap。
- **DMA-BUF 映射权限硬性约束**：使用 `mmap` 将输入 DMA-BUF 映射为虚拟地址供 `rknn_create_mem_from_fd` 使用时，必须声明为 `libc::PROT_READ | libc::PROT_WRITE`。严禁仅使用只读 `PROT_READ`，否则后续调用 `rknn_inputs_set` 执行 Host 内存拷贝时，`librknnrt` 向该张量虚拟地址写入数据将立即触发 Linux 内核缺页写保护致命段错误（SIGSEGV）。
- **受限 CMA 内存下的会话复用**：在 RK3568 等物理连续内存紧缺平台（如 `CmaTotal: 16MB`），两阶段算法（检测+识别）必须通过 `SharedModels` 弱引用单例 Actor 模式统一管理底层 RKNN Context，禁止按摄像头重复初始化导致 CMA OOM。
- INT8 DFL 路径保持 `want_float=0`，先按 `(raw_cls-zp)*scale >= conf_thresh` 剪枝，只对候选网格执行 16-bin softmax；通用解析器通过 `Yolov8RknnConfig.dfl_bins` 和 `num_classes` 参数化，不硬编码为特定模型。
- `RknnOutputsGuard` 在所有退出路径调用 `rknn_outputs_release`；同一 context 非线程安全，必须绑定所属 Worker。

### 连续失败追踪
- **诊断日志**：`source_layout()` 仅在错误分支打 `tracing::error!` 记录上下文（frame_id、stride、format），热路径入口不打 debug 日志。
- **失败追踪 (`FailureTracker`)**：定义在跨平台的 [`cv/diagnostic`](../../../../crates/algo-sdk/src/cv/diagnostic/mod.rs)（与 librga 无绑定，不受 `feature = "rga"` 门控），由 [`GenericDetector::process`](../../../../crates/algo-sdk/src/models/yolo.rs) 接入：**计数边界为硬件段（`Transform` 预处理 + `NpuSession` 推理）**，成功 `record_success()` 重置并打恢复日志，失败 `record_failure()` 递增，`flush()` 时 `reset()` 静默清零；默认阈值 30 帧（约 1 秒 @30fps）。基于 `AtomicU64` 纯计数。
- **状态暴露**：连续失败属于硬件内部状态，通过日志与健康检查暴露，不通过 `ResultEmitter` 向前端推送非业务系统告警。
- **计数边界不得拓宽**：解码与结果发射属业务侧，其失败（如宿主回调断开）严禁计入该计数器，否则会累积到阈值并伪造硬件降级信号，同时让 `record_success()` 打出不成立的“恢复正常”日志。`models::yolo::tests::test_failure_tracker_records_outcome_and_flushes` 锁定成功归零、失败递增、`flush()` 静默重置，以及“发射失败不计入硬件失败”四项契约。

## 算法包私有环境与参数调优 (.env)

为满足边缘现场调优与快速迭代需求，算法包支持通过根目录下的 `.env` 文件调试模型路径与运行时超参数，实现**免重新编译秒级生效**，同时严格遵循进程级安全隔离规范。

### 1. 私有作用域与零全局泄漏 (Package-Scoped Isolation)
- **严格红线**：算法包严禁调用 `std::env::set_var`，严禁在多线程环境中重写宿主操作系统的全局 `environ` 指针，彻底杜绝数据竞争与多算法包相互污染。
- **纯内存局部解析**：算法包在 `init(ctx, config)` 阶段通过 `ctx.load_env()`（基于 `algo_sdk::env::PackageEnv`）只读加载 `package_root/.env`，解析为包实例私有的只读字典，生命周期仅限于当前包内部。
- **发布隔离**：生产打包脚本（`Makefile`）必须通过 `--exclude='.env'` 排除本地调试配置文件；算法包源码中保留带详细参数注解的 `.env.example` 作为现场调优范例。

### 2. 三级参数覆盖优先级阶梯 (Precedence Hierarchy)
```text
┌─────────────────────────────────────────────────────────────┐
│ 【第一级 · 最高级】 宿主显式下发的任务配置 (task.parameters)      │  <-- 针对单路通道/任务的个性化配置严格受保护
└──────────────────────────────┬──────────────────────────────┘
                               │ (宿主未显式提供该参数时回退)
                               ▼
┌─────────────────────────────────────────────────────────────┐
│ 【第二级 · 局部调试】 算法包私有 .env (package_root/.env)         │  <-- 独立 run_local / 本地基线调试免编译即改即生效
└──────────────────────────────┬──────────────────────────────┘
                               │ (.env 也未提供该参数时回退)
                               ▼
┌─────────────────────────────────────────────────────────────┐
│ 【第三级 · 兜底保底】 代码硬编码固定默认值 (Hardcoded Defaults)    │  <-- 算法内部官方基准常数保底
└─────────────────────────────────────────────────────────────┘
```
算法包配置结构体（`InstanceConfig`）在反序列化时记录宿主显式下发的字段集合（`explicit_fields`），在 `config.apply_env(&env)` 时**仅对宿主未下发的字段进行覆盖**，确保生产环境下宿主任务调度与控制台配置拥有绝对权威。

### 3. 模型路径解析契约
- 模型文件属于平台专属权重资产，不强制在 `manifest.json` 中强行绑定；
- 遵循解析顺序：`package_root/.env` 指定路径（如 `MODEL_PATH` / `DETECTOR_MODEL_PATH`） → 约定的固定模型文件路径（`model/*.rknn` 或 `model/*.mlpackage`）；
- `PackageEnv::resolve_model_path()` 强制防路径穿越检查，拒绝任何包含 `..` 的相对路径，确保模型路径安全规范化在合法物理文件系统内。

### 4. 不得用 `.env` 充当安全开关

`.env` 被 `.gitignore` 忽略（仅 `.env.example` 入库），新设备上必然缺失。
以“部署时不存在的东西”作为安全开关是循环依赖，因此：

- **默认策略不得从 `.env` 读取**：无硬件时是否允许回退，由宿主自报的 `platform_id` 与是否处于安装自检决定；
- `ALLOW_CPU_FALLBACK` 仅作为**显式覆盖**（`=1` 时强制 `Allow`），用于运维在无硬件环境手动放开；
- **它不得翻越安装自检硬门**：自检模式下策略恒为 `RequireHardware`；
- 本地开发工具（`run_local` 等）不得依赖该变量，应在代码中显式声明
  `InitContext::with_fallback_policy_override(FallbackPolicy::Allow)`；
- 新增任何“部署期文件驱动安全行为”的开关前，先回答：该文件在新设备/干净检出上是否必然存在。

详见 [硬件回退策略](#硬件回退策略-fallbackpolicy)。

## 验证与已知差异

[testing.rs](../../../../crates/algo-sdk/src/testing.rs) 提供 `MockFrameBuilder`、`MockEmitter`、`MockWeights/MockSession`；图片/硬件辅助分别由 `testing-image/testing-hardware` 启用。

- 验证合法帧、负 stride/重叠 offset、无效句柄/版本、配置更新、flush 及资源释放。
- `MockFrameBuilder::to_nv12(64)` 覆盖对齐；真机测试标记 `#[ignore]`（见 [全局约定](../../guides/conventions.md#测试)）。
- ABI 双侧 [SDK 布局测试](../../../../crates/algo-sdk/tests/c_abi_layout_tests.rs) / [宿主布局测试](../../../../crates/infer/tests/c_abi_layout_tests.rs) 同步；另见 [插件生命周期](../../../../crates/algo-sdk/tests/plugin_lifecycle.rs)、[CV](../../../../crates/algo-sdk/tests/cv_tests.rs)、[沙箱](../../../../crates/infer/tests/algo_sandbox_tests.rs)。
- 验证三路径隔离、池上限、坐标往返、回调借用期与归档越界拒绝。

本次文档整理确认的差异，不能视作已完成能力：

| 旧描述                                                         | 当前事实 / 待对齐点                                                |
| ----------------------------------------------------------- | ---------------------------------------------------------- |
| SafeFrame 强制 `<16384` 尺寸上限                                  | 当前检查非零尺寸与内存布局，尚无该上限检查；不能依赖草案常量                             |
| 宿主所有停止/重置路径均 flush                                          | SDK 已提供入口，`RawAlgoInstance::drop` 当前直接 destroy；调用接线仍需验证/补齐 |
| `instance_update_config` 更新离线提取阈值                           | 库级提取当前使用自身阈值，不能推断与实例配置自动联动；模型由 `shared_models` 惰性初始化       |
| Manifest runtime_constraints/resource_profile/self_test 已生效 | 当前 `AlgoManifest` 未建模这些扩展字段；不能据此声称 OS、资源或自检超时限制已执行         |
| config.schema 必填且六步全部实现                                     | 交付要求保留，但当前校验允许 schema 缺失，沙箱为上述六项                           |
