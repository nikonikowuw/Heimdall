# 算法 SDK 与 C ABI

适用于 `crates/algo-sdk`、`crates/infer` 插件适配层和 `algo-packages/`。插件不依赖 `types/infer/pipeline/db` 等宿主业务 crate。

## 构建边界

- Heimdall 根 workspace 只构建宿主 crate 与 `algo-sdk`；根 `Cargo.toml` 不列出任何具体算法包。
- `algo-packages/` 按平台和硬件运行时拆分为 `macos`、`rknn/rk3568`、`rknn/rk3576` 三个独立 workspace，各自维护成员、算法侧依赖版本、锁文件和构建缓存。算法包通过各自 workspace 的相对路径依赖使用 `crates/algo-sdk`，不通过宿主业务 crate 反向依赖运行时。
- 算法包构建、测试、格式化和交叉编译均以目标平台 workspace 的 manifest 为入口；交付前生成的 `.so/.dylib` 复制到包内 `lib/`，再由 Makefile 打成归档。
- 包内 `lib/` 的插件库是本地构建产物，**不入版本库**（`algo-packages/` 各级 `.gitignore` 已覆盖 `*.so` / `*.dylib`）：版本库只承载 manifest、config schema、模型与源码。从干净检出开始时必须先在该包 workspace 执行 `make` / `make package` 生成 `lib/`（归档同样不入库）；否则宿主启动自愈扫描会因缺少插件库而无法自检装载该内置包，依赖真实包的沙箱测试也只会静默跳过。
- 多个平台 workspace 可以共享 SDK 源码，但不共享 Cargo 的成员集合、锁文件或构建缓存。宿主只在运行时通过 manifest 和 C ABI 动态加载算法制品。

## 权威定义

| 内容             | 源文件                                                                                                                                                          |
| -------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| SDK / 宿主 C ABI | [c_abi.rs](../../../crates/algo-sdk/src/c_abi.rs)、[types.rs](../../../crates/infer/src/c_abi/types.rs)                                                       |
| 插件 trait / 导出宏 | [plugin.rs](../../../crates/algo-sdk/src/plugin.rs)、[macros.rs](../../../crates/algo-sdk/src/macros.rs)                                                      |
| 帧 / 预处理 / 模型会话 | [frame.rs](../../../crates/algo-sdk/src/frame.rs)、[cv](../../../crates/algo-sdk/src/cv/mod.rs)、[model.rs](../../../crates/algo-sdk/src/model.rs)             |
| 后处理工具库 | [cv::postprocess](../../../crates/algo-sdk/src/cv/postprocess/mod.rs)（quantize / dfl / yolov8_rknn）                                                  |
| 目标跟踪算法库 | [track::bytetrack](../../../crates/algo-sdk/src/track/bytetrack.rs)（纯 Rust ByteTrack、卡尔曼滤波与匈牙利最优二分图匹配）                                     |
| 人脸视觉几何库 | [face](../../../crates/algo-sdk/src/face/mod.rs)（Umeyama 相似变换、ArcFace 模板对齐、五点质量姿态拓扑几何）                     |
| 结果 / 向量数学库   | [emitter.rs](../../../crates/algo-sdk/src/emitter.rs)、[math.rs](../../../crates/algo-sdk/src/math.rs)（IoU、NMS、余弦相似度、L2 归一化、Base64 编码）          |
| 加载 / 沙箱 / 注册表  | [loader.rs](../../../crates/infer/src/c_abi/loader.rs)、[sandbox.rs](../../../crates/infer/src/sandbox.rs)、[package.rs](../../../crates/infer/src/package.rs) |

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
- 每个 C ABI 入口隔离 panic，失败返回 `AV_ERR_INTERNAL`（panic 隔离规则见 [全局约定](../guides/conventions.md#防御性错误处理)）。
- `AlgoPlugin: Sized + Send + 'static`，配置为 `DeserializeOwned + Default`；必需实现 `init(ctx, config)` 和同步 `process(SafeFrame, &mut ResultEmitter)`。
- `InitContext` 提供 `package_root/platform_id/instance_id/is_self_test`。默认 `flush/set_rules` 返回成功，`update_config` 返回 `NotImplemented`，不能误报配置已应用。

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

`AlgoError` 映射见 [error.rs](../../../crates/algo-sdk/src/error.rs)；`last_error` 使用线程局部错误缓存提供详情，安全层不传播 C 整数错误码。

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
- 当前实现：[macOS](../../../algo-packages/macos/arm64/face_recognition/src/lib.rs)、[RK3576](../../../algo-packages/rknn/rk3576/face_recognition/src/lib.rs)。

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

### 并发与内存模型（Wait-Free RCU）

- 算法包内部使用 `FaceGallery` 容器，通过 RCU（Read-Copy-Update）与 `Arc<GallerySnapshot>` 维护底库快照。
- **读写完全解耦**：写操作（`insert` / `remove` / `clear`）获取排他写锁，生成新快照并原子替换指针；检索操作（`search`）仅在入口获取瞬间读锁克隆快照的 `Arc` 指针即刻释放读锁，后续点积矩阵计算与 Top-K 堆排序在只读快照上执行，实现**多路摄像头 Worker 线程并发检索完全无锁竞争（Wait-Free）**。
- **内存占用极致轻量**：10,000 张 512D FP32 人脸底库仅占用约 21 MB 内存，多实例完全共享同一底层物理句柄。
- **宿主单一真实信源（Single Source of Truth）**：持久化数据以宿主 SQLite 为唯一准绳，算法包底库仅作为运行期纯内存加速索引；宿主启动或算法包热重载时通过 C ABI 执行全量同步（`clear` + 批量 `insert`），毫秒级重建完成。

### 导出宏

算法包通过 `export_face_gallery!` 宏一键导出该虚表，内置 panic unwind 隔离防崩溃保护：

```rust
algo_sdk::export_face_gallery!(FaceRecognizer);
```

## 硬件预处理与会话

宿主向算法实例提供解码后的原生 `FrameRef`/`AvFrameDesc`，不为所有算法强制设定统一模型输入尺寸。当前默认由算法包自行选择预处理尺寸、裁切、色彩格式和归一化方式；若后续启用宿主预处理，算法包必须先通过 `instance_negotiate` 声明可接受的帧能力，宿主再按实例约束执行，不能用单一目标尺寸覆盖所有模型。

- `CvEngine::letterbox/resize` 返回 `(CvBuffer, PreprocessMode)`；宿主 `AvImageOps` 注入优先，未注入时使用平台引擎。
- 平台默认：macOS→AppleCvEngine，Linux+rga→RgaCvEngine，其他→CpuCvEngine；生产 CPU 回退仍受 [三路径边界](./media-pipeline.md#三条路径) 限制。
- `with_engine` 在线程局部 scope 绑定引擎，RAII Guard 在正常/错误/panic 路径恢复栈，不在实例间共享回调表。
- `CvBuffer` 统一管理 Host、DMA-BUF、CVPixelBuffer、Ascend 显存与宿主视图；宿主视图析构调用相应 free，外部句柄必须有明确 guard/释放责任。
- `compute_letterbox_layout` 提供 scale、padding 和缩放尺寸，`unmap_box` 复用同一布局完成逆变换。
- RGA 输出池在初始化时 import handle 并复用；输入 handle 由 `RgaHandleGuard` 单帧管理，禁止每帧重复 import/release 输出池。
- 单 `RgaCvEngine` 最多缓存 **16** 个输出规格，超限拒绝；优先 system-dma32/system，只有显式 Rga2 强制 DMA32，Auto/Rga3 可使用 64 位物理地址堆。
- 规格预算按**进程**共享且**不淘汰**：所有算法包、所有实例、所有分辨率都从同一 16 个槽位分配。因此算法包请求的 RGA 输出几何必须收敛到固定集合，**禁止**把随帧变化的 ROI 尺寸直接作为裁剪尺寸（典型翻车：best-shot 按人脸位置逐帧精确裁切，十几条航迹即耗尽预算，此后该引擎全部 `letterbox/resize` 永久失败）；超出档位集合时必须显式退化为固定尺寸（如整帧该轴尺寸），不得静默新增规格。
- **几何预算的真实不变量是「单分辨率内有界」，不是「与分辨率无关」**：仅靠「超出档位就退化为整帧该轴尺寸」的策略，每个分辨率仍会额外贡献 1 个与帧尺寸绑定的退化档位（含该分辨率的 letterbox 规格则算 2 个），因此**进程并集随部署分辨率种类线性增长**——实测 4 种常见分辨率（640×360 / 704×576 / 1280×720 / 1920×1080）在 RK3568 人脸包上并集为 9 种几何，7 种分辨率可达 12 种，逼近 16 槽上限。落地要求：
  1. 档位集合与退化策略必须用**跨分辨率并集**回归测试钉住上限（参考 RK3568 人脸包 `plugin::tests::snapshot_roi_geometry_union_stays_within_process_budget`，含「扫描确实到达退化分支」的饱和校验），不能只断言单分辨率内的档位数；
  2. 部署核对时按「分辨率集合 × 档位并集 + 各算法包 letterbox 规格 + 退化档」核算，为未来算法包预留余量；
  3. 需要彻底解耦时应改用固定画布 + 几何逆变换（源图降采样到定长画布后再采样），但会引入重采样误差，必须先做 embedding 一致性验证，不得为省槽位牺牲识别精度。
  参考 RK3568 人脸包 `SNAPSHOT_ROI_TIERS`（128/192/256/384）及其档位有界性回归测试。
- 单个规格的池 `max_size` 默认 4（`min_idle=2`、`acquire_timeout_ms=50`）：第 5 个并发租约会阻塞至多 50 ms 后失败，调用方不得假设 `acquire` 永不阻塞。
- `SharedWeights<W>` 用 `Arc<W>` 共享权重，`session()` 用 `AtomicUsize` Round-Robin 分配，`session_on(Core)` 显式绑定。
- `Core` 支持 Auto、Id、All、Mask；各实例 session 独占。逻辑共享不代替具体 SDK 的物理内存验证。

## 结果发射

结果字段、坐标、告警/证据职责与迁移要求统一见 [检测结果、告警与证据契约](./detection-alarm-contract.md)。下表描述当前实现；`emit_detections` 尚未按新检测协议迁移。

| 方法                                     | 当前行为                          |
| -------------------------------------- | ----------------------------- |
| `emit_detections(&[NormBox])`          | 发射 `AV_RESULT_ALARM` 并附全景抓拍请求 |
| `emit_recognition_json(&[u8])`         | 发射识别结果，不自动附图片请求               |
| `emit_self_test(count)`                | 自检信号                          |
| `emit_json_result(kind, json, images)` | 通用结果与图片请求                     |

`BoxesSerializer` 直接写字节序列，避免中间字符串；当前 emitter 仍分配 JSON Vec/CString，不能声称完全零分配。
结果、JSON 和图片请求指针仅在同步回调期有效；长度不含尾部 NUL，JSON 内嵌 NUL 返回错误，消费者不能保存裸指针。

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

归档由 [archive.rs](../../../crates/api/src/algo/archive.rs) 识别 `.tar.gz/.tgz/.tar/.zip`，拒绝绝对路径、`..` 和逃逸目标目录的条目。
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
| `yolov8_rknn` | `parse_yolov8_int8` 多分支 INT8 解析 + score_sum 快筛 + NMS | RKNN 优化版 YOLOv8 |

- 算法包通过 `Yolov8RknnConfig` 参数驱动（输入尺寸、DFL bins、类别数、score_sum 开关），不需要为每个模型重写后处理。
- 扩展新模型（YOLOv11、RT-DETR 等）在 `postprocess/` 下新增文件，组合现有原语或实现新的解码逻辑。
- `RknnTensorOutput` 类型定义在 `postprocess::yolov8_rknn`，各算法包通过 re-export 使用，不在本地重复定义。
- 自定义标签覆盖在算法包 plugin 层完成（`parse_yolov8_int8` 返回后 `.label = Some(custom)`），不耦合到通用解析器。

## Rockchip RKNN

- RK3576 双核用 `RKNN_NPU_CORE_0_1=3`，RK3588 三核用 `RKNN_NPU_CORE_0_1_2=7`；不把 AUTO 当已启用多核。
- 本项目 BSP 的 `rknn_create_mem_from_fd` 需要有效 `virt_addr`；`dma_mem_cache` 持有映射，禁止逐帧 mmap/munmap。
- **DMA-BUF 映射权限硬性约束**：使用 `mmap` 将输入 DMA-BUF 映射为虚拟地址供 `rknn_create_mem_from_fd` 使用时，必须声明为 `libc::PROT_READ | libc::PROT_WRITE`。严禁仅使用只读 `PROT_READ`，否则后续调用 `rknn_inputs_set` 执行 Host 内存拷贝时，`librknnrt` 向该张量虚拟地址写入数据将立即触发 Linux 内核缺页写保护致命段错误（SIGSEGV）。
- **受限 CMA 内存下的会话复用**：在 RK3568 等物理连续内存紧缺平台（如 `CmaTotal: 16MB`），两阶段算法（检测+识别）必须通过 `SharedModels` 弱引用单例 Actor 模式统一管理底层 RKNN Context，禁止按摄像头重复初始化导致 CMA OOM。
- INT8 DFL 路径保持 `want_float=0`，先按 `(raw_cls-zp)*scale >= conf_thresh` 剪枝，只对候选网格执行 16-bin softmax；通用解析器通过 `Yolov8RknnConfig.dfl_bins` 和 `num_classes` 参数化，不硬编码为特定模型。
- `RknnOutputsGuard` 在所有退出路径调用 `rknn_outputs_release`；同一 context 非线程安全，必须绑定所属 Worker。

### RGA 输入对齐诊断与连续失败追踪
- **诊断日志**：`source_layout()` 仅在错误分支打 `tracing::error!` 记录上下文（frame_id、stride、format），热路径入口不打 debug 日志。
- **失败追踪 (`FailureTracker`)**：基于 `AtomicU64` 纯计数；连续失败递增，成功时 `record_success()` 重置并打恢复日志，`flush()` 时 `reset()` 静默清零；默认阈值 30 帧（约 1 秒 @30fps）。
- **状态暴露**：连续失败属于硬件内部状态，通过日志与健康检查暴露，不通过 `ResultEmitter` 向前端推送非业务系统告警。

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

## 验证与已知差异

[testing.rs](../../../crates/algo-sdk/src/testing.rs) 提供 `MockFrameBuilder`、`MockEmitter`、`MockWeights/MockSession`；图片/硬件辅助分别由 `testing-image/testing-hardware` 启用。

- 验证合法帧、负 stride/重叠 offset、无效句柄/版本、配置更新、flush 及资源释放。
- `MockFrameBuilder::to_nv12(64)` 覆盖对齐；真机测试标记 `#[ignore]`（见 [全局约定](../guides/conventions.md#测试)）。
- ABI 双侧 [SDK 布局测试](../../../crates/algo-sdk/tests/c_abi_layout_tests.rs) / [宿主布局测试](../../../crates/infer/tests/c_abi_layout_tests.rs) 同步；另见 [插件生命周期](../../../crates/algo-sdk/tests/plugin_lifecycle.rs)、[CV](../../../crates/algo-sdk/tests/cv_tests.rs)、[沙箱](../../../crates/infer/tests/algo_sandbox_tests.rs)。
- 验证三路径隔离、池上限、坐标往返、回调借用期与归档越界拒绝。

本次文档整理确认的差异，不能视作已完成能力：

| 旧描述                                                         | 当前事实 / 待对齐点                                                |
| ----------------------------------------------------------- | ---------------------------------------------------------- |
| SafeFrame 强制 `<16384` 尺寸上限                                  | 当前检查非零尺寸与内存布局，尚无该上限检查；不能依赖草案常量                             |
| 宿主所有停止/重置路径均 flush                                          | SDK 已提供入口，`RawAlgoInstance::drop` 当前直接 destroy；调用接线仍需验证/补齐 |
| `instance_update_config` 更新离线提取阈值                           | 库级提取当前使用自身阈值，不能推断与实例配置自动联动；模型由 `shared_models` 惰性初始化       |
| Manifest runtime_constraints/resource_profile/self_test 已生效 | 当前 `AlgoManifest` 未建模这些扩展字段；不能据此声称 OS、资源或自检超时限制已执行         |
| config.schema 必填且六步全部实现                                     | 交付要求保留，但当前校验允许 schema 缺失，沙箱为上述六项                           |
