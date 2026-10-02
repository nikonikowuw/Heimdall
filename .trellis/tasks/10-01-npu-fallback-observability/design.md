# Technical Design: NPU 硬件回退静默降级可观测性与自检硬门

> 依据 `prd.md` §4-§5。所有引用的源码位置均在 2026-10-01 于 `dev` 分支核实。

## 1. 缺陷机制（先定位，再设计）

### 1.1 静默降级路径

```rust
// crates/algo-sdk/src/runtime/platforms/rockchip.rs:483-495
pub fn open_or_fallback(package_root, model_path, options) -> Result<Self, AlgoError> {
    match RknnRuntime::load(package_root) {
        Ok(rt) => Self::new_with_core_mask(rt, model_path, options.core_mask),
        Err(e) => {
            tracing::warn!(reason = ?e, "未检测到物理 librknnrt.so，启用开发调试回退会话");
            Self::new_fallback(model_path)   // 返回 Ok，且 backend = Fallback
        }
    }
}
```

`RknnBackend::Fallback` 在 `infer_with` 中直接返回 `fallback_outputs()`（`rockchip.rs:737,779`），即**写死的 2 个目标**（锚点 10 类别 0 置信度 0.92；锚点 25 类别 1 置信度 0.88）。

### 1.2 为什么宿主完全看不见

| 层 | 事实 | 位置 |
|---|---|---|
| SDK | `is_fallback()` 是 `pub` API | `runtime/mod.rs:90` |
| 宿主 | `crates/infer` 对 `is_fallback` **零引用** | `grep -rn is_fallback crates/infer/` → 0 |
| SDK | 20 处 `tracing::warn/error` 无出口（不消费宿主 `AvLogFn`） | `macros.rs:386`（`LibraryContext` 丢弃 `raw_args.log`） |

### 1.3 虚假认证的完整证据链

自检第 6 步只判定 `instance_process` 的返回码：

```rust
// crates/infer/src/sandbox.rs:901-910
let process_code = unsafe { process_fn(raw_inst, &frame) };
if process_code != AV_OK { return Err(...); }   // fallback 返回 AV_OK
on_progress(SandboxProgressEvent::passed(6));
let detections_count = collected_results.lock().map(|r| r.len()).unwrap_or(0);  // == 2（假框）
```

**结论**：在物理上无 `librknnrt` 的目标设备上，六步沙箱自检**通过**，且 `SelfTestReport.detections_count = 2`（来自伪造框，非真实推理）。宿主已有的 `platform_id` 校验（`package.rs:106`）与 `algorithm_id` 校验（`package.rs:123`）**都无法覆盖"模型是否真的加载到硬件"**。

### 1.4 可利用的关键事实

1. `AlgoPlugin::init` 的 `InitContext` 携带 `is_self_test`、`platform_id` 与 `fallback_policy_override`（`plugin.rs`）；
2. `is_self_test` 由 `raw_args.mode == AV_INSTANCE_INSTALL_SELF_TEST` 派生；
3. `export_algo!` 创建的生产上下文恒将 `fallback_policy_override` 置为 `None`；自检下策略解析始终优先返回 `RequireHardware`；
4. `instance_create` 返回非 `AV_OK` 时，宿主既有自检流程已会失败（`sandbox.rs`）并通过 `last_error` 取回原因。

→ **硬门完全落在 `algo-sdk`，宿主零改动**（§4.4）。

## 2. 契约与边界

### 2.1 策略、状态与运行时选项（`A1` / `A3`）

```rust
// crates/algo-sdk/src/runtime/fallback.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FallbackPolicy {
    #[default]
    Allow,
    RequireHardware,
}

pub enum HardwareStatus { Hardware, Simulated } // 成功构造的会话状态
pub enum HardwareAvailability { Hardware, Simulated, Unavailable } // 构造结果三态

// crates/algo-sdk/src/runtime/platforms/rockchip.rs
#[non_exhaustive]
pub struct RknnSessionOptions {
    pub core_mask: c_int,
    pub fallback_policy: FallbackPolicy, // 新增字段，Default = Allow
}
```

`FallbackPolicy::Allow` 与 `RknnSessionOptions::default()` 保持既有默认行为；`new` / `with_core_mask` 保留原签名。
`hardware_status()` 提供成功会话的硬件状态，既有 `is_fallback()` 保留并委托给该状态。

### 2.2 三态可判别（`A3`）

会话对象只包含成功构造的两态：`HardwareStatus::{Hardware, Simulated}`。需要硬件但不可用时构造返回 `Err(AlgoError::ModelLoad { .. })`，不会产生会话对象。统一归约入口 `classify_session_availability(&Result<HardwareStatus, AlgoError>)` 将这两态与失败映射为 `HardwareAvailability::{Hardware, Simulated, Unavailable}`。

`RknnSession` 与 `RuntimeSession` 都提供 `hardware_status()`；`is_fallback()` 保留既有签名与语义。

### 2.3 策略解析（`A4` / `A5` / `A6`）

平台判定只依据宿主自报的 `platform_id`，不使用 SoC / OS `cfg`；归一化规则在 SDK 侧与宿主保持最小必要重复，并由双方表驱动测试锁定。
RKNN / Ascend 平台归一化后要求硬件；macOS / x86 等平台默认允许模拟回退。

实际解析优先级（高 → 低）：

1. `is_self_test == true` → `RequireHardware`，硬门不可绕过；
2. `InitContext::fallback_policy_override` 为 `Some(policy)` → 采纳显式策略；生产 C ABI 路径恒传 `None`，本地 `run_local` 工具可显式传 `Allow`；
3. 包私有 `.env` 的 `ALLOW_CPU_FALLBACK=1` → 显式 `Allow`，但不能覆盖自检硬门；
4. `platform_requires_hardware(platform_id)` 为真 → `RequireHardware`；
5. 其他 → `Allow`。

统一入口为 `resolve_fallback_policy(is_self_test, platform_id, env, explicit)`；`GenericDetector` 与直接创建 RKNN 会话的插件都使用此入口，避免调用方各自实现优先级。`.env` 不是默认策略来源，仅在用户显式配置时覆盖普通实例的硬件平台策略。

### 2.4 `RequireHardware` 的失败语义（`A2`；开放问题 1 定稿）

**决定**：返回 `AlgoError::ModelLoad { reason }`。

理由：
- `AlgoError::ModelLoad` 已映射到 `AV_ERR_MODEL_LOAD_FAILED = -5`（`error.rs:48`），语义贴合"模型未能加载到目标运行时"；
- **零新增状态码**，`AV_ALGO_API_VERSION` 不变，零 ABI 变更（满足 §5 约束 4）；
- 沿既有链路自然变响：`init` 返回 `Err` → `macros.rs:787-790` `set_last_error` + `to_c_status()` → `instance_create` 返回 `-5` → 宿主 `check_c_status` 取回 `last_error` 详情 → `InferError::CAbiError`。

`reason` 必须包含可定位信息：`package_root`、模型路径、底层加载失败原因和可操作指引。宿主 `check_c_status` 读取 `last_error` 的栈缓冲为 **512 字节**，因此消息顺序固定为：失败结论 → 操作指引 → `package_root` → 模型路径 → 底层原因（最易截断，放最后）。错误采用 `AlgoError::ModelLoad`（映射到 `AV_ERR_MODEL_LOAD_FAILED = -5`）；同时明确即使设置 `ALLOW_CPU_FALLBACK=1` 也不能绕过安装自检硬门。

### 2.5 声明式路径的策略透传（`A4` / `A8`）

`RuntimeSession::open(package_root, model_path)` 保留既有签名并默认 `Allow`，以满足向后兼容；增加 `open_with_policy` 传递显式策略。
`GenericDetector::init` 先加载算法包私有 `.env` 供配置解析，再通过统一 resolver 计算策略：

```rust
let env = ctx.load_env();
let policy = RuntimeSession::resolve_policy(
    ctx.is_self_test,
    ctx.platform_id,
    Some(&env),
    ctx.fallback_policy_override,
);
let session = RuntimeSession::open_with_policy(ctx.package_root, &model_file, policy)?;
```

即使 `.env` 或显式 override 请求 `Allow`，自检模式仍先解析为 `RequireHardware`。

## 3. 数据流

```
宿主六步沙箱自检
   │  instance_create(mode = AV_INSTANCE_INSTALL_SELF_TEST)
   ▼
macros.rs 以 is_self_test=true、fallback_policy_override=None 构造 InitContext
   │
   ▼
GenericDetector::init / 直接 RKNN 插件 init
   │  resolve_policy(true, platform_id, env, explicit) → RequireHardware
   ▼
RuntimeSession::open_with_policy(.., RequireHardware)
或 RknnSession::open_or_fallback(.., RequireHardware)
   │
   ▼
RknnRuntime::load 失败
   ├─ Allow            → new_fallback() → Ok(Simulated)  ← 仅显式开发/调试路径
   └─ RequireHardware  → Err(ModelLoad)                  ← 自检在此中断
                              │
                              ▼
                    macros.rs 设置 last_error + 返回 AV_ERR_MODEL_LOAD_FAILED
                              │
                              ▼
                    sandbox.rs 既有 create 失败分支判定自检失败

普通 C ABI 实例的 override 恒为 None；硬件平台运行时不可用时同样 fail fast。
本地 `run_local` 工具通过 `with_fallback_policy_override(Allow)` 明确表达模拟调试意图，且该声明不经 C ABI 传递。
```

## 4. 边界与取舍

### 4.1 显式覆盖与开发机默认

`.env` 不是默认策略来源；缺失时策略仍由自检模式和宿主 `platform_id` 决定。仅当包私有 `.env` 明确写入 `ALLOW_CPU_FALLBACK=1` 时，才为普通实例显式选择 `Allow`；安装自检始终先于该项并强制 `RequireHardware`。

本地 `run_local` 等工具采用代码级 `with_fallback_policy_override(FallbackPolicy::Allow)`，不依赖 `.env`。生产 `export_algo!` 路径恒传 `None`，因此代码级开发覆盖不会影响 C ABI 装载。

### 4.2 为什么平台判定不用 `cfg`

`algo-sdk` 被编译进各平台算法包，编译期 `cfg` 只能表达"本包编译成什么"，不能表达"宿主运行在什么平台"。且 SDK 内新增 `cfg` 分支会与 AGENTS.md「平台差异只允许收敛在 media/infer 及其 FFI 实现内」的收敛方向相悖。

### 4.3 为什么 `RequireHardware` 适用于普通实例（非仅自检）

仅对自检加门会留下死角：**生产常规实例仍会静默降级**。既然宿主已强制算法包与平台匹配（`package.rs:106`），那么"在 `linux-rknn` 平台上声明 `linux-rknn` 的包"就应当真的用上硬件；用不上就是部署错误，应当响亮失败。

### 4.4 宿主是否需要改动

**硬门不需要宿主改动**：`instance_create` 在 `init` 返回 `Err` 时已有错误码与 `last_error` 传播链路，既有沙箱 create 失败分支据此拒绝安装。真实 C ABI 路径由 `export_algo!` 创建 `fallback_policy_override=None` 的上下文；自检标记优先级最高，因此 `.env` 和调用方显式 `Allow` 都不能让自检通过。

本任务未扩展 `SelfTestReport`；在构造阶段硬件不可用就失败，因此无需在通过的报告中补充模拟态字段。

### 4.5 需要一并更新的既有调用点

以下两个包直接调用 `open_or_fallback`，不走 `RuntimeSession`，因此也必须调用统一策略解析器并把结果写入 `RknnSessionOptions`：

| 包 | 行为 |
|---|---|
| `algo-packages/rknn/rk3568/fire-detections` | 普通硬件平台实例 `RequireHardware`；本地 `run_local` 显式 `Allow` |
| `algo-packages/rknn/rk3576/general_detection` | 普通硬件平台实例 `RequireHardware`；本地 `run_local` 显式 `Allow` |

RKNN 人脸工具直接加载 `RknnRuntime`，不经过模拟 fallback；更新 `InitContext` 字面量时保持 `fallback_policy_override=None`。

## 5. 测试策略

| 验收项 | 测试 | 断言要点 |
|---|---|---|
| A1 | 单元 | `FallbackPolicy::default() == Allow`；`RknnSessionOptions` 既有构造点行为不变 |
| A2 | 单元 | 无运行时 + `RequireHardware` → `Err(ModelLoad)`；断言错误**变体**而非仅 `is_err()` |
| A3 | 单元 | `HardwareStatus` 两态、`HardwareAvailability` 三态归约；`is_fallback()` 与 `Simulated` 等价 |
| A4 | 单元 / C ABI 集成 | 自检模式、显式 `Allow`、`.env` `Allow` 均不能绕过 `RequireHardware` |
| A5 | 单元 | 非硬件平台默认 `Allow`；无驱动回退返回 `Ok(Simulated)` |
| A6 | 单元 | SDK 与宿主平台别名归一化一致；不出现 SoC / OS `cfg` 平台策略分支 |
| A7 | C ABI 集成 | 自检模式 + 无硬件 → `instance_create` 返回 `AV_ERR_MODEL_LOAD_FAILED`；512 字节 `last_error` 窗口内含可操作信息 |
| A8 | 门禁 | 既有测试不改动即通过 |
| A9 | 门禁 | 全量 fmt / clippy / test + 四个算法平台 workspace 全绿（见 `implement.md` §2 Step 9） |

**错误路径测试是本任务的核心**：`A2` / `A4` / `A7` 都必须在"无硬件"条件下断言失败，而开发机（macOS/x86）天然满足该前提，无需 `#[ignore]`。

**硬件相关测试**必须标 `#[ignore]`（AGENTS.md 要求），本任务的测试不依赖真机。

## 6. 回滚形态

改动为加法式，回滚 = 恢复 `GenericDetector::init` 的单行调用 + 移除枚举。
无 ABI 变更、无数据库迁移、无配置破坏性变更，回滚不涉及数据修复。

## 7. 与 `10-01-npu-core-allocation` 的接口对齐

该任务要改 `RknnSessionOptions::core_mask` 与策略透传口。本设计的对齐约定：

- `core_mask` 字段语义、类型、默认值**不变**；
- 新增 `fallback_policy` 为独立字段，不参与 `core_mask` 的组合逻辑；
- `RuntimeSession::open_with_policy` 作为策略透传口，若该任务需要透传 `core_mask`，应在此签名上继续加法式扩展（例如后续增加 options 参数），**不得**改写本任务的 `FallbackPolicy` 语义。
