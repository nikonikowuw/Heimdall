# NPU 多核分配板端性能基线与发布验收 (PRD)

## 1. 目标与背景 (Goal & Context)

本任务是父任务 `10-01-npu-core-allocation`（宿主协同 NPU 多核分配与卡亲和架构）的最终收口与发布验收子任务。在 Subtask 1~4 已分别完成 C ABI 协议基线、宿主双层账本、算法包共享权重运行时与配置持久化/版本栅栏的基础上，本任务旨在真实物理边缘设备（RK3568、RK3576）上执行全链路性能实测、路线 A 线程归属判定、内存增量取证、长稳压测，并冻结性能基线，完成规范更新与发布准入。

## 2. 目标硬件与环境基线 (Hardware Targets)

| 目标平台 | 网络地址 | Linux 内核 | NPU 驱动 | NPU 核心与算力 | librknnrt 版本 |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **RK3568** | `root@192.168.17.140` | Linux 5.10.226 | RKNPU v0.9.8 | 1 Core (1.0 TOPS) | 2.3.2 (429f97ae6b) |
| **RK3576** | `root@192.168.18.28` | Linux 6.1.118 | RKNPU v0.9.8 | 2 Cores (6.0 TOPS) | 2.3.2 (429f97ae6b) |

两台物理设备经探查均已导出 `rknn_dup_context` 符号，具备物理权重共享与逐实例独立子会话的基础环境。

## 3. 核心需求与判定契约 (Requirements)

### R01: 路线 A 线程归属实测与选路裁决 (Blocking Fact: B-R14)
- 优先验证首选**路线 A**（控制面线程加载根权重并 `rknn_dup_context`，将子会话独占移交给实例 Worker 线程；Worker 线程内调用 `rknn_set_core_mask`、绑定独立 IO、执行 `rknn_run` 与清理）。
- 实测验证四项判定标准：
  1. 跨线程设核：实例 Worker 线程对移交的 child context 设置 core mask 是否返回 `RKNN_SUCC (0)`。
  2. 跨线程推理：Worker 线程内部执行 `rknn_run` 是否成功且输出正确。
  3. 并发安全：多实例 Worker 并发执行时是否存在 TLS 污染、段错误或输出覆盖。
  4. 析构安全：Worker 退出销毁 child context、控制侧销毁 root context 是否无 double-free 或阻塞。
- 若路线 A 四项全过，正式裁决采纳路线 A，消解阻塞项 B-R14；若出现不可克服的驱动层 TLS 限制，则激活路线 B。

### R02: 物理共享权重与内存增量实测包 (AC19, AC20, B-CAP)
- 在真实板端对比两组配置：
  - 对照组：同模型 3 个实例分别独立调用 `rknn_init` 加载模型权重（旧/独立模式）。
  - 实验组：同模型 1 个根权重 + 3 个 child context（新共享权重架构）。
- 记录并提取 1/2/3/N 实例时的系统已用内存、PSS/RSS、CMA 使用量（`/proc/meminfo` CmaAllocated/CmaFree）以及私有 workspace 消耗。
- 证明“一份权重只占一份显存/物理内存，实例增加仅增加少量私有 IO/workspace 内存”。

### R03: 宿主分核与多核负载实测 (AC01, AC14, B-THM)
- 在 RK3576（双核）上实测双实例分别绑定 Core0 (0x01) 和 Core1 (0x02)，并发发起推理，验证 `/sys/kernel/debug/rknpu/load` 中 `Core0` 与 `Core1` 均出现对应负载，验证硬件级真正并行。
- 在 RK3568（单核）上实测多实例在 Core0 (0x01) 上的排队复用，验证单核平台降级正常、无死锁。

### R04: 循环启停长稳与资源泄漏验证 (AC04, AC12, AC21)
- 在目标板执行至少 100 轮实例启动、推理、销毁生命周期测试。
- 验证每次测试后无 fd 泄漏（`/proc/$PID/fd` 数量稳定）、CMA 与驻留内存恢复至初始基线，无孤儿句柄。
- 验证先子后根的析构生命周期顺序，杜绝先释放根导致子会话悬挂。

### R05: 全链路门禁、规范更新与发布准入 (AC11, B-TGT, B-CAP, B-THM)
- 根 Workspace 及跨平台算法包（`algo-packages/macos`、`rk3568`、`rk3576`、`rk3588`）通过 `cargo fmt`、`cargo clippy`、`cargo nextest`。
- Web 端运行 `pnpm typecheck`、`pnpm lint`、`pnpm test` 保持全绿。
- 同步更新 `.trellis/spec/infer/backend/inference-backends.md`、`algo-sdk-guidelines.md`，固化 RK3568/RK3576 的性能基线与容量参数。
- 清除父任务 4 个 Blocking Facts，关闭父任务 `10-01-npu-core-allocation`。

## 4. 验收准则 (Acceptance Criteria)

- [x] **AC01 (路线 A 板端成立)**：在 RK3568 和 RK3576 真实板端上，路线 A（控制面 dup 后移交实例线程设核与推理）四项判定标准全部通过，无 TLS 冲突与驱动 panic。
- [x] **AC02 (物理共享增量证明)**：提供板端实测数据报告，证明同模型 3 实例相较于 1 实例，权重内存（CMA/物理显存）不重复分配，增量仅为私有 IO/workspace（< 2MB/实例 vs 模型文件十几 MB）。
- [x] **AC03 (多核真实分流)**：RK3576 板端读取 `/sys/kernel/debug/rknpu/load` 明确显示 Core0 与 Core1 同时有利用率且推理结果完全正确。
- [x] **AC04 (长稳启停零泄漏)**：100 轮启停测试后，fd 数量和 CMA 消耗完全复位回初始基线，无内存泄露。
- [x] **AC05 (全工作区门禁绿灯)**：Rust 根 workspace、各平台 algo-package 及 Web 前端所有格式化、类型检查、lint 与单元/集成测试全部通过。
- [x] **AC06 (规范与发布闭环)**：规范文档更新完毕，消除全部 blocking facts，父任务可整体标记归档。

## 5. 约束与非目标 (Constraints & Non-Goals)

- 不在生产板上执行破坏驱动或破坏系统只读分区的危险操作。
- 不引入跨机分布式 NPU 调度或超范围的算子改动。
- 前端本次仅校验现有 DTO / 类型 / 数据流兼容性，不新增额外 UI 组件。
