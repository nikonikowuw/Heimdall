# NPU 双层账本、放置求解与生命周期状态机技术设计

> 所属任务：`10-03-npu-host-placement-ledger`（阶段 B）。

## 1. 架构总览

```text
Pipeline / Ingestion
       │
       ▼ (发起实例创建 / 请求放置)
PlacementManager (Facade)
  ┌────────────────────────────────────────────────────────┐
  │ Mutex<LedgerState> (短持有锁，严禁锁内 IO/FFI/await)      │
  │   ├── DeviceInventory (NPU 设备拓扑、可用核心、代际)     │
  │   ├── PlacementSolver (纯算法，Auto/Manual 分核计算)    │
  │   ├── ExecutionLedger (执行配额、核心占用、实例上限)     │
  │   └── WeightLedger (模型物理权重驻留、引用计数、单航班)  │
  └────────────────────────────────────────────────────────┘
       │
       ▼ (原子获取 PlacementReservation)
Worker / Instance 创建流程 (锁外异步执行)
       │
       ├── 成功: ledger.mark_ready(reservation_id)
       ├── 失败/取消: ledger.release(reservation_id)
       └── 驱动挂起: 移入 QuarantineSupervisor，占额直到彻底释放
```

---

## 2. 模块划分与核心数据结构

### 2.1 DeviceInventory 拓扑模型 (`crates/infer/src/npu/inventory.rs`)
```rust
#[derive(Debug, Clone)]
pub struct DeviceTopology {
    pub device_id: String,
    pub device_type: NpuDeviceType,
    pub core_count: u32,
    pub supported_core_mask: u32,
    pub supports_core_pinning: bool,
    pub memory_total_mb: u64,
}

#[derive(Debug, Clone)]
pub struct DeviceInventory {
    pub topology_generation: u64,
    pub devices: Vec<DeviceTopology>,
}
```

- **拓扑代际 (`topology_generation`)**：系统启动或发生硬件设备插拔/变更时自增；
- **旧资源释放原则**：无论当前拓扑代际为何值，释放请求只要持有当时有效的 `reservation_id`，均允许正常注销与扣减，严禁因拓扑代际变化而拒绝合法资源释放。

### 2.2 PlacementSolver 求解算法 (`crates/infer/src/npu/solver.rs`)
```rust
pub struct PlacementSolver;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacementDecision {
    pub device_id: String,
    pub runtime_device_index: u32,
    pub core_mask: u32,
    pub strategy: String, // "pinned" | "runtimeManaged" | "degraded"
}

impl PlacementSolver {
    pub fn solve(
        inventory: &DeviceInventory,
        ledger: &ExecutionLedger,
        intent: &AffinityIntent,
        is_offline: bool,
    ) -> Result<PlacementDecision, PlacementError> {
        // 1. Manual 模式严格校验
        // 2. Auto 模式按 Spread / Pack 策略基于 ledger 当前各核心占用数求解
        // 3. 检查 core / device / offline 配额上限
    }
}
```

### 2.3 ExecutionLedger 与 WeightLedger (`crates/infer/src/npu/ledger.rs`)
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReservationId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WeightKey {
    pub model_key: String,
    pub device_id: String,
    pub package_generation: u64,
}

pub struct ExecutionLedger {
    pub reservations: HashMap<ReservationId, ExecutionReservation>,
    pub core_allocations: HashMap<(String, u32), u32>, // (device_id, core_mask) -> active_count
    pub active_offline_count: u32,
    pub max_instances_per_core: u32,
    pub max_offline_workers: u32,
}

pub struct WeightLedger {
    pub owners: HashMap<WeightKey, WeightOwnerEntry>,
    pub loading_slots: HashMap<WeightKey, Arc<tokio::sync::watch::Sender<LoadingState>>>,
}
```

- **单航班加载 (Single-Flight)**：并发多实例请求同一 `WeightKey` 时，第一个实例原子创建 `loading_slot`；后至实例监听 watch 通道等待完成，不发起重复的模型加载；若某个等待者的 future 被取消，仅退出自身等待，不影响后台正在进行的加载任务。

---

## 3. 并发安全与锁策略

1. **绝对不在锁内执行 IO、FFI、`.await` 或 `join()`**：
   - `PlacementManager` 内部保护账本状态的互斥锁（`parking_lot::Mutex<LedgerState>`）仅在内存哈希表增删、计数自增、决策计算期间持有（耗时 < 50 微秒）；
   - 获取 `Reservation` 后立即释放锁；
2. **两阶段提交与原子回滚**：
   - **Phase 1: Reserve**（持锁）：扣减可用槽位，生成 `ReservationId`；
   - **Phase 2: Prepare / Open**（锁外）：启动线程或加载权重；
   - **Phase 3: Commit / Rollback**（短持锁）：成功则标记 `Ready`；取消或失败则调用 `release` 恢复额度。

---

## 4. 熔断机制 (Quarantine Breaker)

- 监控 `quarantined_workers_count()`；
- 当处于隔离状态（线程挂起、无法确认清理）的工作线程数量达到配置上限（默认 5）时：
  - `PlacementSolver` 立即熔断并返回 `PlacementError::QuarantineBreakerTriggered`；
  - 拒绝一切新准入（包括在线视频流新任务、离线特征重提取与热态保活任务）；
  - 已在运行中的正常任务不受影响；
  - 待隔离线程被 Reaper 彻底回收并降回安全线后，自动恢复准入。

---

## 5. 错误分类 (`crates/infer/src/npu/error.rs`)

```rust
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum PlacementError {
    #[error("未找到可用 NPU 设备: {0}")]
    DeviceNotFound(String),

    #[error("指定的 NPU 核心索引越界或不支持: core {core_index} on {device_id}")]
    InvalidCoreIndex { device_id: String, core_index: u32 },

    #[error("设备 {0} 物理上不支持核心绑定")]
    CorePinningNotSupported(String),

    #[error("核心 {core_index} 配额已满 (上限: {max})")]
    CoreQuotaExceeded { core_index: u32, max: u32 },

    #[error("设备 {device_id} 配额已满 (上限: {max})")]
    DeviceQuotaExceeded { device_id: String, max: u32 },

    #[error("离线推理 Worker 配额已满 (上限: {max})")]
    OfflineQuotaExceeded { max: u32 },

    #[error("系统存在过多挂起隔离线程，触发防雪崩熔断")]
    QuarantineBreakerTriggered,

    #[error("未找到有效的资源预留: {0:?}")]
    ReservationNotFound(ReservationId),
}
```
