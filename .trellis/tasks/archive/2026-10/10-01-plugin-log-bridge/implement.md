# Implementation Plan: 插件侧日志桥接宿主 AvLogFn

> 依据 `prd.md` §7 与 `design.md`。**零 ABI 变更**。按 `design.md` §7 的切片 A/B/C 增量交付。

## 1. 执行顺序

```text
[ ] Slice A  捕获并保存宿主 AvLogFn（单点修复，低风险，无新依赖）
    [x] Step 1  HostLogSink 类型 + 存储槽位
    [x] Step 2  在 macros.rs:395 写入（捕获 log/log_user）
    [x] Step 3  library_close 释放持有权（**引用计数**，非无条件停用）
[ ] Slice B  转发 subscriber
    [x] Step 4  依赖选型：**零新增依赖**（tracing 已 re-export 所需 core 类型）
    [x] Step 5  实现 HostLogSubscriber
    [x] Step 6  幂等安装
    [x] Step 7  端到端测试（mock 回调，经真实 C ABI）
[ ] Slice C  健壮性
    [x] Step 8  多 .so 隔离：**已实测**（两个独立 workspace 的 cdylib 各自私有 dispatcher，互不串流）
    [x] Step 9  热路径复核（复用线程局部缓冲，常见路径零显式分配）；体积**已实测**：`.dylib` +27.7 KiB（+1.28%），导出符号集不变
[x]        Step 10 全量门禁
```

**Slice A 可独立提交**：它消除"丢弃回调"这一明确缺陷，不需要新增依赖，风险最低。

## 2. 详细步骤

### Slice A

#### Step 1: `HostLogSink` 类型与存储槽位
- **文件**: `crates/algo-sdk/src/macros.rs`（`LibraryContext` 位于 :65）
- **改动**:
  - 新增 `HostLogSink { log: AvLogFn, user: *mut c_void }`，带 `// SAFETY:` 的 `unsafe impl Send + Sync`；
  - 新增存储槽位。**倾向 `OnceLock<HostLogSink>` + 原子活跃标记**，避免 `AtomicPtr` 需要的真实释放路径（`design.md` §3.4 权衡）。
- **验证**: `cargo test -p algo-sdk --lib` 编译通过（尚未接线）。

#### Step 2: 在 `library_open` 写入
- **文件**: `crates/algo-sdk/src/macros.rs:386`（`__algo_library_open` 内构造 `LibraryContext` 处）
- **改动**: 读取 `raw_args.log` / `raw_args.log_user`（`macros.rs:368` 已有 `raw_args` 绑定），`log` 为 `Some` 时写入槽位；为 `None` 时保持无操作。
- **注意**: `AvAlgoLibraryArgs` 已通过 `validate_abi_header` 校验（`macros.rs:362`），可直接读取字段。
- **验证**: 单元测试用 mock 回调断言"`library_open` 后槽位持有该回调"。

#### Step 3: `library_close` 清理
- **文件**: `crates/algo-sdk/src/macros.rs`（`__algo_library_close`，约 :440 附近）
- **改动**: 清空活跃标记，使转发层不再调用回调。**不**真实释放宿主回调（宿主拥有其生命周期）。
- **验证**: B8 用例（重复 open/close 无泄漏、无 panic）。

### Slice B

#### Step 4: 依赖选型评估（阻塞后续步骤）
- **问题**: `algo-sdk` 当前只依赖 `tracing`，无 `tracing-subscriber`。引入后者会增大**所有算法包**的交付体积。
- **评估项**:
  - 方案 1：引入 `tracing-subscriber`，用 `Layer` API（开发快，依赖重）；
  - 方案 2：直接用 `tracing-core` 实现 `Subscriber`（轻量，需手写 `Interest`/`Event` 处理）；
- **决策依据**: 交付体积实测 + 实现复杂度。**记录结论到 `implement.md` 或代码注释。**
- **注意**: 若最终需要新增依赖，须确认算法包各 workspace 的 lock 同步更新，且四个平台 workspace 均能编译。

#### Step 5: 实现 `HostLogLayer`
- **文件**: 新增 `crates/algo-sdk/src/logging.rs`（或置于 `macros.rs` 邻近模块）
- **改动**:
  - `on_event` 中读取槽位；未激活直接返回；
  - 级别映射：TRACE→0 / DEBUG→1 / INFO→2 / WARN→3 / ERROR→4（**必须与 `loader.rs:891-897` 一致**）；
  - 构造 **NUL 结尾** 缓冲（宿主实现用 `CStr::from_ptr`，`loader.rs:888`）；`len` 填不含 NUL 的字节数；
  - `catch_unwind` 包裹回调调用。
- **热路径要求**: 复用线程局部缓冲，避免每事件分配（`prd.md` §5 约束 2）。
- **验证**: B2/B3/B4 单元测试。

#### Step 6: 幂等安装
- **文件**: `macros.rs` 的 `__algo_library_open` 内
- **改动**:
  ```rust
  let _ = tracing::subscriber::set_global_default(
      tracing_subscriber::registry().with(HostLogLayer)
  );  // Err = 已被占用（后续 open 或插件自身 subscriber）→ 忽略
  ```
- **约束**: 不得 panic；不得覆盖插件自身已安装的 subscriber（`prd.md` B6）。
- **验证**: B6 用例。

#### Step 7: 端到端测试
- **文件**: `crates/algo-sdk/tests/` 或 `macros.rs` 测试模块
- **改动**: mock `extern "C"` 回调（`static AtomicUsize` 计数 + 消息缓存），经 `library_open` 注册后发出各级别事件，断言回调被调用且内容/级别正确。
- **验证**: B1/B2/B3/B4/B5。

### Slice C

#### Step 8: 多库隔离与生命周期
- **改动**: 验证多个 `.so` 各自私有 dispatcher 互不干扰（`design.md` §2.2 待验证推论）。若无法在单元测试中覆盖，转为文档记录 + 手工验证步骤。
- **验证**: B8。
- **实测结论（2026-10-01）**：已用**两个各自独立 workspace** 的临时 cdylib 验证，无需降级为手工步骤。
  - 两个包均从各自的 `algo_sdk::logging::install_host_log_subscriber()` 返回 `1`（各自成功安装）——若 tracing 全局状态被共享，第二个必然返回 `0`；
  - 各自登记回调后各发 3 条事件：`alpha=3, beta=3`，**无串流**；
  - 重复安装幂等，均返回 `0`；宿主侧无 subscriber 时也不 panic。
  - 因探针库为临时产物，未入库为集成测试（入库两个独立 Cargo workspace 会引入无法由根 workspace 门禁覆盖的构建开销）；结论沉淀至 spec「为何必须由插件自装 subscriber」与错误矩阵。

#### Step 9: 体积与热路径复核
- **改动**: 对比改动前后 `.so` 体积；确认日志热路径无锁、无每事件分配。
- **验证**: 记录实测数据。
- **实测数据（2026-10-01，rk3588 人脸包 release）**：

  | 版本 | `.dylib` 字节数 | 差值 |
  | --- | --- | --- |
  | HEAD（`13c4f00`，无桥接） | 2212048 | — |
  | 本任务工作树 | 2240368 | **+28320（+27.7 KiB，+1.28%）** |

  导出符号集**完全一致**（`av_algo_get_abi`/`av_algo_extract_face`/`av_algo_get_gallery_abi`/`av_algo_gallery_bulk`），导出 tracing 符号数 0。
  **不得再写“体积零变化”**：零变化的是依赖图（`Cargo.lock` 无改动），产物大小因转发实现链接而增加。
  热路径：复用 `LOG_BUFFERS` 线程局部缓冲，常见路径零显式分配；无锁（除 `active_sink` 的两次原子读）。

### Step 10: 全量门禁

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
| 引入 `tracing-subscriber` 增大所有算法包体积 | Step 4 先评估 `tracing-core` 自实现 |
| `set_global_default` 与插件自身 subscriber 冲突 | 幂等 + 忽略 Err + B6 测试 |
| NUL 结尾语义理解错误导致宿主越界读 | B4 断言 `msg` 以 `\0` 结尾；交叉引用 `loader.rs:888` |
| 级别映射与宿主漂移 | 双侧互相引用注释 + B3 表驱动 |
| 存储槽位生命周期误用 | 倾向 `OnceLock` + 活跃标记，不做真实释放 |
| 算法包 lock 未同步 | Step 4 结论若含新依赖，逐 workspace 更新并编译 |

**回滚点**: Slice A / B / C 各自独立可回滚。Slice A 为纯加法（新增字段 + 写入），回滚无副作用。

## 4. 完成标准

- `prd.md` §7 的 B1-B9 全部有对应测试或门禁证据；
- 开发机上能演示：注册 mock 回调后发出的 `tracing::warn!` 被回调收到，内容与级别正确；
- 零 ABI 变更（`c_abi_layout_tests.rs` 无改动即通过）；
- 门禁命令全绿，未运行/失败项必须在交付说明中列出。

## 5. 复用清单

- `crates/infer/src/c_abi/loader.rs:877-897` — 宿主 `default_c_logger`（级别映射权威基准）
- `crates/algo-sdk/src/c_abi.rs:75` — `AvLogFn` 签名
- `crates/algo-sdk/tests/c_abi_layout_tests.rs` — ABI 布局回归（B7）
- `crates/algo-sdk/src/macros.rs:353` — `__algo_library_open` 现有结构与 `validate_abi_header` 用法
- `.trellis/spec/guides/logging-guidelines.md` — 级别选择、字段命名、脱敏约束
