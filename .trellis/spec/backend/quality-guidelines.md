# 后端质量规范

> 提交前必须通过的检查、代码风格约束、测试要求。

> ⚠️ **状态：立项约定（尚未经代码验证）**
> 命令与配置文件在首批代码落地后需实测并回填，删除本提示。

---

## 提交前必跑

**硬性规则：每次提交代码前，必须先使用格式化工具自动格式化代码！**

```bash
# 1. 自动代码格式化（每次提交必跑）
cargo fmt --all
clang-format -i $(find native -name '*.c' -o -name '*.h')

# 2. 校验格式化状态、静态分析与测试
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
```

**`-D warnings` 是硬要求**：clippy 警告等同错误。允许积累警告的项目最终会把警告全部忽略掉。

`--all-features` 只在有对应 SDK 的机器上能过。开发机上至少要保证默认 feature（`backend-cpu`）全绿：

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test --workspace
```

---

## 配置文件

| 文件 | 内容要点 |
|------|---------|
| `rustfmt.toml` | 保持默认为主，只固定 `edition = "2021"`。不要发明团队私有风格 |
| `clippy.toml` | 配置 `cognitive-complexity-threshold` 等阈值 |
| `.clang-format` | 基于某个成熟预设（如 `BasedOnStyle: Google`），不手写全套规则 |

**原则**：格式化配置越接近生态默认越好。私有风格会让每个新人和每个 AI 会话都产生噪音 diff。

---

## 必开的 lint

在 workspace 根 `Cargo.toml` 里统一配置：

```toml
[workspace.lints.rust]
unsafe_op_in_unsafe_fn = "deny"
missing_debug_implementations = "warn"

[workspace.lints.clippy]
undocumented_unsafe_blocks = "deny"   # 见 ffi-guidelines.md
unwrap_used = "warn"                  # 库 crate 里不该有裸 unwrap
todo = "deny"
dbg_macro = "deny"
print_stdout = "deny"                 # 一律用 tracing
```

`unwrap_used` 设为 `warn` 而非 `deny`：测试代码和 `app` 启动路径需要它，用 `#[allow]` 局部放开并写明理由。

---

## 测试要求

| 变更类型 | 测试要求 |
|---------|---------|
| 纯逻辑（NMS、坐标变换、时间窗口） | **必须有单元测试**，含边界值 |
| 数据库 repository | 集成测试，用临时 SQLite 文件（不用内存库 —— 与生产的 WAL 行为不一致） |
| API handler | 用 `axum::body` + `tower::ServiceExt::oneshot` 测，不起真实端口 |
| FFI 绑定 | 至少有 POD 布局的 `size_of`/`align_of` 断言 |
| 平台后端 | `#[ignore]` 标注，真机手动跑 |
| Bug 修复 | **先写能复现的失败测试，再改代码** |

**硬性规则**：`cargo test` 在开发机（无 NPU、无摄像头）上必须全绿。任何依赖硬件的测试都要 `#[ignore]`。

---

## 坐标与几何计算必须有测试

预处理的缩放/padding 与后处理的坐标还原是本项目最容易出 bug 的地方，且 bug 表现为"框偏一点点"，肉眼难发现、日志看不出。

**规则**：任何涉及坐标变换的函数必须有单元测试，覆盖：

- 等比缩放 + letterbox padding 的往返一致性
- 非整除的分辨率（如 1920×1080 → 640×640）
- 边界框贴边的情况

---

## 代码审查关注点

按优先级：

1. **资源泄漏** —— FFI 句柄、fd、buffer 池租约是否在所有路径（含错误路径）上归还
2. **并发模型** —— 阻塞调用有没有跑进 async；通道是不是有界；有没有持锁跨 `.await`
3. **每帧路径开销** —— 有没有隐式分配、有没有 `format!`、有没有 `info!` 级日志
4. **平台泄漏** —— `#[cfg]` 有没有跑到 `infer` / `media` 之外
5. **unsafe** —— 有没有 SAFETY 注释，理由是否站得住
6. **错误处理** —— 有没有该降级的地方 panic 了；有没有重复记日志

前三项是 Argus 特有的，通用 Rust 审查清单不会覆盖。

---

## 依赖管理

- 依赖版本统一在 workspace 根的 `[workspace.dependencies]` 声明，子 crate 用 `dep.workspace = true`。
- **新增依赖需要说明理由**。边缘设备关心二进制体积和编译时间，一个便利函数不值得引入一个大 crate。
- `Cargo.lock` 入 git（这是应用不是库）。

---

## 禁止事项

- ❌ 提交前未执行代码格式化（Rust 必须跑 `cargo fmt`，C 必须跑 `clang-format`）
- ❌ 提交带 clippy 警告的代码
- ❌ 提交 `dbg!` / `println!` / `todo!()`
- ❌ 依赖真实硬件才能通过的非 `#[ignore]` 测试
- ❌ 修改已有测试来让新代码通过（除非测试本身写错了，且要在 PR 里说明）
- ❌ 只测 happy path 的坐标变换函数

---

## 场景：系统存储状态端点的运行时装配与 DTO 契约

### 1. Scope / Trigger

- 触发条件：新增或修改 `/api/v1/system/storage/*`，或在 `AppState` 中引入存储运行时句柄。
- 适用边界：`app` 负责按运行配置装配 `StorageCleaner`，`api` 负责调用它并返回共享 DTO；不在 handler 中临时创建清理器。

### 2. Signatures

- `AppState::with_storage_cleaner(evidence_dir) -> AppState`
- `GET /api/v1/system/storage/status -> ApiResponse<types::StorageStatus>`
- `StorageCleaner::get_storage_status() -> Result<types::StorageStatus, PipelineError>`

### 3. Contracts

- 应用启动时必须使用与 `PipelineManager` 相同的 `storage.evidence_dir` 装配 `StorageCleaner`。
- `StorageStatus`、`StorageConfig`、`EvictionReport` 及系统设置网络/时间 DTO 使用 `#[serde(rename_all = "camelCase")]`；例如 `total_gb` 在线上必须为 `totalGb`。
- 成功响应保持 `{ code: 0, message: "success", data: T, timestamp }`；磁盘状态读取失败返回系统错误，不伪造成功数据。

### 4. Validation & Error Matrix

| 条件 | 行为 |
| --- | --- |
| 生产状态未装配 `StorageCleaner` | HTTP 500，错误码 `51300`，记录可定位日志 |
| `statvfs` 读取失败 | HTTP 500，错误码 `51300` |
| cleaner 已装配且文件系统可读 | HTTP 200，`data.totalGb` 等 camelCase 字段存在 |

### 5. Good/Base/Bad Cases

- Good：启动阶段注入配置目录，handler 只读取句柄并返回共享 DTO。
- Base：测试使用临时目录和 `AppState::with_storage_cleaner`，不依赖真实证据文件。
- Bad：仅在 `AppState` 增加 `Option<StorageCleaner>`，却不在生产入口赋值；或直接序列化 Rust snake_case 字段。

### 6. Tests Required

- 使用 `axum::body` 与 `tower::ServiceExt::oneshot` 请求 `/storage/status`。
- 断言 HTTP 200、`code == 0`、`data.totalGb > 0`，并覆盖错误状态的映射。
- app 启动装配必须传入配置的 evidence 目录，避免状态查询和实际写盘路径漂移。

### 7. Wrong vs Correct

#### Wrong

```rust
let state = AppState::new_with_limit(db, pipeline, max_upload_size);
// storage_cleaner 仍为 None，/storage/status 返回 500
```

#### Correct

```rust
let state = AppState::new_with_limit(db, pipeline, max_upload_size)
    .with_storage_cleaner(cfg.storage.evidence_dir.clone());
```

---

## 场景：跨平台系统指标采样与时间单位

### 1. Scope / Trigger

- 触发条件：新增或修改 `/api/v1/system/overview` 的 CPU 使用率、运行时间或平台系统 API。
- 适用边界：平台采样实现留在 `api::system_info`，handler 只负责异步调度和 DTO 映射；前端 `uptimeSeconds` 只按秒格式化。

### 2. Signatures

- `read_system_info() -> SystemInfoRaw`
- `read_cpu_usage() -> CpuInfo`
- `read_uptime() -> f64`，返回运行时长秒数，不是 Unix 时间戳

### 3. Contracts

- `cpuUsagePercent` 必须是 `[0, 100]` 的百分比，而不是 load average 或累计 tick。
- macOS 使用 `PROCESSOR_CPU_LOAD_INFO` flavor `2` 读取 user/system/idle/nice tick；flavor `1` 是 `PROCESSOR_BASIC_INFO`，禁止混用。
- macOS `kern.boottime` 必须按完整 `libc::timeval` 读取，再用 `now - bootTime` 计算秒数；sysctl 失败时返回 `0`，不得把当前 Unix 时间戳当作运行时间。

### 4. Validation & Error Matrix

| 条件 | 行为 |
| --- | --- |
| CPU tick 两次采样有效 | 按 delta 计算并 clamp 到 `0..=100` |
| Mach 采样失败或 tick 布局不完整 | 返回 `0` CPU 使用率，不阻塞系统概览接口 |
| `kern.boottime` 读取成功 | 返回非负运行秒数 |
| `kern.boottime` 读取失败 | 返回 `0` 运行秒数，不返回 epoch 秒数 |

### 5. Good/Base/Bad Cases

- Good：使用平台原生累计 tick，两次采样取差值；使用完整 `timeval` 读取启动秒数。
- Base：CPU 采样失败时降级为 0，但保持接口可用并保证数值范围正确。
- Bad：把 `PROCESSOR_BASIC_INFO` 当 CPU load；只给 `kern.boottime` 分配 8 字节；直接把 Unix 当前时间作为 uptime 返回。

### 6. Tests Required

- macOS 测试断言 CPU tick 两次采样单调增加，且使用率为有限的 `0..=100` 数值。
- macOS 测试断言 `kern.boottime` 成功读取，运行时间小于当前 Unix 秒数。
- 跨平台测试断言 `SystemInfoRaw` 的 CPU 使用率和运行时间非负且有限。

### 7. Wrong vs Correct

#### Wrong

```rust
const PROCESSOR_CPU_LOAD_INFO: processor_flavor_t = 1;
let mut boot_secs: i64 = 0;
let size = size_of::<i64>();
```

#### Correct

```rust
const PROCESSOR_CPU_LOAD_INFO: processor_flavor_t = 2;
let mut boot_time: libc::timeval = unsafe { std::mem::zeroed() };
let mut size = size_of::<libc::timeval>();
```

---

## 场景：macOS 网卡枚举与只读网络配置

### 1. Scope / Trigger

- 触发条件：新增或修改 `/api/v1/system/network/interfaces`，或新增 macOS 网络平台适配。
- 适用边界：macOS 使用 `ifconfig -a` 获取接口事实状态，使用 `networksetup` 补充硬件类型和 Network Service 配置；当前版本只读，不复用 Linux 的 `nmcli`/`networkctl` 修改逻辑。

### 2. Signatures

- `NetworkService::list_interfaces() -> Result<Vec<NetworkInterface>, ApiError>`
- `list_interfaces_macos() -> Result<Vec<NetworkInterface>, ApiError>`
- `GET /api/v1/system/network/interfaces -> ApiResponse<NetworkInterfacesResponse>`

### 3. Contracts

- macOS 至少返回 `lo0` 以及 `ifconfig -a` 能枚举到的其它接口；不能因没有 `nmcli`/`networkctl` 而返回空数组。
- `ifconfig` 负责 `name`、flags、MAC 和 IPv4；`networksetup` 可用时补充 Wi-Fi/以太网类型、DHCP/静态方式、网关和 DNS。
- macOS DTO 的 `manager` 使用现有 `unmanaged` 枚举，`canModifyIp`、`canSetDhcp`、`canSetStatic` 必须为 `false`，并给出只读原因。
- 外部命令必须经 `spawn_blocking` 执行；`networksetup` 补充命令失败时保留 `ifconfig` 的基础网卡数据，不伪造空列表。

### 4. Validation & Error Matrix

| 条件 | 行为 |
| --- | --- |
| `ifconfig -a` 成功 | 返回接口列表，至少包含 loopback |
| `networksetup` 成功 | 补充硬件端口、服务、DHCP、网关和 DNS 信息 |
| `networksetup` 不可用或单项查询失败 | 保留 MAC/状态/IPv4 等基础信息，配置字段按缺失处理 |
| `ifconfig -a` 执行失败 | 返回网络服务错误，不返回误导性的成功空列表 |
| macOS 尝试 PUT 修改网卡 | 返回只读错误，不执行 `nmcli` 或 `networkctl` |

### 5. Good/Base/Bad Cases

- Good：以 `ifconfig` 的当前状态为准，以 `networksetup` 的服务配置补充 DHCP、Router 和 DNS。
- Base：虚拟接口没有硬件端口映射时标记为 `virtual`；没有 IPv4 时 `ipv4` 为 `null`。
- Bad：macOS 直接检测 Linux 管理器并在未找到时返回 `Vec::new()`；或把 `ifconfig` 的当前地址误标成可在线修改的 NetworkManager 配置。

### 6. Tests Required

- 纯解析测试覆盖 `ifconfig` flags、MAC、IPv4/netmask、active/inactive 状态。
- 纯解析测试覆盖 `networksetup` hardware port 与 service-to-device 映射。
- macOS 主机测试调用系统枚举并断言至少存在 `lo0`。
- Linux 现有 `nmcli`/`networkd` 路径必须继续通过 workspace 编译和测试。

### 7. Wrong vs Correct

#### Wrong

```rust
let manager = detect_network_manager();
match manager {
    NetworkManager::Networkmanager => list_interfaces_nm().await,
    NetworkManager::SystemdNetworkd => list_interfaces_networkd().await,
    _ => Ok(Vec::new()),
}
```

#### Correct

```rust
#[cfg(target_os = "macos")]
{
    list_interfaces_macos().await
}
```

---

## 待验证事项

- [ ] CI 环境能否覆盖三个平台的交叉编译检查（至少 `cargo check`）
- [ ] 是否引入 `cargo-deny` 做许可证与安全公告检查
- [ ] 是否需要基准测试（`criterion`）跟踪每帧路径的性能回归
