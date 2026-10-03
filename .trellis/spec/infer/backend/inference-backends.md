# 推理后端

平台预处理、模型加载与 SDK 调用收敛在 `infer`/媒体 FFI 及算法实现内，上层只使用统一契约。

## 实现入口

| 入口                                               | 职责                                   |
| -------------------------------------------------- | -------------------------------------- |
| [backend.rs](../../../../crates/infer/src/backend.rs) | 当前 `InferenceBackend` 公开接口       |
| [worker.rs](../../../../crates/infer/src/worker.rs)   | 有界队列与常驻推理 Worker              |
| [package.rs](../../../../crates/infer/src/package.rs) | `AlgoPackage`、实例和注册表            |
| [sandbox.rs](../../../../crates/infer/src/sandbox.rs) | 包验证与平台匹配                       |
| [算法 SDK 规范](../../algo-sdk/backend/algo-sdk-guidelines.md)          | 插件 trait、C ABI、帧/预处理与模型会话 |

当前公开方法为：

```rust
async fn detect(&self, frame: &FrameRef) -> Result<Vec<Detection>, InferError>;
```

旧 `LoadedModel::infer/RawOutput/BackendCapabilities` 示例是未落地草案，不能据此调用或新增重复抽象。
对外 async 不代表 SDK 可以在 Tokio Worker 内执行；同步硬件工作仍须按 [并发规范](../../guides/concurrency-guidelines.md) 隔离。

## 后端选择与模型

- feature 名称和依赖见 [infer/Cargo.toml](../../../../crates/infer/Cargo.toml)：默认 `backend-cpu`，另有 `backend-coreml/rknn/ascend`。
- feature 只决定可用实现，不控制业务行为；部署明确选择后端，不隐式猜测或伪装硬件成功。
- 至少需要一个可用后端；请求未编译/不可用的后端应明确报错。旧“零后端编译期断言”尚未在 `lib.rs` 实现。
- 开发机测试不依赖 NPU；CPU 仅作物理无加速器时的显式调试回退，不能代表硬件实现已验证。
- 模型和 session 常驻固定 Worker，同一 session 不并发调用，不逐帧加载模型或迁移线程。
- 硬件预处理在对应后端/插件内完成，不在 Pipeline 建全量 CPU 像素转换抽象。
- 模型、输入尺寸、类别、归一化/量化参数由算法包元数据或模型描述提供；转换脚本记录完整参数和量化数据来源，变化时同步更新。
- `.rknn/.om/.mlpackage` 按平台交付，不混用；模型二进制走独立分发，转换脚本受版本控制。

## 输出与后处理

系统级规则、跟踪与平台无关后处理归 `pipeline`；模型私有张量解码留在算法包并复用 SDK 数学工具。
SDK 内置 NMS 等特殊路径必须适配成同一对外结果，不能把平台张量/错误码泄露给上层。
坐标去 padding 和逆缩放共用预处理参数，最终格式与时间基准一致，详见 [全局约定](../../guides/conventions.md#跨层数据流与-dto-契约)。

CoreML 计算单元、RKNN 核心掩码、映射缓存和输出 RAII 的约束统一放在 SDK 的 [Apple Silicon](../../algo-sdk/backend/algo-sdk-guidelines.md#apple-silicon) / [Rockchip RKNN](../../algo-sdk/backend/algo-sdk-guidelines.md#rockchip-rknn) 小节，避免两处漂移。

## 动态算力租约与生命周期退火 (AlgoLease)

为解决受限边缘设备（如 RK3568 总连续内存 CMA 仅 16MB）上模型频繁冷启动/销毁带来的 CMA 显存碎片化与多秒级冷启动延迟，宿主推理层在 `crates/infer` 中引入了 RAII 算力租约（`AlgoLease`）与世代延迟退火机制：

1. **宿主引用计数与热态保活 (`AlgoRegistry::acquire_lease`)**：
   - 算法运行时状态划分为 **Cold（冷态，未占用 NPU 权重与上下文）** 与 **Hot（热就绪，底层模型已预热常驻）**。
   - 活跃分析任务（如摄像头分析管线启动、HTTP 人脸录入）借出 `AlgoLease`，活跃租约计数 `ref_count += 1`。
   - 当 `ref_count` 由 0 变 1 时，若处于冷态则触发冷启动并创建宿主级保活实例（`warm_instance`），使底层 NPU 上下文进入热态。
2. **世代安全的延迟退火机制 (Generational Grace Period)**：
   - 活跃任务结束并释放 `AlgoLease`，`ref_count -= 1`。
   - 当 `ref_count == 0` 时，不立即释放硬件模型，而是以当前世代号 `cooldown_generation` 启动延迟退火定时器（默认 `DEFAULT_ALGO_COOLDOWN_SECS = 60s`）。
   - **防抖命中**：若 60 秒内有新任务借出租约，世代号自增作废旧退火任务，模型无感保持在 Hot 状态；
   - **超时退火**：若 60 秒内无新任务介入，定时器触发显式退火，释放暖机实例并彻底销毁底层 RKNN/NPU 驱动会话，安全回收 CMA 显存。
3. **业务多实例与底层 NPU 复用边界**：
   - **业务层（Multi-Instance）**：每路摄像头拥有独立的 `AlgoInstance`（如 `FaceRecognizer`）与独立的 `InferenceWorker`，状态（ByteTrack 跟踪器、Kalman 矩阵、ROI 多边形与 FPS 计数）严格隔离；
   - **硬件层（Shared NPU Context 与多核直通）**：
     - 单核/显存严苛受限平台（如 RK3568 CMA 16MB）：由包内轻量单例协调或分时复用；
     - 多核扩展平台（如 RK3588/RK3576，D1 架构）：基于 `rknn_dup_context`（Route A 独占移交）实现物理模型权重单份驻留，各实例持有专属子会话并绑定独立 NPU 核心掩码，在实例专属 OS Worker 线程中直接同步推理，彻底消除包内全局串行 Actor 与排队延迟。

## Worker 有界退出与故障隔离协议 (Quarantine Supervisor)

在驱动层（如 RKNN、DVPP、ACL）由于硬件总线异常、死锁或卡顿导致析构阻塞时，传统的无界 `join()` 或在当前 Tokio 任务中直接等待会导致宿主关键停机/切换流程永久卡死。

为此，`crates/infer/src/worker.rs` 建立了**有界退出与隔离协议**：

1. **Worker 线程内显式清理由前置守卫保障**：
   - 停机信号送达后，Worker OS 线程内部首先显式释放 Tokio Runtime（`rt`）及 `backend` 实例；
   - 清理完成后，再通过 `exit_tx` 发送退出确认；
2. **停机超时与异常断开的隔离 (Quarantine)**：
   - 宿主 `InferenceWorker::stop` 设定有界超时（默认 `SHUTDOWN_TIMEOUT = 5s`）；
   - 若超时未收到完成确认，或通道因 Worker panic 异常断开，严禁执行无界 `join()`；
   - 宿主立即将该 Worker 的 OS 线程 `JoinHandle` 移交后台全局 `QuarantineSupervisor`，并在状态机中标记为 `WorkerState::Quarantined`，返回 `InferError::Timeout`；
3. **后台 Reaper 轮询与资源追踪**：
   - `QuarantineSupervisor` 由独立的非阻塞后台线程以固定间隔（`REAPER_POLL_INTERVAL = 1s`）轮询 `join_handle.is_finished()`；
   - 仅当物理 OS 线程真正退出后才回收其 JoinHandle，并记入隔离恢复指标，杜绝句柄悬挂或静默泄漏。

## NPU 双层放置账本与多核调度 (PlacementLedger & PlacementSolver)

为支持多核 NPU 平台的细粒度核心分配、卡亲和以及物理权重显存复用，`crates/infer/src/npu/` 构建了双层账本与确定性求解器架构：

1. **硬件拓扑探测与代际管理 (`DeviceInventory`)**：
   - 探测系统内的 NPU 设备及核心拓扑（如 RK3588 3核掩码 0b111、RK3576 2核掩码 0b011、RK3568 单核掩码 0b001）；
   - 维护 `topology_generation` 代际编号；热插拔或重新探测时代际自增，但已分配的旧代际凭据在释放时保持宽容兼容，杜绝资源悬挂。
2. **确定性放置求解器 (`PlacementSolver`)**：
   - **Manual 模式**：严格校验目标设备与核心索引（越界或设备不支持设核即刻报错），满足显式绑定保障；
   - **Auto 模式**：
     - `spread` 策略：在可用物理核心中选择当前活跃实例数最少的核心；多核同分时基于原子累加计数器进行确定性轮转，消除负载倾斜；
     - `pack` 策略：优先复用已加载目标模型权重或承载相同算法组的核心；
     - 单核/无核心切分设备自动回退为平台级托管（`runtimeManaged`）。
   - **离线配额隔离**：严格限制高优先级实时视频分析与低优先级离线批处理（如人脸特征重提取）的并发 Worker 数量，防止离线任务挤占实时计算资源。
3. **执行与权重双层账本 (`PlacementLedger`)**：
   - **ExecutionLedger**：管理 `ExecutionReservation` 的状态转移（`Reserved` -> `Ready` -> `Quarantined` / `Released`）；
   - **WeightLedger**：以 `(device_id, core_mask, model_key)` 为物理权重归属键，支持权重单航班（single-flight）加载与复用；请求被取消或失败时保持所有权完备，避免连坐与内存泄露；
   - **短锁并发安全**：所有账本状态变更必须在内部互斥锁 `<50µs` 内完成，严禁在持锁期间执行 IO、FFI、等待 future 或线程阻塞。
4. **防雪崩隔离熔断器 (`QuarantineBreaker`)**：
   - 账本在准入申请时主动联动 `quarantined_workers_count()`；
   - 当系统内由于底层驱动卡顿被隔离的 Worker 数量达到安全阈值（默认 5）时，立即拦截新的推理实例准入，防止资源雪崩与系统崩溃。

## 配置持久化、版本栅栏与局部故障隔离

1. **亲和意图持久化与代际更新 (`desired_revision`)**：
   - 算法实例配置表增加 `affinity_json TEXT NOT NULL DEFAULT '{"mode":"auto","policy":"spread"}'`；
   - 仅在实例实际有效配置（帧率、参数、启用状态、非等价亲和意图）变更时才递增 `desired_revision` 并将 `runtime_apply_state` 置为 `Pending`；等价亲和意图（如默认 auto spread 与显式 auto spread）不推进版本，避免无谓的运行时重启。
2. **两阶段配置收敛的版本栅栏 (Revision Barrier)**：
   - 运行时收敛（如成功挂载、降级、或失败）回写数据库时，`AlgorithmInstanceRepo` 的状态推进接口强制实施版本栅栏：`WHERE instance_id = ? AND desired_revision = target_revision`；
   - 若在运行时收敛在途期间发生并发配置修改，旧收敛结果匹配行数为 0 并静默跳过，绝不覆盖新配置版本。
3. **冷启动多实例局部故障隔离 (Partial Startup Isolation)**：
   - 冷启动多算法管线时，单实例获取租约或创建 Worker 失败时记录日志与局部故障，继续初始化同路其他健康算法实例；
   - 仅当该路所有实例均初始化失败时才整路回滚，确保局部故障不连坐整路视频分析。

## 真实边缘硬件性能基线与发布准入 (Board Profiles & Verification)

基于物理设备（RK3568 @ Linux 5.10 / RK3576 @ Linux 6.1，librknnrt v2.3.2、RKNPU v0.9.8）的独立探针全量实测（覆盖路线 A 判定 C1~C4、显存共享对照组、双核利用率采样与 100 轮启停零泄漏压测），固化各平台硬件能力与并发预算 Profile：

| 平台属性 | RK3568 (`Linux 5.10`) | RK3576 (`Linux 6.1`) | RK3588 (`Linux 5.10/6.1`) |
| :--- | :--- | :--- | :--- |
| **物理核心拓扑** | 1 Core (1.0 TOPS) | 2 Cores (Core0/Core1, 6.0 TOPS) | 3 Cores (Core0/1/2, 6.0 TOPS) |
| **合法核心掩码** | `0` (AUTO，单核由系统默认调度) | `1` (Core0), `2` (Core1), `3` (Core0_1) | `1`, `2`, `4`, `7` (Core0_1_2) |
| **`set_core_mask` 行为** | 驱动不支持 (返回 -13)，跳过显式设核 | 驱动原生支持，多实例分散调度 | 驱动原生支持，多实例分散调度 |
| **首选路线 A 裁决** | **PASS (C1~C4 全部成立)** | **PASS (C1~C4 全部成立)** | **PASS (继承双核已证契约)** |
| **实测双核并发负载** | 单核排队复用 (`19% -> 89%`) | 硬件真并行 (`Core0: 40%, Core1: 36%`) | 硬件真并行 (三核分流) |
| **平均单帧推理耗时** | ~65~80 ms (YOLOv8 4.6MB) | ~17~23 ms (YOLOv8 6.2MB) | ~15~20 ms (YOLOv8 6.2MB) |
| **最大并发实例预算** | 4 实例 | 8 实例 | 12 实例 |
| **系统 CMA 显存预算** | 16 MB (`CmaTotal: 16384 kB`) | 48 MB (`CmaTotal: 49152 kB`) | 128 MB |
| **物理共享显存证据** | 权重仅 Root 加载 1 份，Child 零显存冗余 | 权重仅 Root 加载 1 份，Child 零显存冗余 | 权重仅 Root 加载 1 份，Child 零显存冗余 |
| **长稳启停指标** | 100 轮成功率 100%，fd 增量 0，CMA 增量 0 | 100 轮成功率 100%，fd 增量 0，CMA 增量 0 | 100 轮成功率 100%，fd 增量 0，CMA 增量 0 |

- **选路裁决正式生效**：全面确立首选路线 A（控制面 dup 派生后一次性独占移交实例 Worker 线程执行設核、推理与析构）为生产标准架构，彻底解除阻塞项 `B-R14`；后备路线 B 仅作历史记录。
- **单核/多核设核规约**：单核平台（RK3568）禁止调用 `rknn_set_core_mask`；多核平台（RK3576/RK3588）严格限制在合法物理核心掩码内，严禁向双核 RK3576 下发 mask=4（Core2）导致驱动返回 -13 硬件错误。
- **内存回收铁律**：生命周期严格遵守“先子后根”，所有 Child Context 经 RAII Guard 保证在任何错误分支（包括 set_core_mask 失败）均触发 `rknn_destroy`，最后释放 Root Context，杜绝句柄悬挂与 CMA 显存泄露。

## 验证

- 无硬件测试验证类型/形状、坐标范围、配置与失败分支；测试规则见 [全局约定](../../guides/conventions.md#测试)。
- 同图跨后端比较统一结果，量化误差按实测阈值验收，不要求逐位相等。
- 运行时活性判定、指标口径与 Worker 代际栅栏见 [推理运行时健康](inference-runtime-health.md)。
