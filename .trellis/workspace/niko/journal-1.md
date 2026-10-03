# Journal - niko (Part 1)

> AI development session journal
> Started: 2026-10-01

---



## Session 1: 统一前端抽屉组件与两轴 code review 修复

**Date**: 2026-10-01
**Task**: 统一前端抽屉组件与两轴 code review 修复
**Package**: web
**Branch**: `dev`

### Summary

新增共享 Drawer 组合层并迁移 7 个业务抽屉（实体材质、small/compact/medium/wide 尺寸、统一 header/唯一滚动主体/可选固定工具栏与底栏）。两轴 code review 后修复 10 项发现：移除从未渲染的 Drawer ariaLabel 死参数（ModalOverlay 名称契约改为「至少提供其一」类型联合）、新增 titleTooltip 回填摄像头长设备名提示、.drawer-footer > :only-child 接管单一操作项对齐、清除抽屉主体内与实体面板重复的 backdrop-blur/frosted-glass、以 CSS 规则级测试守护固定头尾与唯一滚动区，并修正 AccountPanelDrawer.test.tsx 断言他组件内部类名的问题。Web 门禁全绿：479 tests / lint --max-warnings=0 / typecheck / check:cycles / build。亮暗主题与窄视口视觉检查由开发者人工完成。

### Main Changes

(Add details)

### Git Commits

| Hash | Message |
|------|---------|
| `7d32088` | (see git log) |

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 2: 归档 Bootstrap Guidelines

**Date**: 2026-10-01
**Task**: 归档 Bootstrap Guidelines
**Package**: infer
**Branch**: `dev`

### Summary

按用户要求归档 00-bootstrap-guidelines；抽屉实现提交 7d32088 已由先前 journal 记录，本条不重复引用。NPU 多核分配任务继续处于 planning，模板哈希清单的未提交改动予以保留。

### Main Changes

(Add details)

### Git Commits

(No commits - planning session)

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 3: 归档事件驱动 NVR 录像任务

**Date**: 2026-10-01
**Task**: 归档事件驱动 NVR 录像任务
**Package**: media
**Branch**: `dev`

### Summary

核验 09-30-event-nvr-recording 交付现状，补齐上下文配置并验证 media/pipeline/db/api/web 全套门禁与测试，成功将已完成任务归档至 archive/2026-10/。

### Main Changes

(Add details)

### Git Commits

(No commits - planning session)

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 4: Numeric input blur validation

**Date**: 2026-10-02
**Task**: Numeric input blur validation
**Package**: algo-sdk
**Branch**: `dev`

### Summary

实现共享 NumericField 与 numericDraft，迁移录像、GB28181、网络和存储数值输入，增加失焦/提交兜底及交互测试，并固化前端 spec。Web 自动门禁通过（75 个测试文件、563 项测试；format、lint、typecheck、cycles、build 均通过）。真实浏览器下的三语、键盘焦点及设置流程人工核对未执行。

### Main Changes

(Add details)

### Git Commits

| Hash | Message |
|------|---------|
| `acee8197cf1a31a5895344ec3c476417c071c75c` | (see git log) |
| `4598408e0a6554b4155aeb80e488a6db1e078201` | (see git log) |

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 5: NPU fallback observability hard gate
<!-- trellis-session: v=2 fp=17d76d0b866ac037 -->

**Date**: 2026-10-02
**Task**: NPU fallback observability hard gate
**Branch**: `dev`

### Summary

Implemented explicit hardware fallback policy, platform_id-based resolution, and an unbypassable install self-test gate. Added actionable ModelLoad errors, three-state availability reporting, explicit local-tool Allow overrides, regression tests, and updated specs.

### Main Changes

- RKNN sessions now distinguish hardware-required failure from simulated fallback; self-test cannot be overridden.
- Development tools explicitly opt into simulated fallback without relying on .env.
- Documented policy precedence, 512-byte last_error ordering, and validation contract.

### Git Commits

| Hash | Message |
|------|---------|
| `1cae773` | feat(algo-sdk): enforce hardware-required fallback policy |
| `8d2599b` | docs(spec): document hardware fallback contract |
| `e57ced1` | chore(task): finalize NPU fallback observability artifacts |

### Testing

- [OK] Workspace: 977 tests passed; algo-sdk with rknn feature: 174 passed.
- [OK] macOS, RK3568, RK3576, RK3588 workspaces: 37 / 88 / 34 / 67 tests passed.
- [OK] fmt, workspace/platform clippy, Linux cross-target checks, mutation gate check, and git diff --check passed.

### Status

[OK] **Completed**

### Next Steps

- Continue the separate NPU core-allocation task from its planning phase.

## [2026-10-03] NPU 跨层协议基线、有界清理与路线 A/B 验证 (10-03-npu-abi-protocol-baseline)

### Summary

实现了 NPU 多核分配与卡亲和架构的基线协议（Subtask 1），包括 Worker 线程有界退出与隔离管理（QuarantineSupervisor）、C ABI Placement 扩展（`AvAlgoPlacementExtensionV1` 与双侧 POD 对齐断言）、Wire 放置配置安全剥离保护（`__heimdall_placement`），以及独立的板端验证探针工具（`tools/probe_rknn_dup_context/`）。

### Main Changes

- **Worker 有界清理与隔离**：重构 `crates/infer/src/worker.rs`，在 Worker 线程内部显式释放 runtime 与 backend 后再发送退出完成信号；对停机超时及通道断开引入 `QuarantineSupervisor`，移交后台非阻塞 Reaper 回收，杜绝无界 join() 与误判完成。
- **C ABI Placement 可选扩展**：在 `crates/infer/src/c_abi/types.rs` 与 `crates/algo-sdk/src/c_abi.rs` 中定义 `AvAlgoPlacementExtensionV1` 及 POD 结构（`AvAlgoPlacementCapsPod`、`AvAlgoInstanceReceiptPod`、`AvAlgoCleanupReceiptPod`），维持基础 `AvAlgoAbi` 96 字节不变，双侧通过静态与单元测试严格断言尺寸、对齐与字段偏移。
- **Wire 协议兼容性与配置安全剥离**：在 `crates/algo-sdk/src/macros.rs` 中增加 `deserialize_config_stripping_placement`，在插件配置反序列化前单次剥离 `__heimdall_placement`，保护标记有 `#[serde(deny_unknown_fields)]` 的业务插件。
- **T42 板端独立探针**：编写无宿主依赖的最小 C 探针工具 `tools/probe_rknn_dup_context/`，提供完整 Makefile、多线程并发压测、路线 A（独占移交）与路线 B（加锁 dup）双路线测试及内存查询。
- **自动化测试与回归矩阵**：落地 T01（停机超时隔离）、T02（创建失败隔离）、T03（通道断开隔离）、T13（Wire 兼容性与配置剥离）、T14（回执防御性解析）、T41（自检硬件硬门）全项测试。
- **规范同步**：更新 `.trellis/spec/algo-sdk/backend/algo-sdk-guidelines.md` 与 `.trellis/spec/infer/backend/inference-backends.md`。

### Testing

- [OK] `cargo fmt --all -- --check` 通过。
- [OK] `cargo clippy --all-targets -- -D warnings` 全工作区通过。
- [OK] `cargo nextest run --workspace` 1001 项测试全绿。
- [OK] `algo-packages/macos` 37 项测试全绿。
- [OK] `algo-packages/rknn/{rk3568, rk3576, rk3588}` cargo clippy 全部无告警通过。
- [OK] `probe_rknn_dup_context` 本地编译与静态自测运行通过。

### Status

[OK] **Completed**

### Next Steps

- [x] 推进子任务 2 (`10-03-npu-host-placement-ledger`)：构建宿主 NPU 拓扑探测、核心分配账本与世代屏障。

## [2026-10-03] NPU 双层账本、放置求解与生命周期状态机 (10-03-npu-host-placement-ledger)

### Summary

实现了 NPU 多核分配与卡亲和架构的宿主资源追踪与求解器（Subtask 2），包括硬件设备拓扑建模与代际感知（`DeviceInventory`）、确定性放置求解器（`PlacementSolver`，支持 Manual 严格校验、Auto spread 轮转均衡与 pack 复用）、执行/权重双层短锁账本（`PlacementLedger`）及门面接口（`PlacementManager`），全面落地防雪崩隔离熔断器与 T06–T12, T19 自动化测试矩阵。

### Main Changes

- **硬件设备拓扑建模与代际感知 (`DeviceInventory`)**：
  - 在 `crates/infer/src/npu/inventory.rs` 中抽象 `DeviceTopology` 与 `DeviceInventory`，支持动态代际递增（`topology_generation`）；
  - 覆盖多平台拓扑：RK3588 3核（0b111掩码）、RK3576 2核（0b011掩码）、RK3568 单核（0b001掩码）及无核心切分平台。
- **确定性放置求解器 (`PlacementSolver`)**：
  - 在 `crates/infer/src/npu/solver.rs` 中实现纯 CPU 确定性求解逻辑（<1ms）：
  - `Manual` 模式：严格校验目标设备 ID、核心索引及设备核划分能力（越界/不支持即刻报错）；
  - `Auto` 模式：`spread` 策略根据当前核心负载均衡分配，同分时基于累加计数器确定性轮转；`pack` 策略优先复用已有权重与同组核心；单核自动退化为 `runtimeManaged`；
  - 严格离线 Worker 配额隔离，防止离线重提取任务挤占实时视频推理流。
- **双层短锁并发账本 (`PlacementLedger`)**：
  - 在 `crates/infer/src/npu/ledger.rs` 中使用 `parking_lot::Mutex` 短锁保证状态更新 <50µs，持锁内杜绝 IO/FFI/await；
  - `ExecutionLedger`：跟踪 `ExecutionReservation`（`Reserved` -> `Ready` -> `Quarantined`/`Released`），支持实例启动前取消无残留原子回滚；
  - `WeightLedger`：以 `(device_id, core_mask, model_key)` 为物理权重管理键，提供单航班加载插槽（single-flight loading slot），并发等待与 future 取消不连坐正在进行的加载；
  - 宽容代际释放：硬件拓扑更新代际增加时，旧代际产生的 reservation 仍可平滑无损释放。
- **防雪崩隔离熔断器 (`QuarantineBreaker`)**：
  - 账本准入主动联动 `quarantined_workers_count()`，当底层驱动卡顿导致的隔离 Worker 数超标时，立即熔断拒绝新实例准入。
- **统一门面 (`PlacementManager`)**：
  - 在 `crates/infer/src/npu/manager.rs` 中封装 Inventory、Solver 与 Ledger，导出运行态监控快照 `PlacementLedgerSnapshot`。
- **自动化测试矩阵 (T06–T12, T19)**：
  - 在 `crates/infer/tests/placement_ledger_tests.rs` 中完整覆盖取消回滚、并发单航班加载、Auto spread 同分轮转、Manual 优先与离线配额、隔离实例防复用、拓扑代际平滑释放、防雪崩熔断及冷却期定时器世代复用。
- **规范同步**：更新 `.trellis/spec/infer/backend/inference-backends.md`。

### Testing

- [OK] `cargo fmt --all -- --check` 通过。
- [OK] `cargo clippy --all-targets -- -D warnings` 全工作区通过。
- [OK] `cargo nextest run --workspace` 1016 项测试全绿。
- [OK] `algo-packages/macos` 37 项测试全绿。
- [OK] `algo-packages/rknn/{rk3568, rk3576, rk3588}` cargo clippy 全部无告警通过。

### Status

[OK] **Completed**

### Next Steps

- 归档子任务 2 (`10-03-npu-host-placement-ledger`)，推进后续子任务 3（算法包热加载与物理权重复用）。

## [2026-10-03] SDK placement 注入与 RKNN 共享权重运行时 (10-03-algo-sdk-rknn-shared-weights)

### Summary

实现了 NPU 多核分配与卡亲和架构的 SDK 注入与 RKNN 共享权重运行时（Subtask 3），包括 `InitContext` 放置元数据扩展、`rknn_dup_context` 物理权重共享与 Route A 独占移交所有权模型、`AvAlgoPlacementExtensionV1` C ABI 虚表与回执自动上报机制、彻底剥离进程级 `RKNN_CORE_MASK` 环境变量覆盖，以及彻底消除 RK3588 人脸识别算法包内的全局串行 Actor 和邮箱队列，达成真正多实例直通多核并行推理。

### Main Changes

- **`InitContext` 放置元数据扩展与 Builder**：
  - 在 `crates/algo-sdk/src/plugin.rs` 增加 `wire_placement: Option<WirePlacementMetadata>`、`with_placement()` 与 `target_core_mask()`；
  - 同步更新了 `crates/algo-sdk/src/models/yolo.rs` 与各算法包所有调用点；
  - 在 `crates/algo-sdk/src/macros.rs` 中自动从配置私有字段 `__heimdall_placement` 解析并注入 `InitContext`。
- **C ABI Placement 扩展与回执上报**：
  - 在 `crates/algo-sdk/src/macros.rs` 中实现 `av_algo_get_placement_extension` 导出；
  - 实例创建成功后生成 `AvAlgoInstanceReceiptPod`（ACKNOWLEDGED 状态），实例销毁时生成 `AvAlgoCleanupReceiptPod`（CLEANED 状态）并记录入全局循环回执表。
- **RKNN 共享权重运行时与 Route A 独占移交**：
  - 在 `crates/algo-sdk/src/runtime/platforms/rockchip.rs` 中绑定 `rknn_dup_context` C 符号；
  - 实现 `RknnRootWeight` RAII 句柄与引用计数，支持在独立会话上派生子上下文并立即调用 `rknn_set_core_mask`；
  - 子会话 `RknnSession` 拥有 `Send + !Sync` 特征，严格移交给独占 OS Worker 线程执行；
  - 彻底移除 `std::env::var("RKNN_CORE_MASK")` 进程级环境变量修改，核心绑定完全收敛在会话级别。
- **消除人脸识别全局串行 Actor (`algo-packages/rknn/rk3588/face_recognition`)**：
  - 废弃全局串行 `InferenceWorker` 与跨实例邮箱队列；
  - 实现弱引用根权重池 `SharedModelRoots`，各 `FaceRecognizer` 实例直接持有专属的 `FaceSessions`，在实例专用 OS Worker 线程中直接同步执行 YOLOv8-Face、EdgeFace 等推理，实现完全无竞争的真正硬件多核并行。
- **自动化测试矩阵**：
  - 编写 `crates/algo-sdk/tests/shared_weights_tests.rs` 覆盖 Builder 提取、实例级分核无环境变量副作用、自检硬件硬门拦截以及 Placement 扩展回执生命周期；
  - 确保 macOS、RK3568、RK3576、RK3588 所有算法包全部编译通过且单元测试通过（RK3588 67/67 通过）。
- **规范同步**：更新 `.trellis/spec/algo-sdk/backend/algo-sdk-guidelines.md`。

### Testing

- [OK] `cargo fmt --all -- --check` 通过。
- [OK] `cargo clippy --all-targets -- -D warnings` 全工作区与各平台算法包通过。
- [OK] `cargo nextest run --workspace` 1020 项测试全绿。
- [OK] `algo-packages/macos` 37 项测试全绿。
- [OK] `algo-packages/rknn/rk3568` 88 项测试全绿。
- [OK] `algo-packages/rknn/rk3576` 34 项测试全绿。
- [OK] `algo-packages/rknn/rk3588` 67 项测试全绿。

### Status

[OK] **Completed**

### Next Steps

- 推进子任务 4 (`10-03-npu-persistence-revision-barrier`)：配置持久化、版本栅栏、局部故障隔离与端到端回归。

## [2026-10-03] NPU 亲和配置持久化、版本栅栏与单实例局部隔离 (10-03-npu-persistence-revision-barrier)

### Summary

实现了 NPU 亲和配置持久化、版本栅栏、宿主 Coordinator 局部故障隔离与原子事务（Subtask 4），包括 `V25` 数据库迁移与 `affinity_json` 字段支持、单实例版本栅栏与两阶段配置收敛、`TaskRepo::save_task_with_instances` 内原子 `stream_mode` 持久化、显式 `instance_id` 批次与跨任务安全校验、冷启动多实例单故障隔离不连坐机制，以及覆盖 T04/T05/T15-T18/T22-T24/T30/T39/T40 的端到端集成测试矩阵。

### Main Changes

- **类型系统与校验 (`crates/types`)**：
  - 增加 `TypeError::InvalidAffinity`；
  - 为 `AffinityIntent` 增加 `normalized()`、`is_equivalent_to()` 与 `is_affinity_equivalent()` 比较逻辑；
  - `TaskAlgorithmInstanceConfig` 增加 `affinity: Option<AffinityIntent>`，在 `validate()` 中校验合法性（拒绝空 `deviceId`、非法 `policy`）。
- **数据库 Schema 迁移与实体 (`crates/db`)**：
  - 添加 `V25__algorithm_instance_affinity.sql`，给 `algorithm_instances` 增加 `affinity_json TEXT NOT NULL DEFAULT '{"mode":"auto","policy":"spread"}'`；
  - 更新实体 `algorithm_instance::Model` 与 `affinity_intent()` 辅助解析方法；
  - `AlgorithmInstanceRepo` 的 `mark_apply_applied`、`mark_apply_pending` 与 `mark_apply_failed` 施加单实例版本栅栏（`WHERE instance_id = ? AND desired_revision = target_revision`），受并发修改时返回 `Ok(None)` 杜绝过期覆盖。
- **任务仓储事务原子性与实例校验 (`crates/db/src/repository/task.rs`)**：
  - `SaveTaskWithInstancesParams` 增加 `stream_mode: Option<StreamMode>`，在单一 SQLite 事务内原子持久化 `cameras.stream_mode`，失败整体回滚；
  - 校验显式 `instance_id`：批次唯一性、同算法一致性、跨任务防篡改；
  - 比较新旧亲和意图，仅在非等价亲和意图发生改变时推进 `desired_revision += 1` 并将状态置为 `Pending`，等价自动策略不造成颠簸；
  - 缺省 `instance_id` 时自动匹配同算法已有实例，保留已有 ID 与亲和意图。
- **管线冷启动单实例故障隔离 (`crates/pipeline/src/coordinator.rs`)**：
  - `InstanceLaunchConfig` 增加 `desired_revision` 字段与 `with_desired_revision` 链式方法；
  - `start_camera_pipeline_inner` 在冷启动时遍历算法实例，遇到租约获取或 Worker 创建失败时记录单实例故障并继续初始化其余实例；仅当该路所有实例均失败时才整路回滚，避免局部故障连坐全路视频分析；
  - 成功启动后通过 `params.instances.retain(...)` 过滤有效实例，使运行时信息真实反映活跃实例。
- **HTTP API DTO 与路由 (`crates/api/src/routes/task.rs`)**：
  - `TaskAlgorithmInstanceDto` 与 `TaskAlgorithmInstanceSummaryDto` 增加 `affinity: Option<AffinityIntent>` 字段；
  - `save_task_config_handler` 移除独立的 `update_stream_mode`，直接传递 `dto.stream_mode` 进入任务事务原子更新；
  - `sync_pipeline_with_models` 将已持久化的 `desired_revision` 传递至 `InstanceLaunchConfig`。
- **全方位测试矩阵**：
  - `crates/db/tests/migration_tests.rs`: `test_v25_migration_adds_affinity_json_with_default`（T39 默认值与向后兼容）；
  - `crates/db/tests/affinity_repo_tests.rs`: T04（过期版本栅栏拒绝）、T05（原子 stream_mode 回滚与持久化）、T17/T18（等价亲和防颠簸与亲和变更代际推进）、T22/T23（显式 instance_id 校验）；
  - `crates/pipeline/tests/coordinator_partial_failure_tests.rs`: T24（冷启动单实例隔离与全故障回滚）；
  - `crates/api/tests/affinity_persistence_barrier_tests.rs`: T15/T16（HTTP API 亲和校验）、T30/T40（DTO 驼峰序列化与版本冲突 40903 拦截）。
- **规范同步**：
  - 同步更新 `.trellis/spec/api/backend/api-guidelines.md` 与 `.trellis/spec/infer/backend/inference-backends.md`。

### Testing

- [OK] `cargo fmt --all -- --check` 通过。
- [OK] `cargo clippy --all-targets -- -D warnings` 全工作区与各平台算法包通过。
- [OK] `cargo nextest run --workspace` 1029 项测试全绿。
- [OK] `algo-packages/macos` 37 项测试全绿。
- [OK] `algo-packages/rknn/rk3568` 88 项测试全绿。
- [OK] `algo-packages/rknn/rk3576` 34 项测试全绿。
- [OK] `algo-packages/rknn/rk3588` 67 项测试全绿。

### Status

[OK] **Completed**

### Next Steps

- 归档子任务 4 (`10-03-npu-persistence-revision-barrier`)，推进子任务 5（API 观测与前端可视化）。
