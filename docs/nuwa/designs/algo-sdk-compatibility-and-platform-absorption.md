# Design: 算法 SDK 兼容性契约与平台层收敛 (Algorithm SDK Compatibility Contract & Platform Layer Absorption)

> **状态**: Proposed（规划草案；本文档定义的源码兼容契约、稳定性分区、RKNN 平台层收敛、契约通道启用与兼容性门禁均未实现）
> **目标**: 将算法包 SDK 从"可用工具集"升级为"可承诺契约"：平台硬件样板收敛进 SDK，源码面只加不破，运行时 ABI 冻结演进，使 SDK 变更不再要求全部算法包同步修改源码。
> **关联规范**: [算法 SDK 与沙箱](../backend/algo-sdk-guidelines.md)、[FFI 规范](../backend/ffi-guidelines.md)、[架构概览](../guides/architecture-overview.md)、[全局约定](../guides/conventions.md)、[质量与测试](../backend/quality-guidelines.md)

---

## 1. 背景与结论

当前 SDK 已经完成"实现即合规"的插件契约：`AlgoPlugin` trait 必填 `init`/`process`，`export_algo!` 宏单点生成 11 个 C ABI 入口并内置 panic 隔离、有界解析、规则坐标校验与引擎租约。运行时 ABI v1 也已具备加法演进能力：96 字节冻结虚表、逐结构 `size`/`api_version` 头、可选导出符号由宿主探测降级。

但算法包侧仍存在两类问题：

1. **样板未收敛**：RKNN 运行时绑定在 6 个包中重复 7,107 行（RK3568/RK3588 人脸版差异仅约 20 行）；模型定位、配置三阶覆盖、平台 stub 等样板逐包复制；`instance_negotiate` 与 `min_adapter_version` 两条契约通道已定义但未启用。
2. **源码契约无政策**：SDK 公开面没有 `#[non_exhaustive]`、没有语义化版本纪律、没有 CI 门禁；`InitContext` 由用户以结构体字面量构造，加一个字段就会穿透到多个开发二进制。

本设计的结论：

1. 兼容性必须按**两个契约**分别承诺：运行时契约（宿主 ↔ `.so` 制品）冻结演进；源码契约（SDK ↔ 包源码）"只加不破"。
2. 平台样板（首要是 RKNN 绑定）以**绞杀者模式**收敛进 SDK，是"用户只写算法逻辑"目标的最后一块拼图。
3. 已定义未启用的契约通道按"默认关闭、显式启用"原则接通；所有启用都必须对旧包保持零改动。
4. 兼容性不是纪律问题而是门禁问题：semver 检查、全矩阵编译与旧制品实证必须能随每次 SDK 变更执行。

---

## 2. 当前实现与差异

### 2.1 已有能力

| 能力 | 实现入口 | 对兼容性的意义 |
| --- | --- | --- |
| trait 必填 + 默认方法 | [`AlgoPlugin`](../../../crates/algo-sdk/src/plugin.rs)（`flush`/`update_config`/`set_rules` 带默认实现） | 新增能力可走"默认方法"，旧包零改动 |
| 宏单点生成 ABI | [`export_algo!`](../../../crates/algo-sdk/src/macros.rs) | ABI 签名只有一处真源，用户无法写错 |
| 虚表冻结与版本门 | [`AvAlgoAbi`](../../../crates/algo-sdk/src/c_abi.rs)、[`LoadedLib::load`](../../../crates/infer/src/c_abi/loader.rs) | `av_algo_get_abi(requested)` 不匹配返回空指针；宿主对 `AvAlgoAbi.size` 精确校验 |
| 可选符号扩展 | `av_algo_extract_face`、`av_algo_get_gallery_abi`、`av_algo_gallery_bulk` | 新能力 = 新符号 + 探测 + 降级，已有"加法式演进，不动虚表"先例 |
| 沙箱六步 | [`sandbox`](../../../crates/infer/src/sandbox.rs)、`__verify-algo` 子进程 + 10s 看门狗 | 制品准入门槛与宿主解耦校验 |
| 平台引擎先例 | `cv/platforms/rockchip`（feature `rga`） | SDK 吸收平台硬件的既有模式，RKNN 只是对称补齐 |
| 工程约束有界 | `validate_abi_header` 对宿主→插件结构允许 `size >= expected`（可加字段）；插件→宿主返回结构体宿主精确校验（不可加字段） | 输入结构可演进、输出结构必须冻结，这是既有非对称约定 |

### 2.2 当前缺口

| 缺口 | 现状事实 | 后果 |
| --- | --- | --- |
| RKNN 绑定重复 | 6 份 `rknn.rs`，合计 7,107 行；RK3568 vs RK3588 人脸版差异仅约 20 行；同一 RK3568 三份副本的 core_mask 处理已出现不一致（face 用 `0`+容忍 `-13`+校验返回值；fire/safetyhelmet 用 `1` 且忽略返回值） | 硬约束（DMA-BUF 映射权限、cache sync、输出释放）维修单点分散；副本漂移已实际发生 |
| `InitContext` 结构体字面量 | 11 处（9 个 `run_local`/probe 开发二进制 + 2 个真机 `hardware_infer_test.rs`）直接构造 | 加字段即源码破坏开发者第一入口与真机测试 |
| `AlgoError` 非 `non_exhaustive` | SDK 内零处使用 `#[non_exhaustive]` | 外部消费者穷举匹配时，新增错误变体成为破坏 |
| `min_adapter_version` 未强制 | manifest 已建模、已落库 `algorithm_versions`、已过 DTO，但无任何 semver 比较 | 宿主版本门槛通道空转 |
| `instance_negotiate` 空转 | 虚表槽位已存在；宏实现为 offered→accepted 透传；宿主无任何调用点 | 算法无法声明帧输入契约，不兼容帧只能在 `process` 内逐帧失败 |
| 路径依赖无版本约束 | 4 个 workspace 均为 `algo-sdk = { path = ... }` | `Cargo.lock` 不防源码破坏，破坏只能在编译时暴露 |
| 无 CI | 仓库无 CI 配置；门禁靠 Makefile `algo-check-all` 手工执行 | SDK 变更不保证全矩阵验证 |
| 平台 stub 与样板重复 | `locate_model_file` 3 变体、静态标签泄漏 helper ≥3 变体、`#[cfg]` 非目标平台 stub 每包一份、`explicit_fields`/`apply_env` 配置样板 7 份 | 用户被迫维护与算法逻辑无关的代码 |
| 文档漂移 | `algo-packages/README.md` 仅列 3 个 workspace（实际 4 个，`rk3588` 已在根 Makefile 门禁内）；`macos-arm64/` 为无源码无 workspace 的遗留目录；SDK `lib.rs` 的文档指引链接指向占位 URL（`https://github.com/`），未指向仓库内真实存在的 `crates/algo-sdk/GUIDE.md`；该 GUIDE.md 目前引导开发者“复用现有 `rknn.rs` 实现”，正是样板复制的来源 | 新算法开发者被误导；样板复制被文档化鼓励 |

---

## 3. 双契约模型

### 3.1 契约拓扑

```text
            算法包源码（8 个包，4 个 workspace）
                 │
                 │ ① 源码契约：SDK ↔ 包源码
                 │    path 依赖、同树编译；只加不破 + 门禁
                 ▼
            crates/algo-sdk ─────────────────────┐
                                                 │ 编译进制品
                                                 ▼
        lib{algorithm_id}.so/.dylib ── ② 运行时契约：C ABI v1（冻结）──► heimdall 宿主
```

关键区分：path 依赖使"SDK 一动、全部包**重编译**"在单仓库内不可避免（同源编译进每个 `.so`），但"全部包**改源码**"只发生在破坏性源码变更；已发布 `.so` 是预编译制品，SDK 源码变化不影响其加载。**耦合发生在构建/发版时刻，而不是部署时刻**——这是宿主先升级、算法包按需重打包的物理基础。

### 3.2 兼容方向与承诺

| 方向 | 场景 | 承诺 | 保障机制 |
| --- | --- | --- | --- |
| 向后（运行时） | 新宿主加载旧 `.so` | 必须成立 | v1 虚表冻结 + 可选符号探测降级 + 默认透传语义 |
| 向后（源码） | 新 SDK 编译旧包源码 | 必须成立（major 内） | 加法工具箱 + semver 门禁 |
| 向前（运行时） | 旧宿主加载新 `.so` | 能力优雅降级 | 可选能力探测；`min_adapter_version` 拒绝过新的包 |
| 前向（源码） | 旧 SDK 编译新包源码 | 不承诺 | 包在文档/manifest 声明所需 SDK 下限（发布场景） |

### 3.3 变更爆炸半径

| 变更类型 | 示例 | 需要改源码的包 | 已发布 `.so` 受影响 | 相关版本动作 |
| --- | --- | --- | --- | --- |
| 内部实现修复 | RKNN 绑定内部修复 | 0 | 否 | SDK patch |
| 纯加法 | 新模块、带默认实现的 trait 方法、新可选符号 | 0 | 否 | SDK minor |
| 用户构造的公开结构加字段 | `InitContext` 加字段 | 11 处构造点 | 否 | 需构造函数化后豁免 |
| 签名变更/删除/枚举加变体 | `process` 签名变化、`AlgoError` 加变体 | 相关包全部 | 否（旧 `.so` 仍可加载） | SDK major + 弃用过渡 |
| 虚表布局变化 | `AvAlgoAbi` 加槽位 | 全部包重编译且新旧制品互相拒载 | 是（破坏） | 禁止；只能新符号或 v2 双导出 |

---

## 4. 稳定性分区与加法工具箱

### 4.1 三层稳定性分区

| 分区 | 内容 | 演进规则 |
| --- | --- | --- |
| **L1 核心契约** | `plugin`（`AlgoPlugin`/`InitContext`）、`error`、`emitter`、`frame`、`c_abi`、`export_algo!` 调用形态 | 只能加法（默认方法、新函数、新宏 arm）；破坏仅限 major + 一版 `deprecated` 过渡 |
| **L2 平台能力** | `cv`（含未来 `platforms::rockchip::rknn`）、`postprocess`、`track`、`face`、`env`、`model`、`testing` | 加法为主；重命名走 `#[deprecated]` 别名托管一版 |
| **L3 内部实现** | 私有字段、原始 FFI 类型与探测逻辑 | `pub(crate)`/`doc(hidden)`，自由演进；严禁泄漏进公开签名 |

### 4.2 加法工具箱（机制映射）

```rust
// ① 新公开类型：non_exhaustive + Default/构造函数，字段可加
#[non_exhaustive]
pub struct RknnSessionOptions {
    pub core_mask: Option<c_int>,
    pub allow_cpu_fallback: bool,
}

impl Default for RknnSessionOptions {
    fn default() -> Self {
        Self { core_mask: None, allow_cpu_fallback: true }
    }
}

// ② 已公开并由用户构造的类型：补构造函数并迁移调用点，字段集从此冻结
impl<'a> InitContext<'a> {
    pub fn new(
        package_root: &'a Path,
        platform_id: &'a str,
        instance_id: &'a str,
        is_self_test: bool,
    ) -> Self {
        Self { package_root, platform_id, instance_id, is_self_test }
    }
}

// ③ trait 新能力：必须带默认实现，旧包零改动
pub trait AlgoPlugin: Sized + Send + 'static {
    /// 声明实例可接受的帧能力；None 表示不约束（保持既有行为）
    fn frame_requirements() -> Option<FrameRequirements> { None }
}

// ④ 弃用托管：重命名不删除，经一版过渡
impl RknnSession {
    #[deprecated(since = "1.1.0", note = "请使用 `RknnSession::open_or_fallback`")]
    pub fn open(package_root: &Path, model: &Path) -> Result<Self, AlgoError> {
        Self::open_or_fallback(package_root, model, RknnSessionOptions::default())
    }
}
```

以上代码为机制示意，最终签名以实施为准。可选符号模式（`av_algo_gallery_bulk` 的既有先例）不在 SDK 源码中体现，但属于同一工具箱：**新能力 = 新符号 + 探测 + 降级**。

### 4.3 红线

- 不修改 `AvAlgoAbi` 布局、不新增槽位；新能力只走新符号或 v2 双导出。
- 不给 `AlgoPlugin` 增加无默认实现的方法。
- 不给非 `non_exhaustive` 的公开枚举增加变体；不给用户构造的公开结构增加字段。
- 不删除公开符号（先 `#[deprecated]` 托管一版）。
- 不让原始 FFI 类型（`libloading`、裸指针、平台 C 结构）进入公开签名。

---

## 5. RKNN 平台层收敛

### 5.1 目标形态

新增 `crates/algo-sdk/src/platforms/rockchip/rknn/`（feature `rknn`，与既有 `rga` 平行），把 6 份重复绑定收敛为单份：

```rust
// 平台核掩码常量（包按平台显式传入，SDK 不猜平台）
pub const RKNN_NPU_CORE_0: c_int = 1;      // RK3568
pub const RKNN_NPU_CORE_0_1: c_int = 3;    // RK3576
pub const RKNN_NPU_CORE_0_1_2: c_int = 7;  // RK3588

pub struct RknnRuntime { /* libloading + 动态符号表 */ }
impl RknnRuntime {
    pub fn load(package_root: &Path) -> Result<Arc<Self>, AlgoError>;
}

pub struct RknnSession { /* RAII：ctx + runtime + 输出释放守卫 */ }
impl RknnSession {
    /// 优先物理 librknnrt；未检测到硬件时按 debug_cpu_fallback 语义保底
    pub fn open_or_fallback(
        package_root: &Path,
        model: &Path,
        options: RknnSessionOptions,
    ) -> Result<Self, AlgoError>;

    pub fn infer_dma_buf<R>(
        &mut self, fd: i32, size: usize,
        parse: impl FnOnce(&RknnOutputsGuard<'_>) -> R,
    ) -> Result<R, AlgoError>;

    pub fn infer_host<R>(
        &mut self, input: &[u8],
        parse: impl FnOnce(&RknnOutputsGuard<'_>) -> R,
    ) -> Result<R, AlgoError>;

    pub fn is_fallback(&self) -> bool;
}
```

以上为接口示意。设计约束：

- 张量输出类型只保留一处定义（`postprocess::yolov8_rknn` 现有 `RknnTensorOutput` 由 SDK 模块复用/re-export），禁止双写。
- `RknnSessionOptions` 为 `#[non_exhaustive]`，core_mask 由调用包按目标平台传入，环境变量覆盖策略（如 `RKNN_CORE_MASK`）收进 SDK 单点。
- feature 与目标平台组合：`rknn` 模块只在 feature 启用且目标为 Linux 时编译；macOS workspace 零影响。

### 5.2 绞杀者迁移

```text
① SDK 新增 rknn 模块（纯加法，不动任何既有类型与 ABI）
② 逐包迁移：一次一个包，替换本地 rknn.rs；其余代码零改动或最小改动
③ 全部迁移完成后删除 6 份本地副本；SDK 模块进入 L2 冻结，此后加法演进
```

任意时刻**所有包都能编译**：未迁移的包保持现状，SDK 从未被"修改"、只被"扩展"。每包迁移后必须通过该包 workspace 门禁与至少一次真机 `hardware_infer_test`。

### 5.3 必须随迁的硬约束

| 约束 | 说明 |
| --- | --- |
| DMA-BUF 映射权限 | `mmap` 输入 DMA-BUF 必须 `PROT_READ \| PROT_WRITE`；只读映射会在 `rknn_inputs_set` 写时触发内核缺页写保护 SIGSEGV |
| 映射生命周期 | `dma_mem_cache` 持有映射，禁止逐帧 mmap/munmap |
| core_mask 平台差异与已发生漂移 | 当前副本行为不一致：RK3568 face 用 `0`（AUTO）并容忍 `-13`、失败报错；RK3568 fire/safetyhelmet 用 `1` 且**忽略返回值**；RK3576 general 用 `3` 且忽略返回值；RK3576 face 用 `3`、RK3588 face 用 `7`（环境变量覆盖）且失败报错。迁移目标：取值由包传入，返回值检查与单核 `-13` 容忍策略由 SDK 单点统一 |
| 输出释放 | 全退出路径 `rknn_outputs_release`（RAII guard），不得随迁移丢失 |
| 回退语义 | 未检测到 `librknnrt` 时走 `debug_cpu_fallback_path`，日志与 `is_fallback()` 语义保持现状 |
| CMA 受限平台会话复用 | `SharedModels` 弱引用单例等包级策略暂不进 SDK 通用层，按包保留 |

---

## 6. 契约通道启用

### 6.1 帧能力协商（`instance_negotiate`）

虚表槽位已存在，**零 ABI 变更**，只做语义启用：

1. SDK：trait 增加带默认实现的 `frame_requirements() -> Option<FrameRequirements>`（默认 `None`）。
2. 宏：`None` 保持 offered→accepted 透传；`Some(req)` 求 offered ∩ required，为空返回 `AV_ERR_INCOMPATIBLE_FRAME` 并写入 `last_error`。
3. 宿主（第二阶段）：在 `create_instance` 成功后、首个 `process` 前调用 negotiate；流能力未知时传 `None` 维持透传，不阻塞主路径。

兼容性论证：旧包默认透传，新宿主行为不变；新包在旧宿主下不被校验但照常运行；两侧都不需要改动 ABI。

### 6.2 宿主适配版本门槛（`min_adapter_version`）

`AlgoManifest.min_adapter_version` 已建模落库但无比较。启用方案：

```rust
// 上传校验与冷启动注册共用的 manifest 校验函数内
if let Some(min) = manifest.min_adapter_version.as_deref().filter(|s| !s.is_empty()) {
    let host = semver::Version::parse(ADAPTER_VERSION)?;
    let required = semver::Version::parse(min)?;
    if host < required {
        return Err(/* 明确可读的"宿主版本过旧"错误 */);
    }
}
```

- 需为 `infer`（或校验所在层）增加 `semver` 依赖；适配版本来源初期取 app 版本（`CARGO_PKG_VERSION`），需要独立编号时再拆分常量。
- 行为：低于门槛的包在上传与冷启动注册两个入口被拒载，错误信息包含要求版本与实际版本。
- 语义边界：`min_adapter_version` 只表达"宿主下限"，不承担能力协商职责；能力协商由 6.1 负责。

---

## 7. 源码兼容修复与样板收敛

### 7.1 `InitContext` 构造函数化

先加 `InitContext::new(...)`，将 11 处构造点（`run_local`/probe 二进制与真机测试）迁移为构造函数调用；之后该类型字段集冻结，`#[non_exhaustive]` 在下一个 minor 视需要追加。这是"用户构造类型加字段"这一类破坏的根治手段，也是 negotiate/后续 ctx 扩展的前置条件。

### 7.2 `AlgoError` 标注 `non_exhaustive`

当前包内只有构造与 `?` 传播、无穷举匹配，是补属性的最低成本时点；补上后新增错误变体不再构成源码破坏。

### 7.3 样板收敛（L2 加性）

| 样板 | 现状 | 收敛目标 |
| --- | --- | --- |
| 模型定位 | `locate_model_file` 3 变体 | SDK `env` 提供 `locate_model_file(package_root, env, env_key, candidates)` |
| 静态标签 | `leak_custom_label`/`leak_label` ≥3 变体 | SDK 提供单点 helper |
| 平台 stub | 每包一份 `#[cfg(not(target_os = "..."))]` NotImplemented | 提供宏/模板统一表达 |
| 配置三阶覆盖 | 7 份 `explicit_fields` + `apply_env` | SDK `ConfigEnv` trait 或 derive，机制单点 |

### 7.4 脚手架生成器与 manifest 一致性

- `scripts/new_algo_package.sh`（或 `make algo-new ALGO_PLATFORM=... ALGO_ID=...`）：从模板生成 `Cargo.toml`（`cdylib + rlib`，lib 名 = `algorithm_id`）、`manifest.json`、`config.schema.json`、`Makefile`、`.env.example`、`src/{lib,plugin,config}.rs` 骨架与 README。
- SDK 测试助手：构造实例后调 `library_query`，断言元数据与 `manifest.json` 一致——把沙箱第 5 步前移到 `cargo test` 编辑期反馈。
- 顺带修正 `algo-packages/README.md` 的 workspace 清单（4 个）与 `macos-arm64/` 遗留目录的说明（处置另行确认，不在本设计内删除）。

---

## 8. 兼容性门禁

| 门禁 | 工具/命令 | 触发时机 |
| --- | --- | --- |
| SDK 公开面破坏检查 | `cargo semver-checks check-release --manifest-path crates/algo-sdk/Cargo.toml --baseline-rev <tag>`（需先建基线 tag） | SDK 每次变更 |
| 特征组合编译 | `cargo check -p algo-sdk --no-default-features` / `--features rga` / `--features rknn` | SDK 每次变更 |
| 全矩阵编译与测试 | 根 workspace + `make algo-check-all` / `algo-test-all`（4 个平台） | SDK 每次变更 |
| canary 契约测试 | `algo-sdk/tests/plugin_lifecycle.rs`、双侧 `c_abi_layout_tests.rs` 保留在 SDK | SDK 每次变更 |
| 旧制品实证 | 旧 tag 构建 `.so` → 当前宿主 `heimdall __verify-algo <包目录>` | ABI 相关变更 |

门禁应固化为 Makefile 目标（如 `make compat`），在具备 CI 后升级为流水线步骤。

---

## 9. 实施阶段

### Phase 0：契约固化

- 在 [算法 SDK 与沙箱](../backend/algo-sdk-guidelines.md) 增加"兼容性与稳定性分区"小节（引用本设计）。
- 补 `#[non_exhaustive]`（`AlgoError` 与新公开类型）、`InitContext::new` 并迁移 11 处调用点。
- 建 SDK 基线 tag；新增兼容性门禁脚本/目标。

### Phase 1：RKNN 平台层收敛

- 新增 `platforms::rockchip::rknn`（feature `rknn`，含 5.3 全部硬约束）。
- 逐包迁移：先 rk3576（双包）→ rk3568（三包）→ rk3588（单包，含 probe 二进制）。
- 迁移完成后删除 6 份本地副本，更新包内 README/依赖。

### Phase 2：契约通道启用

- 帧能力协商：SDK trait + 宏语义启用；宿主 `create_instance` 接线（能力未知时透传）。
- `min_adapter_version` 强制：统一 manifest 校验函数 + 上传/注册入口接线。

### Phase 3：样板收敛与脚手架

- `locate_model_file`、静态标签、平台 stub、`ConfigEnv` 收敛。
- `new_algo_package` 脚手架与 manifest 一致性测试助手。
- 用脚手架生成一个示例包并跑通六步沙箱，作为开发者体验验收。

### Phase 4：文档同步

- `algo-sdk-guidelines.md` 补 RKNN 模块、协商契约、版本门槛的调用约束。
- 修正 `algo-packages/README.md` 与索引漂移；修正 SDK `lib.rs` 中指向占位 URL 的 GUIDE.md 链接，并把 `crates/algo-sdk/GUIDE.md` 中“复用现有 `rknn.rs`”的引导更新为“使用 SDK `platforms::rockchip::rknn`”。

---

## 10. 验收标准

1. `cargo-semver-checks` 对 SDK 零破坏报告（基线 tag 建立后）。
2. 根 workspace 与 4 个算法 workspace 门禁全绿；11 处 `InitContext` 构造点（含真机测试）不因 SDK 变更而破坏。
3. 旧 tag 构建的 `.so` 能被当前宿主 `__verify-algo` 通过（运行时契约未破的实证）。
4. `rknn.rs` 收敛为 SDK 单份（预估 1,500–2,000 行，feature 门控），包侧删除 6 份、约 7,100 行副本；各包 `plugin.rs` 只保留算法逻辑与配置；core_mask 行为统一为 SDK 单点策略（调用后校验返回值、单核容忍 `-13`），不再出现忽略返回值的副本。
5. 默认 `frame_requirements() = None` 的包在启用协商的宿主下行为逐字节不变；声明需求的包收到不兼容能力时创建失败且错误可读。
6. `min_adapter_version` 高于宿主版本的包在上传与冷启动注册处被拒载，错误信息含双方版本。
7. 新脚手架生成的包在 30 分钟内通过六步沙箱。
8. 文档索引、包 README 与规范同步，无失效链接。

---

## 11. 决策与实现落点

### 11.1 已定决策

- **不引入类继承/第二 ABI 层**：trait + 宏 + 冻结 ABI 已是"实现即合规"的完整机制；扩展只走默认方法、新符号、新模块。
- **平台模块 feature 门控**：`rknn` 与 `rga` 平行；macOS workspace 零影响。
- **加法演进 + 绞杀者迁移**：SDK 从未被修改、只被扩展；包从未被强制升级、只被逐步采纳。
- **协商默认关闭**：`frame_requirements() = None` 保持既有透传语义；宿主在能力未知时不校验。
- **`min_adapter_version` 只表达宿主下限**：不承担能力协商；适配版本来源初期取 app 版本。
- **core_mask 策略统一**：SDK 统一“调用后校验返回值、单核平台容忍 `-13`”的策略；fire/safetyhelmet 现状“忽略返回值”属于副本漂移，迁移时按统一策略收紧，属于有意行为变更，需在迁移包的变更说明中标注。
- **版本政策靠门禁而非 Cargo**：in-tree path 依赖不引入逐包版本约束；破坏性检查由 semver 门禁承担。

### 11.2 实现落点

| 层 | 落点 | 关键契约 |
| --- | --- | --- |
| SDK | `platforms/rockchip/rknn/`（新模块）；`plugin.rs`（`frame_requirements`）；`error.rs`（`non_exhaustive`）；`macros.rs`（negotiate 交集语义）；`env.rs`（模型定位）；`testing.rs`（manifest 一致性助手） | 全部为加法；`AvAlgoAbi` 布局与 `AV_ALGO_API_VERSION` 不动 |
| infer | `package.rs`（`create_instance` 调用 negotiate）；manifest 校验路径（`min_adapter_version` 比较）；新增 `semver` 依赖 | 能力未知时透传；门槛拒绝错误可读 |
| 算法包 | 6 个包迁移 rknn 绑定；11 处 `InitContext::new`；配置/模型定位样板替换 | 行为与 `debug_cpu_fallback_path` 语义不变 |
| 脚本/Makefile | `scripts/new_algo_package.sh`、`make compat` 门禁目标、基线 tag 流程 | 门禁可重复执行 |
| docs | `algo-sdk-guidelines.md` 兼容性小节；`algo-packages/README.md` 修正；SDK `GUIDE.md` 与 `lib.rs` 文档链接修正 | 未实现内容不得写成现有能力 |

### 11.3 已知取舍

- SDK 体积增加：`rknn` 代码进入 SDK 源码树，由 feature 门控避免无关平台编译。
- path 依赖下重编译不可避免：本设计只承诺"零源码改动"，不承诺"零重编译"。
- `#[non_exhaustive]` 对既有穷举匹配者是一次性源码破坏：当前包内无穷举匹配，是成本最低的时点。
- 协商的真实性受限：宿主在实例创建时可能尚不知道流能力，启用条件为"能力已知才校验"；这不是完整帧契约，只比现状前进一大步。
- 6 份副本删除后，历史包的独立可编译性（脱离 monorepo）下降，符合"样板收敛"的取舍。

### 11.4 仍待处理

- `macos-arm64/` 遗留目录的处置确认（另行处理，不在本设计内删除）。
- SDK 是否发布到 registry 面向外部算法开发者；若发布，需补 semver 发布流程与版本下限声明。
- `rknn` feature 与 `testing-hardware` 等测试特性的组合矩阵细化。
- RKNN 模块是否需要 `doc(hidden)` 的裸 FFI 子模块分层（供极端场景直接调用，同时保持公开面收窄）。
