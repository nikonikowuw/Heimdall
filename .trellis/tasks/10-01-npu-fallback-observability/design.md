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

1. `AlgoPlugin::init` 的 `InitContext` 同时携带 `is_self_test` 与 `platform_id`（`plugin.rs:19-23`）；
2. `is_self_test` 由 `raw_args.mode == AV_INSTANCE_INSTALL_SELF_TEST` 派生（`macros.rs:558`）；
3. `instance_create` 返回非 `AV_OK` 时，宿主的自检流程**已经会失败**（`sandbox.rs:838`）并通过 `last_error` 取回原因（`macros.rs:788`）。

→ **硬门可完全落在 `algo-sdk` 内，宿主零改动**（§4.4）。

## 2. 契约与边界

### 2.1 新增类型（`A1`）

```rust
// crates/algo-sdk/src/runtime/platforms/rockchip.rs（或 runtime 内共享位置）

/// 无硬件时的行为策略
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FallbackPolicy {
    /// 无硬件时降级为模拟会话（开发/调试）
    #[default]
    Allow,
    /// 无硬件时必须失败，不得返回模拟会话
    RequireHardware,
}

#[derive(Debug, Clone, Copy)]
pub struct RknnSessionOptions {
    pub core_mask: c_int,
    /// 加法式新增；`Default` 即既有行为
    pub fallback_policy: FallbackPolicy,
}
```

**加法式保证**：`fallback_policy` 的 `Default` 为 `Allow`，与既有行为逐位一致。所有既有 `RknnSessionOptions::with_core_mask(...)` / `RknnSessionOptions::default()` 构造点行为不变（`A8`）。

### 2.2 三态可判别（`A3`）

现状是布尔 `is_fallback()`，无法区分"需硬件但不可用"。因 `RequireHardware` 失败即返回 `Err`，该态不会产生会话对象，故**三态只需在"成功构造"域内区分两态 + 用 `Err` 表达第三态**：

```rust
/// 会话的硬件可用性状态
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HardwareStatus {
    /// 真实硬件运行时已就绪
    Hardware,
    /// 开发调试模拟会话（未使用硬件）
    Simulated,
}

impl RknnSession {
    pub fn hardware_status(&self) -> HardwareStatus { /* backend 映射 */ }
    /// 保留既有布尔查询，内部委托 hardware_status，避免破坏调用点
    pub fn is_fallback(&self) -> bool { self.hardware_status() == HardwareStatus::Simulated }
}
```

`RuntimeSession` 同步提供 `hardware_status()`，并对 `Fallback` 变体返回 `Simulated`。

> 三态语义：`Hardware` = 真硬件；`Simulated` = 模拟会话；`Err(...)` = 需硬件但不可用。

### 2.3 策略解析（`A4` / `A5` / `A6`）

```rust
/// 宿主平台标识 → 是否必须真实硬件
///
/// 判据来自宿主自报的 platform_id，**不使用 cfg**：
/// 同一进程可能同时装载多平台算法包，而 algo-sdk 无法用 cfg 表达“宿主是什么”。
fn platform_requires_hardware(platform_id: &str) -> bool {
    matches!(normalize_platform_id(platform_id), "linux-rknn" | "linux-ascend")
}
```

归一化必须与宿主一致：宿主在 `package.rs:106` 用 `normalize_platform_id` 校验 `manifest.platform_id == current_platform_id()`，rsknn 包的 manifest 统一写 `linux-rknn`（**不含 SoC**，已核实 6 个 rknn 包）。

```rust
/// 解析回退策略
pub fn resolve_fallback_policy(is_self_test: bool, platform_id: &str) -> FallbackPolicy {
    if is_self_test {
        return FallbackPolicy::RequireHardware;   // 自检不得用模拟冒充
    }
    if platform_requires_hardware(platform_id) {
        return FallbackPolicy::RequireHardware;
    }
    FallbackPolicy::Allow
}
```

因为 `algo-sdk` 不能反向依赖 `crates/infer` 的 `normalize_platform_id`，归一化逻辑在 SDK 侧做**最小必要重复**（仅 4 个别名族），并以单元测试锁定与宿主一致。**这是本设计唯一的重复点，必须在测试中显式断言。**

### 2.4 `RequireHardware` 的失败语义（`A2`；开放问题 1 定稿）

**决定**：返回 `AlgoError::ModelLoad { reason }`。

理由：
- `AlgoError::ModelLoad` 已映射到 `AV_ERR_MODEL_LOAD_FAILED = -5`（`error.rs:48`），语义贴合"模型未能加载到目标运行时"；
- **零新增状态码**，`AV_ALGO_API_VERSION` 不变，零 ABI 变更（满足 §5 约束 4）；
- 沿既有链路自然变响：`init` 返回 `Err` → `macros.rs:787-790` `set_last_error` + `to_c_status()` → `instance_create` 返回 `-5` → 宿主 `check_c_status` 取回 `last_error` 详情 → `InferError::CAbiError`。

`reason` 必须包含可定位信息（`prd.md` §5 约束 5「失败信息可操作」）：`package_root`、模型路径、底层加载失败原因、以及"如需在无硬件环境运行请显式设置 `ALLOW_CPU_FALLBACK=1`"的操作指引。

### 2.5 声明式路径的策略透传（`A4` / `A8`）

`GenericDetector::init` 目前调用 `RuntimeSession::open(ctx.package_root, &model_file)`（`models/yolo.rs:260`），无策略入参。

```rust
impl RuntimeSession {
    /// 既有入口保持签名与行为（A8）
    pub fn open(package_root: &Path, model_rel_path: &Path) -> Result<Self, AlgoError> {
        Self::open_with_policy(package_root, model_rel_path, FallbackPolicy::Allow)
    }

    /// 加法式新增：显式策略入口
    pub fn open_with_policy(
        package_root: &Path,
        model_rel_path: &Path,
        policy: FallbackPolicy,
    ) -> Result<Self, AlgoError> { /* ... */ }
}
```

`GenericDetector::init` 改为：

```rust
let policy = resolve_fallback_policy(ctx.is_self_test, ctx.platform_id);
let session = RuntimeSession::open_with_policy(ctx.package_root, &model_file, policy)?;
```

**关键收益**：自检硬门在此处自动生效，无需宿主改动。

## 3. 数据流

```
宿主六步沙箱自检
   │  instance_create(mode = AV_INSTANCE_INSTALL_SELF_TEST)
   ▼
macros.rs:558  is_self_test = true
   │
   ▼
GenericDetector::init
   │  resolve_fallback_policy(true, "linux-rknn") → RequireHardware
   ▼
RuntimeSession::open_with_policy(.., RequireHardware)
   │
   ▼
RknnSession::open_or_fallback
   │  RknnRuntime::load 失败
   ▼
   ├─ Allow            → new_fallback() → Ok(Simulated)  ← 仍可用于开发机
   └─ RequireHardware  → Err(ModelLoad)                  ← 自检在此中断
                              │
                              ▼
                    macros.rs:787 set_last_error + AV_ERR_MODEL_LOAD_FAILED
                              │
                              ▼
                    sandbox.rs:838 create_code != AV_OK → SandboxValidation 失败
                              │  step = "5.算法库 C ABI 导出符号核对"
                              ▼
                    自检拒绝该算法包（不再有 2 个假框的 detections_count）
```

## 4. 边界与取舍

### 4.1 为什么不用 `.env` 作为策略来源（开放问题 3）

`.env` 被 `.gitignore` 忽略（`.env.example` 仅为模板），新设备上必然缺失。以"部署时不存在的东西"作为安全开关是循环依赖。
**决定**：`.env` 仅作为**显式覆盖**（`ALLOW_CPU_FALLBACK=1` 时强制 `Allow`），默认不从 `.env` 读取策略。

### 4.2 为什么平台判定不用 `cfg`

`algo-sdk` 被编译进各平台算法包，编译期 `cfg` 只能表达"本包编译成什么"，不能表达"宿主运行在什么平台"。且 SDK 内新增 `cfg` 分支会与 AGENTS.md「平台差异只允许收敛在 media/infer 及其 FFI 实现内」的收敛方向相悖。

### 4.3 为什么 `RequireHardware` 适用于普通实例（非仅自检）

仅对自检加门会留下死角：**生产常规实例仍会静默降级**。既然宿主已强制算法包与平台匹配（`package.rs:106`），那么"在 `linux-rknn` 平台上声明 `linux-rknn` 的包"就应当真的用上硬件；用不上就是部署错误，应当响亮失败。

### 4.4 宿主是否需要改动

**硬门不需要**（§1.4 事实 3：`instance_create` 非 `AV_OK` 已使自检失败）。

建议的**可选防御性增强**（非本任务验收门槛）：在自检第 6 步的 `SelfTestReport` 中记录硬件状态，使自检报告能区分"真实推理 N 个目标"与"模拟会话 N 个目标"。当前 `SelfTestReport` 只有 `detections_count`，无法自证。**若实施，需评估是否扩展 `SelfTestReport` 字段及其对既有测试的影响。**

### 4.5 需要一并更新的既有调用点

两个包**直接**调用 `open_or_fallback`，不走 `RuntimeSession`：

| 包 | 位置 |
|---|---|
| `algo-packages/rknn/rk3568/fire-detections` | `src/plugin.rs:98` |
| `algo-packages/rknn/rk3576/general_detection` | `src/plugin.rs:92` |

它们已在 `init(ctx)` 内持有 `ctx.platform_id`，改为传入解析后的策略即可（各约 2 行）。不更新则这两个包保持当前（不安全）行为。

## 5. 测试策略

| 验收项 | 测试 | 断言要点 |
|---|---|---|
| A1 | 单元 | `FallbackPolicy::default() == Allow`；`RknnSessionOptions` 既有构造点行为不变 |
| A2 | 单元 | 无运行时 + `RequireHardware` → `Err(ModelLoad)`；断言错误**变体**而非仅 `is_err()` |
| A3 | 单元 | `hardware_status()` 三态映射；`is_fallback()` 与 `Simulated` 等价 |
| A4 | 单元 | `resolve_fallback_policy(true, *) == RequireHardware`（平台无关） |
| A5 | 单元 | `resolve_fallback_policy(false, "darwin-aarch64") == Allow`；无驱动回退仍返回 `Ok(Simulated)` |
| A6 | 单元 | 归一化与宿主一致：对 `linux-rknn` / `linux-arm64-rknn` / `rknn` / `linux-ascend` / `ascend` 全族断言；对 `macos-arm64-coreml` 等断言 `Allow` |
| A7 | 集成 | 自检模式 + 无硬件 → `instance_create` 返回 `AV_ERR_MODEL_LOAD_FAILED`；`last_error` 含可定位信息 |
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
