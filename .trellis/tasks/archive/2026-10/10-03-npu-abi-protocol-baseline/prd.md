# NPU 跨层协议、有界清理与路线 A/B 验证基线

> 状态：planning / 修订草案；所属母任务：`10-01-npu-core-allocation`（阶段 A0 & A）。

## 1. 目标与范围

作为母任务 `10-01-npu-core-allocation` 的基础协议与故障隔离交付单元，本任务聚焦于**无硬件强依赖的跨层契约冻结、Worker 有界退出协议重构，以及 A0 目标板路线 A/B 的实测验证**。

### 本期范围
1. **基础 C ABI 稳定与可选 Placement 扩展**：
   - 严格保持 `AvAlgoAbi` 现有 64 位 96 字节基础布局及既有函数指针签名完全不变；
   - 建立可选 C ABI 扩展规范（类似 gallery optional symbol 机制），定义执行隔离/共享权重能力查询、权重根 prepare/release、实例创建回执、以及双层清理回执（POD）布局；
   - 建立双侧 `size_of`、`align_of` 及关键字段偏移量断言。
2. **Worker 退出确认与析构隔离（修复 H01 / T01–T03）**：
   - 重构 `crates/infer/src/worker.rs`：backend 清理必须在所属专用 Worker 线程内完成后再发送退出确认信号；
   - 停止流程、初始化失败、通道断开共用有界超时清理机制；TLS 析构阻塞或 FFI 挂起时转入隔离 supervisor，禁止无界 `join()`；Reaper 在锁外回收。
3. **Wire 数据协议与新旧版本兼容（T13–T14）**：
   - 定义宿主下发给插件的 `__heimdall_placement` 协议（内部 snake_case，与 REST camelCase 分离）；
   - 建立宿主与插件新旧 4 种组合的兼容矩阵测试（新宿主+新插件、新宿主+旧插件、老宿主+新插件、老宿主+旧插件）；
   - 覆盖错版本、错组、错代际、截断和超长清理回执的容错与解析。
4. **硬件回退策略交汇回归（T41）**：
   - 确保 placement 注入后 `RequireHardware` 策略不可被翻越，自检硬门恒定生效，不产生未授权的 `debug_cpu_fallback_path` 模拟会话。
5. **A0 目标板实测方案与路线 A/B 判定（T42，解除 B-R14）**：
   - 优先在真实 BSP 上实测首选路线 A（控制面 dup 后以 `Send + !Sync` 独占所有权胶囊移交给实例 Worker，由实例 Worker 完成设核、绑定 IO、推理与销毁）；
   - 若路线 A 出现跨线程 TLS 异常则验证后备路线 B（实例内加锁 dup）；输出实测数据与选路结论。

### 不在本期范围
- 宿主侧双层内存账本与分配求解（由 `10-03-npu-host-placement-ledger` 负责）；
- 算法包内部改造与 YOLO 通用流水线（由 `10-03-algo-sdk-rknn-shared-weights` 负责）；
- SQLite 持久化与 Web/API 路由（由 `10-03-npu-persistence-revision-barrier` 负责）；
- 8 小时长稳与全量板端压测（由 `10-03-npu-board-verification-release` 负责）。

---

## 2. 需求列表

### R01 — 基础 C ABI 布局不变与加法扩展（对应母任务 R06）
* 基础 `AvAlgoAbi` 64 位 96 字节布局不变；
* 新增 placement 扩展通过可选符号加载（如 `av_algo_get_placement_extension`）；
* 扩展接口通过固定大小的 POD 缓冲区与状态码通信，禁止跨 FFI 传递裸 Rust 类型（`Vec`/`String`）或未受保护的动态分配。

### R02 — 有界退出与析构隔离（对应母任务 R04）
* 正常停机、初始化失败、通道断开时，Worker 的退出等待必须有明确硬超时；
* 析构过程发生挂死或延迟时，所属 Worker 句柄转交隔离观察队列，不阻塞调用者；
* 清理状态与线程结束状态分别追踪，未完成清理不能假报 `completed`。

### R03 — Wire 数据校验与版本兼容（对应母任务 R06, R10）
* 插件解析 `config_json` 时单次剥离 `__heimdall_placement`，解析失败不静默回退默认值；
* 老插件面对未知 placement 键或缺少扩展时表现为 legacy/unsupported，不引发崩盘；
* 新插件在无注入环境下回退到本地单实例模式，但不伪造宿主账本。

### R04 — 硬件回退策略硬门交汇（对应母任务 R05, R14）
* placement 注入元数据后，`FallbackPolicy::RequireHardware` 仍处于最高优先级，禁止在亲和降级路径回退到 CPU 模拟会话。

### R05 — A0 目标板路线 A/B 实测验证（对应母任务 R14）
* 在真实开发板上实测首选路线 A，重点验证：
  1. child context 在非创建线程内完成推理、设核与销毁的能力；
  2. 兄弟实例并发多核推理能力；
  3. 单 child 独立销毁不影响兄弟实例；
  4. root context 活到最后 child 清理确认的生命周期。
* 若路线 A 通过，确认采纳路线 A；若路线 A 异常，切换路线 B；记录实测返回码并解除 `B-R14`。

---

## 3. 验收准则 (Acceptance Criteria)

- [ ] **AC01** (原 AC06): 基础 `AvAlgoAbi` 64 位 96 字节布局与已有签名完全不变；双侧布局测试断言通过；旧插件 fixture 加载正常。
- [ ] **AC02** (原 AC12): 推理正常但 backend Drop 阻塞、初始化失败清理阻塞、通道断开均在有界期限内隔离，无未捕获的无界 `join()`。
- [ ] **AC03** (原 AC23): A0 目标板实测产出结论报告：按首选路线 A 或后备路线 B 闭环验证 4 项判定指标，消除 `B-R14` 阻断。
- [ ] **AC04** (原 AC24): placement 注入与 auto 亲和降级均保持 `RequireHardware`，自检硬门不可翻越，无模拟会话回退。
- [ ] **AC05**: 宿主与插件新旧 4 种组合兼容测试全绿；超长/截断/错误代际清理回执被正确拦截。
