# NPU 双层账本、放置求解与生命周期状态机需求 (PRD)

> 所属母任务：`10-01-npu-core-allocation`（阶段 B）。

## 1. 目标与范围

本任务聚焦于宿主侧（`crates/infer`）的**NPU 拓扑探测、核心放置求解算法、双层内存/容量账本与生命周期状态机**，为多实例分核调度与物理权重复用提供无锁竞争、原子准入与故障隔离的核心底座。

### 本期范围
1. **DeviceInventory 拓扑探测与设备模型**：
   - 探测并统一描述系统 NPU 拓扑（RK3588 3核、RK3576 2核、RK3568 单核、无 NPU 降级）；
   - 设备身份规范化与核心掩码建模（`0b001`, `0b010`, `0b100` 等）；
   - 拓扑代际追踪（`topology_generation`），设备变化递增代际，但不中断旧代际合法资源的清理。
2. **PlacementSolver 放置求解算法**：
   - 纯函数/无锁设计：基于当前可用拓扑与账本状态计算最优分配；
   - 严格 `Manual` 分配：指定设备与核心，超界或设备不支持设核时快速拒绝；
   - 智能 `Auto` 分配：
     - `Spread`（打散）：优先分配当前实例计数最少的核心，同分轮转（Round-Robin），保证多核均衡；
     - `Pack`（紧凑）：优先复用已有负载核心，减少核心唤醒；
   - 输出符合 Wire 协议的 `PlacementDecision`。
3. **ExecutionLedger 与 WeightLedger 双层账本**：
   - 短锁（`Mutex<LedgerState>`）内原子预留执行资源与缺失的权重根预算；
   - 锁外执行异步初始化与低频 FFI 操作，杜绝持锁发生 IO / FFI / await / join；
   - 权重单航班（Single-Flight）：并发多实例请求同一模型权重时，原子登记唯一 loading slot，锁外初始化，等待者取消不连坐；
   - 容量硬上限控制：支持每核心最大实例数、每设备最大实例数、离线 Worker 与在线任务隔离配额。
4. **生命周期状态机与 Quarantined 熔断**：
   - 状态流转：`Reserved -> Initializing -> Ready <-> Cooling -> Draining -> Released / Quarantined`；
   - 隔离熔断（Quarantine Circuit Breaker）：当隔离 Worker 数量达到阈值（默认 5）时，停止新准入（含离线与 warmup），保护主机；
   - 资源原子回滚：在 `Reserved` 阶段取消立即回滚；`Initializing` 阶段取消移交后台安全收敛。
5. **故障回归测试矩阵 (T06–T12, T19)**：
   - 覆盖预留取消、初始化取消保活、多核均衡轮转、Manual 越界拒绝、拓扑换代兼容、隔离熔断与代际退火复用。

### 不在本期范围
- 插件底层 `rknn_dup_context` 真实调用（由 `10-03-algo-sdk-rknn-shared-weights` 负责）；
- SQLite 持久化表结构与 REST HTTP API（由 `10-03-npu-persistence-revision-barrier` 负责）；
- 板端 8 小时长稳压测（由 `10-03-npu-board-verification-release` 负责）。

---

## 2. 需求列表

### R01 — 统一设备拓扑与代际管理
* 系统启动时自动探测 NPU 设备（支持 Rockchip、Ascend 与开发机 Mock 平台）；
* 暴露静态与运行态拓扑快照：设备 ID、设备类型、物理核心数、支持的核心掩码、是否支持运行时核心绑定；
* 维护 `topology_generation: u64`，拓扑发生变化时自增，任何代际的 reservation 均可按其原代际正常释放。

### R02 — 确定性放置求解算法 (PlacementSolver)
* `Manual` 模式：严格按目标 `device_id` 与 `core_index` 校验并分配，核心不支持或索引非法时明确报错；
* `Auto` 模式：
  * 单核 NPU（如 RK3568）：退化为 `core_mask = 0` / runtimeManaged；
  * 多核 NPU（如 RK3588/RK3576）：在符合约束的核心池中按 `policy` 求解，默认 `Spread` 策略下优先最低负载核心，相同负载下单调自增轮转；
* 求解过程必须为纯 CPU 逻辑，耗时 < 1ms，不涉及任何外设 IO。

### R03 — 双层账本原子预留与单航班加载
* `ExecutionLedger`：按 `reservation_id` 追踪执行会话槽位，严格保障每个核心/设备的实例数不超过配置上限；
* `WeightLedger`：按 `WeightKey`（模型 key + 设备 ID + 制品代际）追踪物理权重驻留，同模型多实例仅登记引用计数，不重复分配权重内存；
* 首次请求同模型权重时，通过单航班机制合并并发初始化，避免重复触发重型模型加载；某等待实例取消时不中断正在加载的根所有权。

### R04 — 离线与在线配额隔离
* 独立的离线配额控制（`max_offline_workers`），离线特征提取等后台批量任务不可挤占在线视频流分析任务的核心与槽位；
* 离线任务使用独立的 `execution_group_id`。

### R05 — 熔断保护与安全状态机
* 接入 `crates/infer/src/worker.rs` 的 `quarantined_workers_count()`；当隔离线程达到上限（默认 5）时，账本触发熔断，拒绝新准入；
* 状态机生命周期：
  * `Reserved`：资源预留成功，未启动线程；取消直接释放；
  * `Initializing`：线程启动中；取消时标记意图，等待线程就绪或隔离后释放；
  * `Ready`：实例运行中；
  * `Cooling`：权重引用归零，进入代际冷却定时退火；若退火前被新实例借用，作废旧定时器并复用；
  * `Quarantined`：异常隔离状态，占额不释放；确认清理后转 `Released`。

---

## 3. 验收准则 (Acceptance Criteria)

- [ ] **AC01** (对应原 AC01, AC03): 3 核设备上 3 个同优先级 Auto 实例分别分配至 Core 0、Core 1、Core 2，同分轮转无偏角；单核设备正确回退。
- [ ] **AC02** (对应原 AC04, AC18): 在 `Reserved` 或 `Initializing` 阶段取消任务，账本原子回滚释放，无额度泄漏。
- [ ] **AC03** (对应原 AC05, AC14): `Manual` 模式指定越界核心或在不支持设核设备上设核时被精确拒绝；同批 `Manual` 优先于 `Auto` 准入。
- [ ] **AC04** (对应原 AC08, AC21): 并发 5 个实例请求同一模型权重时，只触发一次根权重预留；部分请求取消不影响其余实例。
- [ ] **AC05** (对应原 AC12, AC22): 隔离线程达到阈值时触发熔断，新请求被拒绝且不影响已有实例；旧代际拓扑的清理请求被正确接受。
