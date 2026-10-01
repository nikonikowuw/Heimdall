# Implementation Plan: NPU 硬件回退静默降级可观测性与自检硬门

> 依据 `prd.md` §7 验收标准与 `design.md`。改动为**加法式**，无 ABI 变更、无数据库迁移。

## 1. 执行顺序

```text
[ ] Step 1  FallbackPolicy 枚举 + RknnSessionOptions 加法式字段
[ ] Step 2  HardwareStatus 三态查询（A1 / A3）
[ ] Step 3  open_or_fallback 执行策略（A2）
[ ] Step 4  resolve_fallback_policy 策略解析 + platform_id 归一化（A4 / A5 / A6）
[ ] Step 5  RuntimeSession::open_with_policy 策略透传口（A4）
[ ] Step 6  GenericDetector::init 接入（自检硬门生效）
[ ] Step 7  两个直接调用包同步（rk3568/fire-detections、rk3576/general_detection）
[ ] Step 8  集成测试：自检模式 + 无硬件 → instance_create 失败 + last_error（A7）
[ ] Step 9  全量门禁（A8 / A9）
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

### Step 2: `HardwareStatus` 三态查询
- **文件**: 同上 + `crates/algo-sdk/src/runtime/mod.rs`
- **改动**:
  - 新增 `HardwareStatus { Hardware, Simulated }`；
  - `RknnSession::hardware_status()`；`is_fallback()` 内部委托，签名与语义不变；
  - `RuntimeSession::hardware_status()`（`Fallback` 变体 → `Simulated`）。
- **验证**: 单元测试断言 `is_fallback() == (hardware_status() == Simulated)`。

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
- **文件**: `crates/algo-sdk/src/runtime/mod.rs`（或 SDK 内共享模块）
- **改动**:
  - `platform_requires_hardware(platform_id: &str) -> bool`，含与宿主一致的归一化；
  - `resolve_fallback_policy(is_self_test: bool, platform_id: &str) -> FallbackPolicy`；
  - 归一化别名族覆盖：`linux-rknn` / `linux-arm64-rknn` / `rknn`；`linux-ascend` / `linux-arm64-ascend` / `ascend`。
- **约束**（`A6`）: `algo-sdk` 内**不得**新增按目标 SoC / 宿主 OS 的 `cfg` 分支。实现后通读一遍确认。
- **重复点说明**: 与 `crates/infer/src/sandbox.rs:36` 的 `normalize_platform_id` 是最小必要重复；测试必须断言两者对同一输入族结论一致，并在代码注释中互相引用。
- **验证**: 表驱动单元测试覆盖全别名族 + `macos-arm64-coreml` / `linux-x64` 反例。

### Step 5: `RuntimeSession::open_with_policy`
- **文件**: `crates/algo-sdk/src/runtime/mod.rs:98`
- **改动**: 保留 `open`（委托 `Allow`），新增 `open_with_policy(.., policy)`，内部构造 `RknnSessionOptions` 时带上策略。
- **验证**: `cargo test -p algo-sdk` 既有测试不改动通过（`A8`）。

### Step 6: `GenericDetector::init` 接入
- **文件**: `crates/algo-sdk/src/models/yolo.rs:260`
- **改动**:
  ```rust
  let policy = resolve_fallback_policy(ctx.is_self_test, ctx.platform_id);
  let session = RuntimeSession::open_with_policy(ctx.package_root, &model_file, policy)?;
  ```
- **验证**: 现有 `test_generic_yolo_detector_lifecycle` 等必须仍通过。**已核实**：这些用例构造的 `InitContext` 为 `platform_id: "test-platform"`、`is_self_test: false`（`models/yolo.rs:360-365` 及 :408/:439/:488/:535 同构），`resolve_fallback_policy(false, "test-platform")` → `Allow`，行为不变。
- **待确认**: 若后续有用例传入硬件平台 id（如 `linux-rknn`）且依赖 fallback 成功，需改为断言失败（或显式传 `Allow`）。实现时先 `grep -rn 'platform_id' crates/algo-sdk/src crates/algo-sdk/tests` 复核。

### Step 7: 两个直接调用包同步
- **文件**:
  - `algo-packages/rknn/rk3568/fire-detections/src/plugin.rs:98`
  - `algo-packages/rknn/rk3576/general_detection/src/plugin.rs:92`
- **改动**: 在 `init(ctx)` 内解析策略并传入 `RknnSessionOptions`（各约 2 行）。
- **注意**: 这两个包在非 Linux 开发机上会因 `#[cfg(target_os = "linux")]` 大量代码被裁掉；改动需保证两种 cfg 组合均可编译。
- **验证**: 两个包作用域 `cargo clippy` + `cargo test` 通过。

### Step 8: 集成测试 — 自检硬门
- **文件**: `crates/algo-sdk/tests/`（新增或扩展）
- **改动**: 经 C ABI 以 `AV_INSTANCE_INSTALL_SELF_TEST` 模式 `instance_create`，在无运行时的条件下断言返回 `AV_ERR_MODEL_LOAD_FAILED`，并通过 `last_error` 断言含可定位信息。
- **参照**: `crates/algo-sdk/tests/generic_yolo_lifecycle.rs` 的既有 C ABI 生命周期用例结构。
- **验证**: 用例在开发机（无 RKNN 运行时）上必须真实走到失败分支，不得 `#[ignore]`。

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
git diff --check
```

## 3. 风险与回滚点

| 风险 | 缓解 |
|---|---|
| 既有测试隐式依赖 fallback 行为 | Step 1 完成后立即跑全量 `cargo test -p algo-sdk`，先于任何行为改动暴露 |
| `platform_id` 归一化与宿主漂移 | Step 4 表驱动测试锁定别名族，并与 `infer` 侧常量交叉注释 |
| 平台判定误伤 macOS 开发流 | `A5` 用例断言 `Allow`；算法包 `run_local` 不走 ABI，不受影响 |
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
