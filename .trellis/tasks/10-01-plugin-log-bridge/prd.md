# PRD: 插件侧日志桥接宿主 AvLogFn

> 状态：planning。创建日期 2026-10-01。来源：`09-30-edge-torch-algo-ecosystem` 交接项 H3 排查中的连带发现。

## 1. 目标与用户价值

修复"宿主提供日志回调 → 插件从不消费 → cdylib 内所有 `tracing` 输出被静默丢弃"的断链，使插件侧诊断在生产中可被运维看见。

交付价值：

- 算法包内的降级、失败、硬件异常告警能到达宿主日志与运维页；
- 宿主侧排查算法问题时不再需要"能连上去改代码"；
- 为 `10-01-npu-fallback-observability` 的失败详情提供可视通道（该任务的失败可通过返回值传播，但插件内部诊断不能）。

## 2. 背景与现状 (Problem Statement)

宿主在 `library_open` 时**提供了**日志回调：

```rust
// crates/infer/src/c_abi/loader.rs:594
let args = AvAlgoLibraryArgs {
    ...
    log: Some(default_c_logger),   // 会把插件日志按级别转到宿主 tracing
    log_user: std::ptr::null_mut(),
};
```

`default_c_logger` 已完整实现级别映射（0=trace … 3=warn, else=error，`loader.rs:877-897`）。

但插件侧**把它丢弃了**：

```rust
// crates/algo-sdk/src/macros.rs:386-389
let lib_ctx = Box::new(LibraryContext {
    package_root: std::path::PathBuf::from(root_str),
    platform_id: platform_str.to_string(),
});   // raw_args.log / raw_args.log_user 从未被读取

// crates/algo-sdk/src/macros.rs:65-68
pub struct LibraryContext {
    pub package_root: PathBuf,
    pub platform_id: String,   // 无 log 字段
}
```

同时 `algo-sdk` **不自带 subscriber**（`grep -rn tracing_subscriber crates/algo-sdk/src/` → 0 命中）；`tracing_subscriber` 仅出现在各算法包的 `run_local.rs` / 探针二进制中。

**净效果**：cdylib 内 `algo-sdk` 的 20 处 `tracing::warn/error` 全部进入无 subscriber 的全局 dispatcher，被丢弃。宿主想收，插件有得发，中间的桥是断的。

影响面不止 fallback：所有插件侧硬件异常、预处理降级、池耗尽等诊断同样失明。

## 3. 范围

**In scope**：
- `algo-sdk` 在 `library_open` 捕获 `log` / `log_user` 并保存到库级上下文
- 安装一个 `tracing` Layer/Subscriber，把 SDK 内 `tracing` 事件转发到宿主 `AvLogFn`
- 库级上下文的生命周期管理（与 `library_close` 对应）
- 单元测试

**Out of scope**：
- 修改 `AvLogFn` 签名或 `AvAlgoLibraryArgs` 布局
- 宿主侧 `default_c_logger` 的行为（已正确，不需改）
- 日志采样/限流策略（若需要，单独立项）
- 替换宿主自身的 subscriber 初始化（`app/main.rs`）

## 4. 功能需求

### 4.1 捕获并保存宿主日志回调

`library_open` 必须读取 `raw_args.log` 与 `raw_args.log_user`，存入 `LibraryContext`。为空时退化为无操作（保持既有行为，不 panic）。

### 4.2 转发机制

把 `algo-sdk` 的 `tracing` 事件按级别映射到 `AvLogFn` 调用。要求：

- 映射与宿主 `default_c_logger` 的期望一致（0=trace, 1=debug, 2=info, 3=warn, ≥4=error）；
- 消息必须以宿主可读的形式传递（`default_c_logger` 用 `CStr::from_ptr` 读取，即**需要 NUL 结尾的 C 字符串**）；
- 转发失败（回调为空、字符串含内部 NUL）不得 panic，不得影响业务路径。

### 4.3 不破坏既有行为

- 插件内若已有 subscriber（如算法包的 `run_local.rs`），不得产生冲突或 panic；
- 无宿主回调时必须退化为无操作；
- 现有日志输出来源（宿主 `tracing`）不得被重复或丢失。

### 4.4 生命周期正确性

库级上下文随 `library_open` 建立、`library_close` 销毁。转发器不得在 `library_close` 之后继续持有或访问已释放资源。

## 5. 非功能性约束

1. **零 ABI 变更**：`AvAlgoLibraryArgs` / `AvLogFn` / `AvAlgoAbi` 布局不变。这是纯 SDK 内部补桥。
2. **不得在业务热路径引入额外分配**：日志调用可能高频，格式化与分配策略需明确。
3. **不得 panic**：所有 FFI 回调路径受既有 `catch_unwind` 纪律约束。
4. **不得破坏 `run_local` 与探针二进制**：它们自行 `tracing_subscriber::fmt().init()`，SDK 的转发器必须能与之共存。
5. **日志内容遵循仓库规范**：级别选择、字段命名、脱敏要求见 `.trellis/spec/guides/logging-guidelines.md`（固定中文短消息、变量放英文 snake_case 字段、禁止打印密码/带凭据 URL/像素全量/Base64 图片）。
6. **不使用 `println!/eprintln!` 替代 tracing**。

## 6. 依赖与关联

- **被 `10-01-npu-fallback-observability` 依赖**：该任务的失败详情可经返回值传播，但插件内部的降级告警只能靠本任务打通。建议**本任务先行**。
- **关联**：`crates/algo-sdk/src/runtime/platforms/rockchip.rs:491` 的"启用开发调试回退会话"`warn!` 是本任务最直接的受益点。

## 7. 验收标准 (Acceptance Criteria)

- [x] **B1：回调被捕获** — `LibraryContext.log_bridge_lease` 承载 RAII 持有权；`plugin_lifecycle.rs::test_library_open_bridges_host_log_callback` 经真实 ABI 验证回调**被真实调用**（非仅字段非空）。
- [x] **B2：事件可达** — 同一端到端用例断言消息内容与结构化字段均到达回调。已实测该断言具备判别力（禁用登记后失败：`left: 0, right: 1`）。
- [x] **B3：级别映射正确** — `logging::tests::level_mapping_matches_host_contract`（发五级断言 `[0,1,2,3,4]`）+ `level_mapping_is_total_over_all_levels`；已与宿主 `loader.rs` 逐项核对一致。
- [x] **B4：消息格式正确** — `message_is_nul_terminated_and_interior_nul_is_sanitized`：断言第 `len` 字节为终止符（宿主 `CStr::from_ptr` 的读取前提）+ 内部 NUL 被替换为 U+FFFD 且两段内容均保留。
- [x] **B5：空回调安全** — `library_open` 在 `log` 为 `None` 时不安装、不登记、不计数，其 `library_close` 也不递减。白盒 `logging::tests::none_callback_registers_nothing` + **端到端判别性用例** `plugin_open_failure.rs::none_log_close_does_not_decrement_other_handles`（若 `log = None` 的 close 递减，则会扣掉另一个存活句柄的配额，断言到 `left: 0, right: 1`）；停用路径另见 `inactive_bridge_is_a_noop`。
- [x] **B6：与既有 subscriber 共存** — `install_is_idempotent_and_never_panics`：全局 subscriber 只能安装一次，后续返回 `false` 且不 panic、不覆盖。反向约束（先 open 后 `fmt().init()` 会 panic）已在 `logging.rs` 模块文档与 spec 错误矩阵中声明：仓库内无此类二进制（探针走 rlib、不调 `library_open`）。
- [x] **B7：零 ABI 变更** — `crates/algo-sdk/tests/c_abi_layout_tests.rs` **零改动**（`git diff --numstat` 为 0 行）即通过；`AvAlgoSignedArgs`/`AvLogFn`/`AvAlgoAbi`/`AvFrameDesc` 布局均未触碰。本次仅给 `AvLogFn` 与 `log`/`log_user` 字段**补文档注释**（含跨线程契约），无布局变化；宿主镜像定义 `crates/infer/src/c_abi/types.rs` 同步同一注释。
- [x] **B8：生命周期安全** — 回调永不释放（所有权属宿主）；持有权为 **RAII**，随 `LibraryContext` 析构释放，覆盖正常/失败/unwind 三条路径（`lease_drop_releases_exactly_one_holding`、`lease_carries_no_callback_payload`）；常驻句柄存活期间不被短命句柄的 close 误停用（`bridge_stays_active_until_last_handle_released`）；未配对释放饱和于 0（`release_without_register_saturates_at_zero`）；`repeated_register_and_deactivate_is_safe` 循环 64 次。
- [x] **B10（评审新增）：失败 open 不泄漏持有权** — `plugin_open_failure.rs::failed_open_does_not_leak_log_bridge`：`open_hook` 返回 `Err` 时（该路径宿主永不调用 `library_close`）不得留下计数，含反复失败不累积。**已做变异验证**：把登记结果 `mem::forget`（精确复现修复前语义）后，本用例以「失败 open 不得留下持有权」`left: 1, right: 0` 失败。
- [x] **B9：门禁全绿** — `fmt --check` ✅；`clippy --all-targets -D warnings` ✅；`cargo doc -p algo-sdk --no-deps` 对本次改动零告警 ✅；根 workspace **947 组**测试全绿；四个算法包 workspace（`macos`/`rk3568`/`rk3576`/`rk3588`）fmt+clippy+test 全绿；`git diff --check` ✅。**零依赖变更**（`Cargo.lock` 0 行改动，未引入 `tracing-subscriber`）。**产物体积非常量**：rk3588 人脸包 release `.dylib` `2212048 → 2240368`（**+27.7 KiB，+1.28%**），导出符号集不变、导出 tracing 符号数 0。

## 8. 开放问题（均已定稿）

1. ~~转发器采用全局 subscriber 还是 `tracing::dispatch` 局部作用域？~~
   **已定稿 → `design.md` §2.2**：必须在 cdylib **内部**安装转发 subscriber。
   决定性事实：算法包为 `cdylib`、拥有独立 workspace 与独立 lock，`tracing-core` 被静态链入每个 `.so` 的私有副本，且 `.so` 仅导出 `av_algo_get_abi` / `av_algo_extract_face` 两个符号（`nm -D` 实测）。
   宿主因此无法为插件设置 dispatcher，只能由插件自行安装。
2. ~~是否需要限流/采样？~~
   **已定稿 → 不作限制**，保持与宿主 `default_c_logger` 语义一致（`loader.rs:877-897` 无限流）。
   若后续确有高频噪声需求，单独立项，不在本任务引入。
3. ~~多库实例的回调是否按库隔离？~~
   **已定稿 → `design.md` §3.4**：lib 级静态槽位 + 活跃标记。
   理由：`library_open` 会被多次调用（`package.rs:121`/170/183/243）而全局 subscriber 只能设置一次；
   宿主始终传同一 `default_c_logger`，首次写入即稳定，写幂等。
