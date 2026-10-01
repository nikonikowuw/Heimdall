# Technical Design: 插件侧日志桥接宿主 AvLogFn

> 依据 `prd.md` §4-§5。源码位置于 2026-10-01 在 `dev` 分支核实。

## 1. 缺陷机制与断点定位

| 环节 | 状态 | 位置 |
|---|---|---|
| 宿主提供回调 | ✅ 已实现 | `loader.rs:594` `log: Some(default_c_logger)` |
| 宿主实现回调体 | ✅ 完整（0=trace…3=warn, else=error） | `loader.rs:877-897` |
| 插件读取 `raw_args.log` | ❌ **从未读取** | `macros.rs:386-389` |
| 插件保存回调到上下文 | ❌ `LibraryContext` 无此字段 | `macros.rs:65-68` |
| 插件具备 subscriber | ❌ `algo-sdk` 内 0 处 `tracing_subscriber` | `grep` → 0 命中 |

→ **断点唯一且明确：`macros.rs:386` 构造 `LibraryContext` 时丢弃了 `raw_args.log` / `raw_args.log_user`。**
## 2. 决定性约束：cdylib 拥有私有 tracing 全局状态

这是本设计的核心，且**推翻了"直接复用宿主 subscriber"的直觉方案**。

### 2.1 已核实的事实

| 事实 | 证据 |
|---|---|
| 算法包为 cdylib | `Cargo.toml:12` `crate-type = ["cdylib", "rlib"]` |
| 算法包有**独立 workspace 与独立 lock** | `algo-packages/rknn/rk3568/Cargo.lock` 与根 `Cargo.lock` 各自存在 |
| `.so` 只导出 2 个符号 | `nm -D libface_recognition.so` → 仅 `av_algo_get_abi`、`av_algo_extract_face`（共 87 符号，其余为静态内部） |
| `tracing-core` 被静态链入 | 算法包 lock 内 1 份 `tracing-core 0.1.36`；`.so` 未导出任何 tracing 符号 |

### 2.2 推论

`tracing` 的分发器（dispatcher）是**每个 `tracing-core` 实例一份进程内全局状态**。由于 `.so` 静态链入自己的 `tracing-core` 副本且**不导出**任何相关符号：

1. 宿主**无法**为插件设置 dispatcher（没有可调用的导出符号）；
2. 宿主在 `main.rs` 安装的 `tracing_subscriber::registry()...init()` 只作用于**宿主自己的** `tracing-core`；
3. 插件内 `tracing::warn!` 进入插件的私有 dispatcher——因无人设置而回落到 no-op **Sink**。

→ **结论：必须在插件（cdylib）内部自行安装转发 subscriber。** 这是唯一可行方向，`prd.md` 开放问题 1 由此定稿。

> 待实现期验证：多算法包（多个 `.so`）各自的私有 dispatcher 互不干扰这一推论。拟以"同时装载两个插件，各自收到自己的日志"的集成测试确认。

## 3. 契约与边界

### 3.1 `AvLogFn` 契约解读

```rust
// crates/algo-sdk/src/c_abi.rs:75
pub type AvLogFn =
    unsafe extern "C" fn(user: *mut c_void, level: c_int, msg: *const c_char, len: u32);
```

**关键**：宿主实现 `default_c_logger` **忽略 `len`**，改用 `CStr::from_ptr`（NUL 结尾语义）：

```rust
// crates/infer/src/c_abi/loader.rs:883-888
let c_str = unsafe { CStr::from_ptr(msg) };
let text = c_str.to_string_lossy();
```

→ **插件必须传 NUL 结尾缓冲**，否则宿主会读过界。`len` 目前被忽略，但仍须按契约填正确值（不含 NUL 的字节数），避免依赖宿主的实现细节。

### 3.2 级别映射（复用既有约定）

| `tracing` 级别 | `level` 值 | 宿主映射 |
|---|---|---|
| TRACE | 0 | `tracing::trace!(target: "algo_c")` |
| DEBUG | 1 | `debug` |
| INFO | 2 | `info` |
| WARN | 3 | `warn` |
| ERROR | ≥4 | `error` |

映射必须与宿主 `loader.rs:891-897` 一致——**这是双向契约，两侧需互相引用注释**。

### 3.3 转发的目标范围

只转发 `algo-sdk` 自身产生的事件，**不得**劫持或改变宿主事件。

**定稿（实现后修正）**：**不做 `target` 过滤**。原计划“可对 `target` 前缀做过滤”基于一个已被推翻的前提——cdylib 拥有**私有的** `tracing-core` 副本（§2.2），本订阅器只能看到**本插件自身**（SDK + 算法包代码）发出的事件，宿主事件根本不会进入这个 dispatcher，因此不存在“劫持宿主事件”的风险，过滤无对象。

实测（2026-10-01）：两个各自独立 workspace 的 cdylib 均能成功安装全局 subscriber 且事件互不串流（`alpha=3, beta=3`），进一步证实每个 `.so` 的 dispatcher 是独立的封闭世界。

若将来某个依赖包出现高频噪声，再按 `target` 前缀裁剪。

### 3.4 上下文与生命周期

```rust
// crates/algo-sdk/src/macros.rs:65（改后）
pub struct LibraryContext {
    pub package_root: PathBuf,
    pub platform_id: String,
    /// 宿主日志回调；`None` 时退化为无操作
    pub log_sink: Option<HostLogSink>,
}

/// 宿主日志回调的进程内注册点
#[derive(Clone, Copy)]
pub struct HostLogSink {
    pub log: AvLogFn,
    pub user: *mut c_void,
}
```

`HostLogSink` 需 `unsafe impl Send + Sync` 并附 `// SAFETY:` 说明：回调与 `user` 由宿主在库生命周期内保持有效（宿主在 `library_open` 期间提供，`library_close` 前不释放）。

**存储位置的选择（本设计的关键取舍）**：

`library_open` 会被**多次**调用（`package.rs:121`、170、183、243 均触发 `RawAlgoLibrary::open`），而 `tracing` 的全局 subscriber 只能设置一次。因此回调不能存放在 `LibraryContext` 里让订阅器去读它（订阅器需要 `'static` 访问）。

**决定**：用一个 lib 级 `static` 槽位存放当前回调 + **库句柄引用计数**：

```rust
static HOST_LOG_SINK: OnceLock<HostLogSink> = OnceLock::new();
static OPEN_LIBRARY_COUNT: AtomicUsize = AtomicUsize::new(0);
```

- `library_open`：首次写入 `OnceLock`（后续写入幂等），计数 `+1`；
- `library_close`：计数**饱和递减**；
- 活跃性由计数派生（`count > 0`），**不另设 `AtomicBool`**；
- 回调**永不释放**（所有权属于宿主，且始终是同一 `default_c_logger`）。

使用原子量而非 `RwLock` 是为了让日志热路径**无锁、无分配**（`prd.md` §5 约束 2）。

> **为何不设独立活跃标志位**（实现期修正）：初稿设想「一个原子布尔标记库是否活跃」。
> 但两个独立状态源之间存在丢失更新窗口——`release` 把计数减到 0 的同时另一线程
> `register`，前者的 `store(false)` 可能落在后者的 `store(true)` 之后，造成
> 「计数为 1 但桥接已关」，新句柄的日志被静默丢弃。
> 改为由计数单一派生后，该竞态从根本上不存在。

### 3.5 实现期修正：`library_close` 必须引用计数，不能无条件停用 ⚠️

> **本节修正了设计初稿的错误假设。** 初稿写的是「`library_close` 清空槽位」，隐含假设 `library_close` 只在真正卸载时触发一次。**这个假设是错的。**

已核实的事实：

| 事实 | 证据 |
|---|---|
| `RawAlgoLibrary::drop` 每次都调 `library_close` | `crates/infer/src/c_abi/loader.rs` 的 `impl Drop for RawAlgoLibrary` |
| 常驻 Worker **长期持有**一个库句柄 | `crates/infer/src/package.rs` 的 `create_worker` 把 `raw_lib` 移入 worker 结构体 |
| `extract_face` / `create_gallery` 另开**短命**句柄，用完即关 | `package.rs` 的 `extract_face` 注释：「库级句柄在当前调用线程打开、使用并关闭」 |
| 既有规范已警告此行为 | `algo-sdk-guidelines.md`：「`library_close` 在**每次** `RawAlgoLibrary::drop` 都触发（含 `extract_face` 等高频短操作）」 |

**若无条件停用会出什么事**：常驻 Worker 存活（桥接活跃）→ 业务调用 `extract_face`（open 激活 → 用完 close）→ **桥接被关**→ 常驻 Worker 后续所有日志被静默丢弃。这正是本任务要消灭的「静默降级」，等于用一个 bug 换另一个。

**修正后的契约**：

1. `library_open`（且 `log` 为 `Some`）获取一个持有权；
2. `library_close` 仅在**本次 open 登记过回调**时释放一个持有权；
3. 仅当计数归零（最后一个句柄释放）才停用转发；
4. 计数为 0 时递减必须**饱和**（拒绝回绕），否则计数会变成天文数字而永不归零。

该手法与 [`crate::cv::DefaultEngineLease`](../../../../crates/algo-sdk/src/cv/mod.rs) 处理同类进程级资源完全一致（同样使用 `fetch_update` + `checked_sub` + `previous == Ok(1)` 判定）。

**回归测试**：`logging::tests::bridge_stays_active_until_last_handle_released` 锁定此契约（模拟「常驻句柄 + 短命句柄」两个持有者）。已验证该用例在无条件停用的实现下会失败，具备判别力。

## 4. 转发实现（Step 5 的技术选型）

### 4.1 依赖选型结论：零新增依赖 ✅

> 设计初稿留了一个待评估项：「引入 `tracing-subscriber` vs 用 `tracing-core` 直接实现 `Subscriber`」。
> **实现期结论：两者都不需要——连 `tracing-core` 都不用加。**

关键事实：`tracing` 已经把所需 core 类型全部 re-export，因此可以在**不新增任何依赖**的前提下手写 `Subscriber`：

| 需要的东西 | 来自 |
|---|---|
| `Subscriber` trait | `tracing::Subscriber` |
| `Event` / `Metadata` / `Level` | `tracing::{Event, Metadata, Level}` |
| `span::Id` / `span::Attributes` / `span::Record` | `tracing::span::*` |
| `field::{Field, Visit}` | `tracing::field::*` |
| `subscriber::{set_global_default, Interest}` | `tracing::subscriber::*` |

`tracing 0.1.44` 本就依赖 `tracing-core 0.1.36`，因此这些类型与 trait 与 `tracing` 内部使用的**是同一个 crate 实例**，`set_global_default` 可直接接受自定义实现。

**收益**：四个算法包 workspace 的 `Cargo.lock` **零变化**（已用 `git status -- '*Cargo.lock'` 验证），所有 cdylib 交付体积不受影响。

### 4.2 转发实现

```rust
/// 把 algo-sdk 的 tracing 事件转发到宿主 AvLogFn
struct HostLogLayer;

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for HostLogLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        // 1. 读 HOST_LOG_SINK；空则直接返回（无操作）
        // 2. 收集字段到线程局部 String（复用缓冲，避免每次分配）
        // 3. 构造 NUL 结尾 CString
        // 4. catch_unwind 包裹闭包体（注意：对 extern "C" 回调内的 panic 无效，见 §4.3）
        let _ = std::panic::catch_unwind(|| unsafe { log(user, level, ptr, len) });
    }
}
```

安装时机与冲突处理：

```rust
// library_open 内，幂等
let _ = tracing::subscriber::set_global_default(
    tracing_subscriber::registry().with(HostLogLayer)
);   // 返回 Err(SetGlobalDefaultError) 表示已被占用 → 忽略，不 panic
```

**为什么不 panic**：`set_global_default` 只能成功一次。同进程内后续 `library_open` 必然失败，属预期；且插件若在自身 `run_local`/探针二进制中已安装 `fmt().init()`，SDK 的安装必须让路而非崩溃（`prd.md` B6）。

**新增依赖评估**：需要 `tracing-subscriber`（`algo-sdk` 当前仅依赖 `tracing`）。这会进入所有算法包的交付体积。**需在 Step 3 评估**：是否可用更轻的 `tracing-core` 直接实现 `Subscriber` 从而避免引入 `tracing-subscriber`。倾向先评估，避免为 20 处日志引入不必要的依赖。

### 4.3 实现期修正：`catch_unwind` 对 `extern "C"` 回调内的 panic 无效 ⚠️

> 设计初稿与风险表都写的是「用 `catch_unwind` 防止宿主回调 panic 穿透」。**实测证明该手段对本题无效。**

最小复现（`rustc -O` 实测）：

```rust
unsafe extern "C" fn panicking_cb() { panic!("回调查崩了"); }
std::panic::catch_unwind(|| unsafe { panicking_cb() });   // 拦不住
```

运行结果：

```
panic in a function that cannot unwind
thread caused non-unwinding panic. aborting.
退出码 134 (SIGABRT)
```

原因：`extern "C"` 函数带**不可 unwind 守卫**，panic 在离开该函数时就被转成 abort，控制流根本回不到调用方的 `catch_unwind`。

**结论与责任划分**：

- 宿主回调的 panic 隔离**只能由宿主自己做**——`default_c_logger` 已把函数体包在 `catch_unwind` 内（`crates/infer/src/c_abi/loader.rs:883`），这是正确的位置；
- 插件侧的 `catch_unwind` 保留，但**只**兜闭包体自身的 unwind，并在 `AvLogFn` 未来迁移到 `extern "C-unwind"` 时继续生效。代码注释已按此如实描述，不再声称它能拦住宿主 panic；
- **因此不写「回调 panic 被隔离」的测试**：在 `extern "C"` 下不可断言（写了只会把进程打崩）。

**附带发现（测试设计陷阱）**：`HOST_LOG_SINK` 是 `OnceLock`（set-once，与生产语义一致），因此同一测试二进制内所有用例必须登记**同一个**回调。初稿曾有一个登记「会 panic 的回调」的用例，约 1/190 概率抢先写入槽位，导致后续任意用例转发时进入它 → SIGABRT 崩掉整个测试进程（退出码 134、无断言失败信息，极易误判为环境问题）。该用例已移除，约束已写入代码注释与规范。

## 5. 数据流

```
宿主 library_open
   │  AvAlgoLibraryArgs { log: Some(default_c_logger), log_user: null }
   ▼
macros.rs:386  保存到 HOST_LOG_SINK（本次修复点）
   │  安装 HostLogLayer（幂等，失败忽略）
   ▼
插件内 tracing::warn!("未检测到物理 librknnrt.so，启用开发调试回退会话")
   │  进入 .so 私有 dispatcher（宿主不可见）
   ▼
HostLogLayer::on_event
   │  级别映射 + NUL 结尾编码
   ▼
default_c_logger  →  tracing::warn!(target: "algo_c")  →  宿主 subscriber
   │
   ▼
stdout + journald  →  运维可见
```

## 6. 测试策略

| 验收项 | 测试 | 断言要点 |
|---|---|---|
| B1 | 单元 | mock `AvLogFn` 记录调用；断言 `library_open` 后发出事件**确实调用**了回调 |
| B2 | 单元 | 事件内容出现在 mock 记录中（而非仅"回调被调过"） |
| B3 | 单元 | 表驱动：五级 → 五个 `level` 值 |
| B4 | 单元 | 断言收到的 `msg` 以 `\\0` 结尾；构造含内部 NUL 的消息不 panic、不越界 |
| B5 | 单元 | `log = None` 时发出事件不 panic，业务返回值不变 |
| B6 | 单元 | 预先安装 subscriber 后再 `library_open`，不 panic |
| B7 | 门禁 | `c_abi_layout_tests.rs` 继续通过，无尺寸/偏移变化 |
| B8 | 单元 | 重复 `library_open`/`library_close` 循环不泄漏、不 panic |

**mock 回调的必要性**：不能用宿主 `default_c_logger` 验证（它是 `unsafe extern "C"` 私有函数且写入宿主 tracing）。测试自建 `static AtomicUsize` 计数 + 消息缓存，导出 `extern "C"` 适配器。

**无硬件依赖**：全部用例可在开发机运行，无需 `#[ignore]`。

## 7. 增量交付切片

`A` → `B` → `C` 每段独立可验证，可分批提交：

| 切片 | 内容 | 独立价值 |
|---|---|---|
| **A** | 捕获并保存 `AvLogFn` 到 `HostLogSink`（`macros.rs:386`） | 消除"丢弃回调"这一明确缺陷；为 B 铺路 |
| **B** | 安装转发 subscriber，用 mock 回调验证端到端 | 日志真正到达宿主 |
| **C** | `library_close` 清理 + 多库隔离 + 依赖体积复核 | 生产健壮性 |

**建议先独立落地切片 A**：它是单点、低风险、可立即验证的修复，且不需要新增依赖。

## 8. 回滚形态

纯 SDK 内部改动，零 ABI 变更、零数据库迁移。回滚 = 移除 subscriber 安装 + 保留 `HostLogSink` 字段（无害）。

## 9. 风险

| 风险 | 缓解 |
|---|---|
| 引入 `tracing-subscriber` 增大所有算法包交付体积 | Step 3 评估用 `tracing-core` 直接实现 `Subscriber` |
| `set_global_default` 与插件内既有 subscriber 冲突 | 幂等安装 + 忽略错误 + B6 测试 |
| `AtomicPtr`/`OnceLock` 的生命周期误用导致 use-after-free | 倾向 `OnceLock` + 活跃标记，不做真实释放；`library_close` 后不转发 |
| 宿主回调 panic 穿透插件 | **不可行**：`AvLogFn` 是 `extern "C"`，回调内 panic 直接 abort，外层 `catch_unwind` 拦不到。隔离责任在宿主（`default_c_logger` 自带 `catch_unwind`）；插件侧保留 `catch_unwind` 仅为兜住闭包体自身 unwind（见 §4.3） |
| 日志热路径分配开销 | 线程局部复用缓冲；`target` 前缀过滤减少进入转发层的事件数 |
| 与宿主 `default_c_logger` 的级别约定漂移 | 双侧互相引用注释 + B3 表驱动测试 |
