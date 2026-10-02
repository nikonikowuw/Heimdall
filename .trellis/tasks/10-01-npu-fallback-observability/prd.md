# PRD: NPU 硬件回退静默降级可观测性与自检硬门

> 状态：in_progress。创建日期 2026-10-01。来源：`09-30-edge-torch-algo-ecosystem` 交接项 H3。

## 1. 目标与用户价值

消除"算法包在无硬件时静默伪装成硬件推理"的失败模式，使**部署错误在安装自检阶段即被拒绝**，而不是在生产中稳定输出伪造结果。

交付价值：

- 运维能依赖「算法包安装自检通过」这一信号判断该包在本机硬件上真实可用；
- 硬件不可用时失败是**响亮且可定位**的，而不是静默降级；
- 开发机（macOS / x86）的无驱动回退能力**不被剥夺**。

## 2. 背景与现状 (Problem Statement)

`algo-sdk` 的 `RknnSession::open_or_fallback` 在 `RknnRuntime::load` 失败时静默返回 `CpuFallbackSession`，该会话产出 `fallback_outputs()` 的**固定模拟检测框**（2 个写死的目标）。

触发条件的宽度超出设计意图：不只是"物理上确无加速单元"，任何 `librknnrt` 加载失败（`package_root` 拼写错误、ABI 不匹配、`.so` 缺失）都会走到该分支。

三个加重因素：

1. `is_fallback()` 是 `pub` API，但 `crates/infer` 侧**零消费者**——宿主无法观测降级；
2. `RknnSessionOptions` 只有 `core_mask` 一个字段，**没有禁止降级的能力**；
3. 插件侧那句 `tracing::warn!("启用开发调试回退会话")` 在生产中**无出口**（见 §6 依赖），进一步掩盖降级。

后果：宿主侧已完成的 `platform_id` 校验、`algorithm_id` 校验、六步沙箱自检，**全部无法覆盖"模型是否真的加载到硬件"**。安装自检会对一个"只会输出假框的包"给出通过。

## 3. 范围

**In scope**：
- `algo-sdk` 侧回退策略的表达与执行（`algo-sdk` 的 `runtime` 模块）
- 安装自检模式下的硬件强制要求
- 单元测试与集成测试
- （可选）宿主自检报告的硬件状态记录，作为防御性增强而非验收门槛

**Out of scope**：
- 插件侧日志桥接（→ 独立任务 `10-01-plugin-log-bridge`）
- `RknnSessionOptions::core_mask` 与分核策略（→ `10-01-npu-core-allocation`）
- Ascend / CoreML 后端的等价能力（尚未实现，不在本任务伪造接口）
- 未经真机验证就断言 RKNN 错误码语义

## 4. 功能需求

### 4.1 回退策略化

`RknnSessionOptions` 增加回退策略维度，与既有 `core_mask` **并列且加法式**：

| 策略 | 语义 |
|---|---|
| `Allow` | 无硬件时降级为模拟会话（开发/调试） |
| `RequireHardware` | 无硬件时必须返回错误，**不得**返回模拟会话 |

`RequireHardware` 失败时必须携带可定位原因（含 `package_root`、模型路径、底层加载失败原因）。

### 4.2 策略解析规则

解析优先级（高 → 低）：

1. 安装自检模式 → 强制 `RequireHardware`；此硬门不可由代码显式覆盖或 `.env` 翻越。
2. 调用方显式策略 `fallback_policy_override` → 采纳显式值。仅供受控调用方（如本地开发工具）声明意图；生产 C ABI 路径恒传 `None`。
3. 算法包私有 `.env` 中 `ALLOW_CPU_FALLBACK=1` → 显式选择 `Allow`，但不影响安装自检。
4. 宿主 `platform_id` 归一化后属于硬件平台 → `RequireHardware`。
5. 其他平台 → `Allow`。

`run_local` 等本地开发工具应在代码中显式声明 `Allow`，不依赖 `.env` 逃生口。
**不得用 `.env` 作为默认策略来源。** `.env` 被版本库忽略，新设备上必然缺失；没有显式覆盖时，策略由自检模式与宿主自报的 `platform_id` 决定。

**平台判定必须基于宿主自报的 `platform_id`，不得依赖编译期 `cfg`。**
理由：`algo-sdk` 存在同一进程内同时服务多个平台算法包的场景，且 `cfg` 在 SDK 内无法表达"宿主是什么"。

### 4.3 安装自检硬门

安装自检只有在实例初始化满足真实硬件要求时才能通过。自检模式下无论调用方显式声明或包私有 `.env` 如何配置，都必须使用 `RequireHardware`；硬件不可用时 `instance_create` 返回模型加载错误，宿主既有自检流程据此判定失败。

### 4.4 声明式路径可传递策略

`GenericDetector` 等声明式骨架目前经 `RuntimeSession::open(package_root, model_path)` 构造会话，**无策略入参**。必须提供传递通道，且**不得破坏既有签名**（向后兼容）。

## 5. 非功能性约束

1. **调用方必须能区分三态**：真实硬件可用 / 模拟降级 / 需要硬件但不可用。三者行为不同，不得合并为布尔。
2. **向后兼容**：既有 `open_or_fallback` 调用点与 `RuntimeSession::open` 签名不得被破坏。
3. **不跨 FFI 传播 Panic**：新增错误路径必须经既有 `catch_unwind` 出口。
4. **加法式 ABI 演进**：不得修改 `AvAlgoAbi` / `AvFrameDesc` 布局。优先零 ABI 变更。
5. **失败信息可操作**：错误必须能让运维定位到具体原因，不得只报"初始化失败"。
6. **开发机能力保留**：macOS / x86 上的无驱动回退必须继续可用。
7. **错误语义遵循仓库既有约定**：不得吞错误后伪造成功（见 `.trellis/spec/guides/error-handling.md`）。

## 6. 依赖与关联

- **依赖 `10-01-plugin-log-bridge`（建议先做）**：若日志桥未接通，`RequireHardware` 的失败细节只能通过返回值传播，插件内的降级告警仍不可见。两者独立可交付，但合并后诊断能力完整。
- **关联 `10-01-npu-core-allocation`**：该任务同样要改 `RknnSessionOptions` 与策略透传口。本任务必须采用**加法式**设计（新增枚举 + 新构造入口），不修改 `core_mask` 语义，避免冲突。
- **回归风险**：`10-01-npu-core-allocation` 的调研指出 `open_or_fallback` 的 `core_mask` 在降级路径中会被丢弃。本任务若改变降级决策点，必须确认该行为不被进一步复杂化。

## 7. 验收标准 (Acceptance Criteria)

- [x] **A1：策略可表达** — `RknnSessionOptions` 可表达 `Allow` / `RequireHardware`；默认值等于既有行为（`Allow`），既有测试无需改动即通过。
- [x] **A2：`RequireHardware` 真实生效** — 无可用运行时时，构造返回 `Err`，**不得**返回可用的模拟会话。测试须断言"返回的是错误"而不只是"返回了某个东西"。
- [x] **A3：三态可判别** — 提供查询能力使调用方能区分真实硬件 / 模拟降级 / 需硬件但不可用。测试覆盖三种状态。
- [x] **A4：自检硬门** — 安装自检模式下，模拟降级不得导致自检通过。测试须覆盖"自检 + 无硬件 → 失败"。
- [x] **A5：开发机回退保留** — 非硬件平台上默认策略仍为 `Allow`，既有回退路径测试继续通过。
- [x] **A6：平台判定基于 platform_id** — 规则以归一化后的宿主 `platform_id` 为准，`algo-sdk` 内**不得**新增按目标 SoC 或宿主 OS 的 `cfg` 分支。可通过代码审查 + 测试断言。
- [x] **A7：自检流程在模拟降级下失败（可由 SDK 单独满足）** — 安装自检模式下，模拟降级必须导致自检失败，且给出可定位错误。
  - **实现路径已定稿（`design.md` §1.4 / §4.4）**：硬门完全落在 `algo-sdk` 内，由 `instance_create` 返回 `AV_ERR_MODEL_LOAD_FAILED` 自然使 `sandbox.rs:838` 判定失败，**宿主零改动**。
  - 因此本项的验收证据是「自检模式 + 无硬件 → `instance_create` 返回 `-5` 且 `last_error` 可定位」，**不**要求宿主新增校验代码。
  - 可选防御性增强（非验收门槛）：在 `SelfTestReport` 中记录硬件状态，使自检报告能区分真实推理与模拟会话（`design.md` §4.4）。
- [x] **A8：向后兼容** — `RuntimeSession::open`、`RknnSession::open_or_fallback` 现有签名与行为对外保持可用；`algo-sdk` 与算法包的既有测试全绿。
- [x] **A9：门禁全绿** — `cargo fmt --all -- --check`、`cargo clippy --all-targets -- -D warnings`、`cargo nextest run --workspace`（或 `cargo test`），以及 AGENTS.md 列出的四个算法平台 workspace 门禁全部通过。

## 8. 开放问题（均已定稿）

1. ~~`RequireHardware` 失败应映射到哪个既有状态码？~~
   **已定稿 → `design.md` §2.4**：`AV_ERR_MODEL_LOAD_FAILED = -5`（经 `AlgoError::ModelLoad`）。零新增状态码、零 ABI 变更。
2. ~~宿主自检的“显式校验”采用哪种形式？~~
   **已定稿 → `design.md` §1.4 / §4.4**：**无需宿主新增校验**。
   硬门完全落在 `algo-sdk` 内——`instance_create` 返回 `-5` 已自然使 `sandbox.rs:838` 判定自检失败。
   可选增强（非验收门槛）：`SelfTestReport` 记录硬件状态。
3. ~~是否提供 `.env` 覆盖开关？~~
   **已定稿 → `design.md` §4.1**：仅作为**显式覆盖**（`ALLOW_CPU_FALLBACK=1` 时强制 `Allow`），默认不从 `.env` 读取策略。
   理由：`.env` 被版本库忽略，新设备上必然缺失，不能充当安全开关。
