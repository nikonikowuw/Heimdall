# NPU 双层账本、放置求解与生命周期状态机实施计划

> 所属任务：`10-03-npu-host-placement-ledger`（阶段 B）。

## 1. 实施检查清单

### Phase 1: 失败测试基线先行（T06–T12, T19）
- [x] 在 `crates/infer/tests/placement_ledger_tests.rs` 中编写测试：
  - `test_reservation_cancel_before_start_releases_budget_t06`: reservation 产生后线程启动前取消，断言预算原子回滚，无残留占用；
  - `test_weight_loading_future_cancellation_retains_owner_t07`: 模拟权重加载中等待 future 取消，断言正在进行的加载任务不连坐，配额不丢；
  - `test_concurrent_auto_spread_round_robin_t08`: 并发多实例在 3 核设备上申请 Auto，断言 3 核心负载均衡且同分轮转无偏角；
  - `test_manual_pinning_and_offline_quota_isolation_t09`: 验证同批 Manual 优先准入，以及离线 Worker 配额隔离；
  - `test_quarantined_instance_replacement_blocks_reuse_t10`: 验证处于隔离状态的实例不可被同组直接复用；
  - `test_topology_generation_advancement_preserves_old_release_t11`: 模拟设备发生拓扑变更，断言旧代际持有的 reservation 依然能正常释放；
  - `test_quarantine_breaker_triggers_and_rejects_new_admissions_t12`: 模拟隔离线程达到阈值，断言新请求被安全拦截；
  - `test_cooling_weight_reuse_and_generational_safe_timer_t19`: 模拟冷却期内被新任务接入，验证定时器作废并安全复用。

### Phase 2: DeviceInventory 拓扑建模与探测
- [x] 在 `crates/infer/src/npu/inventory.rs` 中定义 `DeviceTopology`、`DeviceInventory` 与核心掩码计算逻辑。
- [x] 接入平台探测（RK3588 3核掩码 0b111、RK3576 2核掩码 0b011、RK3568 单核掩码 0b001、Mock 平台）。
- [x] 实现拓扑代际自增与查询。

### Phase 3: PlacementSolver 放置求解器
- [x] 在 `crates/infer/src/npu/solver.rs` 中实现 `PlacementSolver`：
  - `Manual` 严格校验逻辑；
  - `Auto` 算法：实现 `Spread`（负载优先 + 轮转）与 `Pack` 策略；
  - 单核设备自动退化为 `runtimeManaged`。
- [x] 导出 `PlacementDecision` 与 `PlacementError`。

### Phase 4: ExecutionLedger 与 WeightLedger 双层账本
- [x] 在 `crates/infer/src/npu/ledger.rs` 中实现：
  - 短持有互斥锁 `parking_lot::Mutex<LedgerState>`；
  - `ExecutionLedger`：`reserve`、`commit_ready`、`release`；每核心与每设备上限控制；离线 Worker 配额追踪；
  - `WeightLedger`：基于 `WeightKey` 的引用计数与单航班 `loading_slot`；
  - 熔断器：集成 `quarantined_workers_count()`，超限拒绝准入。

### Phase 5: PlacementManager 统一门面
- [x] 在 `crates/infer/src/npu/manager.rs` 或 `crates/infer/src/npu/mod.rs` 中提供统一 `PlacementManager`：
  - 组合 Inventory、Solver 与 Ledger；
  - 暴露两阶段准入与释放接口；
  - 导出运行态监控快照 `PlacementLedgerSnapshot`。

### Phase 6: 全量质量门禁验证
- [x] 运行 Phase 1 的所有测试用例（T06–T12, T19），确保全部转绿。
- [x] 运行 `cargo fmt --all -- --check`。
- [x] 运行 `cargo clippy --all-targets -- -D warnings`。
- [x] 运行 `cargo nextest run --workspace`。

---

## 2. 门禁命令与验收标准

### 格式化与静态检查
```bash
cargo fmt --all -- --check
cargo clippy -p heimdall-infer -p heimdall-types --all-targets -- -D warnings
```

### 自动化测试
```bash
cargo nextest run -p heimdall-infer --test placement_ledger_tests
cargo nextest run --workspace
```

### 验收判定
1. T06–T12 与 T19 自动化用例全绿；
2. 账本操作全程无锁内 IO / FFI / await / join；
3. 隔离熔断机制有效生效，旧代际释放零拒绝。
