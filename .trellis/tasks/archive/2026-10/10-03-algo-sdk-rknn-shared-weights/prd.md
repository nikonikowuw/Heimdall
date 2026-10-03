# SDK placement 注入与 RKNN 共享权重运行时需求 (PRD)

> 所属母任务：`10-01-npu-core-allocation`（阶段 C）。

## 1. 目标与范围

本任务聚焦于**算法 SDK（`crates/algo-sdk`）与算法包（`algo-packages/rknn/`）的物理权重复用与多核独立执行**：
打通宿主下发的 `WirePlacementMetadata` 决策，基于首选**路线 A（控制面加载 root，`rknn_dup_context` 派生 child 并一次性独占移交实例 Worker 线程；实例线程独占设核、独占 IO 与推理）**构建共享权重 Provider，彻底移除人脸识别与 YOLO 插件内部全局串行的 `SharedModels` 单例 Actor，实现“物理权重单份驻留、多实例多核真正并发”。

### 本期范围
1. **InitContext 升级与 Placement 决策注入**：
   - 扩展 `InitContext`，支持注入可选的 `WirePlacementMetadata`（`device_id`、`core_mask`、`strategy` 等）；
   - 剥离进程级 `RKNN_CORE_MASK` 环境变量的隐式覆盖，改为在会话级别逐实例设核；
   - 插件在 `instance_create` 时生成合法的 `AvAlgoInstanceReceiptPod`，在实例销毁时生成 `AvAlgoCleanupReceiptPod`。
2. **`rknn_dup_context` 共享权重运行时与 Provider (`RknnSharedWeights`)**：
   - 在 `crates/algo-sdk/src/runtime/platforms/rockchip.rs` 引入 `rknn_dup_context` 动态符号绑定与安全抽象；
   - 建立 `RknnSharedWeightProvider`：同一进程/包内同模型只加载一次物理权重（Root Context）；
   - 派生独立 child session，采用 `Send + !Sync` 独占所有权胶囊一次性移交给目标 Worker 线程；
   - 先子后根的 RAII 清理：子 session 析构调用 `rknn_destroy(child)`，Root 引用计数归零时安全调用 `rknn_destroy(root)`。
3. **移除人脸识别与 YOLO 的全局串行 Actor**：
   - 重构 `algo-packages/rknn/rk3588/face_recognition`：移除全局 `static SHARED_MODELS` 串行 Mailbox，每个 `FaceRecognizer` 拥有独立 child session，消除核间吞吐死锁；
   - 升级 `GenericDetector`（YOLO）：无缝支持多实例基于 `rknn_dup_context` 物理权重共享与独立分核；
   - 离线提取任务（如 `av_algo_extract_face`）同样复用共享物理权重，杜绝重复加载模型与显存浪费。
4. **硬核安全与回退约束**：
   - 严格遵循 `FallbackPolicy::RequireHardware`：自检模式与生产硬件环境下，缺 dup 符号或派生失败坚决报错（`-5`），严禁静默回退到 `debug_cpu_fallback_path`；
   - 跨线程安全性与生命周期隔离验证（T10, T11, T28, T29, T31–T38, T44）。

### 不在本期范围
- REST API 与 SQLite 数据库表的持久化配置（由 `10-03-npu-persistence-revision-barrier` 负责）；
- 板端 8 小时长稳压测与真实 benchmark 出表（由 `10-03-npu-board-verification-release` 负责）。

---

## 2. 需求列表

### R01 — InitContext 扩展与无侵入兼容
* `InitContext` 增加 `placement: Option<WirePlacementMetadata>` 字段与链式构建接口；
* 维持向后兼容：当宿主未传递 placement 元数据时，保持原有默认核心策略；
* 严禁通过 `std::env::set_var("RKNN_CORE_MASK", ...)` 污染整个进程，改用 `rknn_set_core_mask(child_ctx, mask)` 逐实例绑定。

### R02 — 路线 A 共享权重运行时 (`RknnSharedWeights`)
* 基于 `rknn_dup_context` 提供物理权重复用能力；
* 控制面加载 root context；各实例创建时调用 `rknn_dup_context` 产生 child context；
* 独占所有权封装为 `Send + !Sync` 胶囊移交至实例 Worker 专用线程，禁止跨线程共享同一个 context；
* 实例 Worker 内部独占调用 `rknn_set_core_mask`、`rknn_inputs_set`、`rknn_run` 与 `rknn_outputs_get`；
* 引用计数与先子后根 RAII：持有 Root 句柄的 `Arc<RknnRootWeight>`，子 context 析构不影响 Root，最后一个子 context 释放后 Root 安全退火/销毁。

### R03 — 算法插件解耦（移除全局串行 Actor）
* 移除 `algo-packages/rknn/rk3588/face_recognition` 中 `static SHARED_MODELS` 及串行 `InferenceWorker` 队列；
* 每个 `FaceRecognizer` 实例直接持有其专用的 YOLOv8-Face 与 EdgeFace child sessions；
* 多路摄像头实例并行执行推理，互不排队阻塞，彻底发挥 RK3588 3 核并行算力。

### R04 — C ABI 放置回执合规交付
* 插件实现 `av_algo_get_placement_extension`，输出合法的 `AvAlgoPlacementExtensionV1`；
* 实例创建时正确填充 `AvAlgoInstanceReceiptPod`（`status = 1` 确认成功绑定，`applied_core_mask` 为实际绑核掩码）；
* 实例销毁时生成 `AvAlgoCleanupReceiptPod`，供宿主账本进行审计和释放。

---

## 3. 验收准则 (Acceptance Criteria)

- [ ] **AC01** (对应原 AC06, AC08): `InitContext` 成功接收宿主注入的 `WirePlacementMetadata`；新旧插件构造点 100% 编译通过且无回退。
- [ ] **AC02** (对应原 AC19, AC20): 同一模型权重文件在多实例启动时只触发一次底层 `rknn_init`，多实例均通过 `rknn_dup_context` 派生 child context。
- [ ] **AC03** (对应原 AC01, AC19): 移除全局串行 Actor；多路人脸识别实例在不同核心（如 Core 0 与 Core 1）上可同时并发执行推理，无锁排队。
- [ ] **AC04** (对应原 AC04, AC21): 乱序销毁实例时，先释放 child context，Root 权重在所有 child 释放后安全析构，无显存泄漏或崩溃。
- [ ] **AC05** (对应原 AC24): 在 `is_self_test = true` 或生产硬件环境下，当派生失败或无硬件加速器时，严格遵循 `RequireHardware` 抛出错误，拒绝伪造 CPU 回退。
- [ ] **AC06** (对应原 AC23): 全链路契约符合路线 A；context 句柄严格绑定到所属 Worker 线程，杜绝多线程并发访问同一 context。
