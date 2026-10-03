# NPU 跨层协议、有界清理与路线 A/B 验证实施计划

> 所属任务：`10-03-npu-abi-protocol-baseline`（阶段 A0 & A）。

## 1. 实施检查清单

### Phase 1: 稳定失败测试基线先行（T01–T05, T13–T14, T41）
- [x] 在 `crates/infer/src/worker.rs` 中编写 `test_worker_exit_timeout_isolation_t01`（模拟 backend Drop 挂起，断言在规定超时后完成隔离并释放主线程，对应 T01）。
- [x] 编写 `test_worker_factory_hung_thread_quarantined_t02`（模拟 runtime 创建失败且析构阻塞，断言不发生无界 join，对应 T02）。
- [x] 编写 `test_worker_exit_channel_disconnect_not_completed_t03`（模拟 startup/exit 通道异常断开，断言不虚假报已完成，对应 T03）。
- [x] 编写 `test_wire_placement_compatibility_t13`（覆盖宿主与插件新旧 4 种组合，包含 `deny_unknown_fields` 插件的回归测试，对应 T13）。
- [x] 编写 `test_corrupted_cleanup_receipt_handling_t14`（覆盖错版本、错代际、超长/截断回执的防御性解析，对应 T14）。
- [x] 编写 `test_self_test_fallback_hard_gate_t41`（验证注入 placement 且 `is_self_test=true` 时，策略恒为 `RequireHardware`，对应 T41）。

### Phase 2: Worker 退出协议重构与有界隔离
- [x] 重构 `crates/infer/src/worker.rs`：
  - 修改 `InferenceWorker::stop` 与 run loop，确保在 Worker 线程内部显式调用 backend 清理逻辑，清理完毕后通过 oneshot 发送退出确认；
  - 引入 `QuarantineSupervisor`，当停机超时或通道异常断开时，将线程 JoinHandle 移交 supervisor 进行后台观察，主流程立即返回超时错误；
  - Reaper 线程在锁外执行已隔离句柄的轮询回收。
- [x] 验证 Phase 1 的 T01–T03 失败测试全部转绿。

### Phase 3: C ABI Placement 可选扩展与双侧断言
- [x] 在 `crates/infer/src/c_abi/` 中定义 `AvAlgoPlacementExtensionV1` 及配套 POD 结构（`AvAlgoPlacementCapsPod`、`AvAlgoInstanceReceiptPod`、`AvAlgoCleanupReceiptPod`）。
- [x] 在 `crates/algo-sdk/src/` 中定义对应的 C ABI 垫片结构体。
- [x] 编写双侧单元测试，断言基础 `AvAlgoAbi` 保持 96 字节，断言扩展 POD 结构体的 `size_of`、`align_of` 和关键字段偏移完全一致。
- [x] 改造 `crates/algo-sdk/src/macros.rs`：在反序列化业务配置前单次剥离 `__heimdall_placement`，保护 `deny_unknown_fields` 业务插件。
- [x] 验证 Phase 1 的 T13–T14、T41 失败测试全部转绿。

### Phase 4: A0 目标板实测与选路报告（T42，解除 B-R14）
- [x] 编写最小独立板端探针程序 `probe_rknn_dup_context`（不依赖完整宿主系统，包含完整 C 源码、Makefile 与使用指南于 `tools/probe_rknn_dup_context/`）：
  - 加载模型并初始化 root context；
  - 控制线程调用 `rknn_dup_context(&root, &child)`；
  - 将 child 移交给独立 Worker 线程，调用 `rknn_set_core_mask` 并进行推理；
  - 兄弟 Worker 线程并发在不同核心推理；
  - 销毁单个 child，验证兄弟线程不受影响；
  - 销毁所有 child 后销毁 root。
- [ ] 在真实开发板（RK3588/RK3576）运行探针，记录返回码、耗时与内存行为（在子任务 5 / 板端验证阶段上机运行）。
- [ ] 若路线 A 通过，确认基线；若出现跨线程 TLS 段错误，按路线 B（实例内加锁 dup）实测后备方案。
- [ ] 产出 A0 实验报告并入档，闭环消除 `B-R14` 阻断。

---

## 2. 故障与回归验证矩阵

| 测试编号 | 场景 / 注入 | 必须断言 | 对应 AC |
| :--- | :--- | :--- | :--- |
| **T01** | 推理正常、backend Drop 阻塞 | 停止期限内隔离、真实清理前占额、主线程不卡死 | AC02 |
| **T02** | factory 成功后 runtime 创建失败且析构阻塞 | Err 分支不发生无界 join | AC02 |
| **T03** | startup/exit 通道断开、panic/TLS 析构延迟 | 通道断开不等于资源释放；不虚报已完成 | AC02 |
| **T13** | 无能力/拒绝未知 JSON 的旧插件、新插件无注入 | 新旧四组合正确协商，旧插件正常运行，新插件本地无注入默认运行 | AC01, AC05 |
| **T14** | 错版本/错组/错代际/截断/超长应用或清理回执 | 防御性拦截，不超分配，不读已销毁指针 | AC01, AC05 |
| **T41** | placement 注入 + `is_self_test=true`；亲和降级且无运行时 | 策略恒为 `RequireHardware`；不产生模拟会话；错误明确 | AC04 |
| **T42** | 首选路线 A 实测（控制面 dup 后独占移交、实例线程设核与推理）；后备路线 B 验证 | 4 项判定标准明确，选定路线下无跨线程并发误用，出具硬件证据 | AC03 |

---

## 3. 门禁命令与验收标准

### 格式化与静态检查
```bash
cargo fmt --all -- --check
cargo clippy -p heimdall-infer -p heimdall-types -p heimdall-algo-sdk --all-targets -- -D warnings
```

### 自动化单元测试
```bash
cargo nextest run -p heimdall-infer -p heimdall-types -p heimdall-algo-sdk
```

### 验收判定
1. 基础 `AvAlgoAbi` 64 位 96 字节测试通过；
2. T01–T05, T13–T14, T41 自动化用例全绿；
3. A0 实测探针产出有效结论，`B-R14` 阻断项被解除。
