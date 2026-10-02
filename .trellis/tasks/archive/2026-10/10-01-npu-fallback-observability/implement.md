# Implementation Plan: NPU 硬件回退静默降级可观测性与自检硬门

> 依据 `prd.md` §7 验收标准与 `design.md`。改动为**加法式**，无 ABI 变更、无数据库迁移。

## 1. 执行顺序

```text
[x] Step 1  FallbackPolicy 枚举 + RknnSessionOptions 加法式字段
[x] Step 2  HardwareStatus / HardwareAvailability 三态查询（A1 / A3）
[x] Step 3  open_or_fallback 执行策略（A2）
[x] Step 4  resolve_fallback_policy 策略解析 + platform_id 归一化（A4 / A5 / A6）
[x] Step 5  RuntimeSession::open_with_policy 策略透传口
[x] Step 6  GenericDetector::init 接入（自检硬门生效）
[x] Step 7  两个直接调用包同步 + 本地工具显式 Allow
[x] Step 8  集成测试：自检模式 + 无硬件 → instance_create 失败 + last_error（A7）
[x] Step 9  全量门禁（A8 / A9）
```

Step 1-3 与 Step 4-5 无依赖，可并行；Step 6 依赖 4 与 5；Step 7 依赖 4。

## 2. 详细步骤

### Step 1: `FallbackPolicy` 与 `RknnSessionOptions` 扩展
- **文件**: `crates/algo-sdk/src/runtime/platforms/rockchip.rs`（`RknnSessionOptions` 定义位于 :357）
- **改动**:
  - 新增 `FallbackPolicy { Allow, RequireHardware }`，`#[derive(Default)]` 且 `#[default]` 标在 `Allow`；
  - `RknnSessionOptions` 增加 `pub fallback_policy: FallbackPolicy`；
  - `Default for RknnSessionOptions` 设为 `Allow`（既有行为）；
  - `new` / `with_core_mask` 不动（保持既有签名），可加法式增加 `with_fallback_policy`。
- **兼容性检查**: `grep -rn "RknnSessionOptions" crates/ algo-packages/` 确认所有构造点仍编译且行为不变。
- **验证**: `cargo test -p algo-sdk --lib` 既有测试不改动即通过。

### Step 2: `HardwareStatus` / `HardwareAvailability` 查询
- **文件**: `crates/algo-sdk/src/runtime/fallback.rs`, `crates/algo-sdk/src/runtime/mod.rs`, `crates/algo-sdk/src/runtime/platforms/rockchip.rs`
- **改动**:
  - `HardwareStatus { Hardware, Simulated }` 表示成功构造的会话；
  - `HardwareAvailability { Hardware, Simulated, Unavailable }` 与 `classify_session_availability` 提供构造结果三态；
  - `hardware_status()`；保留 `is_fallback()` 并令其委托状态查询。
- **验证**: 单元测试覆盖三态映射并断言 `is_fallback() == (hardware_status() == Simulated)`。

### Step 3: `open_or_fallback` 执行策略
- **文件**: `crates/algo-sdk/src/runtime/platforms/rockchip.rs:483`
- **改动**:
  ```rust
  match RknnRuntime::load(package_root) {
      Ok(rt) => Self::new_with_core_mask(rt, model_path, options.core_mask),
      Err(e) => match options.fallback_policy {
          FallbackPolicy::Allow => {
              tracing::warn!(reason = ?e, "未检测到物理 librknnrt.so，启用开发调试回退会话");
              Self::new_fallback(model_path)
          }
          FallbackPolicy::RequireHardware => Err(AlgoError::ModelLoad {
              reason: format!(寻址信息 + 底层原因 + 操作指引),
          }),
      },
  }
  ```
- **错误信息要求**（`design.md` §2.4）: 必须含 `package_root`、模型路径、底层加载失败原因、`ALLOW_CPU_FALLBACK=1` 操作指引。
- **验证**: 单元测试断言 `Err` 且变体为 `AlgoError::ModelLoad`（**不是**仅 `is_err()`）。

### Step 4: `resolve_fallback_policy` 与平台判定
- **文件**: `crates/algo-sdk/src/runtime/fallback.rs`
- **改动**:
  - `platform_requires_hardware(platform_id: &str) -> bool`，含与宿主一致的归一化；
  - `resolve_fallback_policy(is_self_test, platform_id, env, explicit)`；
  - 优先级：自检硬门 → `InitContext::fallback_policy_override` → `.env` 显式 `ALLOW_CPU_FALLBACK=1` → 硬件 `platform_id` → 默认 `Allow`；
  - 自检硬门不得被 override 或 `.env` 翻越；生产 C ABI 始终传 `explicit=None`；本地开发工具通过 builder 显式传 `Allow`。
- **约束**（`A6`）: `algo-sdk` 内**不得**新增按目标 SoC / 宿主 OS 的 `cfg` 分支。实现后通读一遍确认。
- **重复点说明**: 与 `crates/infer/src/sandbox.rs` 的 `normalize_platform_id` 是最小必要重复；SDK 与宿主测试覆盖相同别名族。
- **验证**: 表驱动单元测试覆盖全别名族 + `macos-arm64-coreml` / `linux-x64` 反例，并测试自检硬门对两种 Allow 覆盖均有效。

### Step 5: `RuntimeSession::open_with_policy`
- **文件**: `crates/algo-sdk/src/runtime/mod.rs:98`
- **改动**: 保留 `open`（委托 `Allow`），新增 `open_with_policy(.., policy)`，内部构造 `RknnSessionOptions` 时带上策略。
- **验证**: `cargo test -p algo-sdk` 既有测试不改动通过（`A8`）。

### Step 6: `GenericDetector::init` 接入
- **文件**: `crates/algo-sdk/src/models/yolo.rs:260`
- **改动**:
  ```rust
  let policy = RuntimeSession::resolve_policy(
      ctx.is_self_test,
      ctx.platform_id,
      Some(&env),
      ctx.fallback_policy_override,
  );
  let session = RuntimeSession::open_with_policy(ctx.package_root, &model_file, policy)?;
  ```
- **验证**: 现有 `test_generic_yolo_detector_lifecycle` 等必须仍通过。硬件平台 `platform_id` 下无 runtime 时普通 C ABI 实例失败；本地工具显式声明 `Allow` 后保留模拟路径。自检时即使显式 Allow 或 `.env` 为 Allow 也必须失败。

### Step 7: 两个直接调用包与本地工具同步
- **文件**:
  - `algo-packages/rknn/rk3568/fire-detections/src/plugin.rs`
  - `algo-packages/rknn/rk3576/general_detection/src/plugin.rs`
  - 相关 `run_local.rs` 与 RKNN 人脸工具中的 `InitContext` 字面量
- **改动**: 两个插件在 `init(ctx)` 内使用统一策略解析器，并把策略放入 `RknnSessionOptions`；其 `run_local` 显式选择 `FallbackPolicy::Allow`。硬件人脸工具不走模拟路径，显式写 `fallback_policy_override: None`。
- **验证**: 两个包作用域 `cargo clippy` + `cargo test` 通过；四个平台 workspace 均验证。

### Step 8: 集成测试 — 自检硬门
- **文件**: `crates/algo-sdk/tests/fallback_policy_gate.rs`
- **改动**: 经真实 C ABI 虚表，以 `AV_INSTANCE_INSTALL_SELF_TEST` 模式调用 `instance_create`；无 `librknnrt` 时断言返回 `AV_ERR_MODEL_LOAD_FAILED`、实例为空，且宿主同尺寸 512 字节 `last_error` 含运行时、包路径和操作指引。另覆盖硬件平台普通实例及非硬件平台开发 fallback。
- **验证**: 开发机无 RKNN runtime 时真实走失败分支，不使用 `#[ignore]`；安装自检和普通硬件平台测试在检测到本机 runtime 时明确跳过前提不成立的用例。

### Step 9: 全量门禁
```bash
cargo fmt --all && cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo nextest run --workspace          # 或 cargo test --workspace
for manifest in algo-packages/macos/Cargo.toml \
                algo-packages/rknn/rk3568/Cargo.toml \
                algo-packages/rknn/rk3576/Cargo.toml \
                algo-packages/rknn/rk3588/Cargo.toml; do
  cargo fmt --manifest-path "$manifest" --all -- --check
  cargo clippy --manifest-path "$manifest" --workspace --all-targets -- -D warnings
  cargo nextest run --manifest-path "$manifest" --workspace
done
# Linux target cross-checks for target_os-gated run_local/plugin code
git diff --check
```

**执行结果**：workspace 977 tests；`algo-sdk --features rknn` 174 tests；macOS / RK3568 / RK3576 / RK3588 workspace 分别 37 / 88 / 34 / 67 tests 全通过。`cargo fmt`、workspace 与平台 clippy、Linux cross-target 检查及 `git diff --check` 全通过。

## 3. 风险与回滚点

| 风险 | 缓解 |
|---|---|
| 既有测试隐式依赖 fallback 行为 | Step 1 完成后立即跑全量 `cargo test -p algo-sdk`，先于任何行为改动暴露 |
| `platform_id` 归一化与宿主漂移 | Step 4 表驱动测试锁定别名族，并与 `infer` 侧常量交叉注释 |
| 平台判定误伤 macOS 开发流 | `A5` 用例断言非硬件平台默认 `Allow`；**rknn 包的 `run_local` 不受此保护**——它直接调用插件 `init`，与 C ABI 走同一条策略解析路径，因此在这些开发工具中显式声明 `FallbackPolicy::Allow`（`InitContext::with_fallback_policy_override`），不依赖 `.env` 逃生口 |
| Step 6 改变 `GenericDetector` 在 rknn 平台的既有测试行为 | Step 6 前先跑一遍确认真实行为，再改；用 `cargo test -p algo-sdk` 对照 |
| 与 `10-01-npu-core-allocation` 的 `RknnSessionOptions` 冲突 | `design.md` §7 的加法式约定；若对方先动 `core_mask`，本任务只 rebase 不改语义 |

**回滚点**: 每个 Step 独立可回滚。最坏情况回滚 Step 6 单行调用即恢复既有行为，其余新增类型为纯加法、无副作用。

## 4. 完成标准

- `prd.md` §7 的 A1-A9 全部有对应测试或门禁证据；
- 开发机上能演示：无 `librknnrt` 时自检模式 `instance_create` 返回 `-5`，并给出可定位 `last_error`；
- 开发机上能演示：非硬件平台（macOS）默认策略下 fallback 仍可用（`Ok` + `Simulated`）；
- 门禁命令全绿，未运行/失败项必须在交付说明中列出。

## 5. 复用清单

- `crates/algo-sdk/tests/generic_yolo_lifecycle.rs` — C ABI 生命周期测试结构
- `crates/algo-sdk/src/testing.rs` — `MockSession` / `LocalPluginRunner` / `MockFrameBuilder`
- `crates/algo-sdk/src/error.rs` — `AlgoError::to_c_status` 映射（`ModelLoad → -5`）
- `crates/algo-sdk/src/macros.rs:788` — `set_last_error` 错误详情通道
- `crates/infer/src/sandbox.rs:36` — 宿主侧 `normalize_platform_id`（归一化对齐基准）
