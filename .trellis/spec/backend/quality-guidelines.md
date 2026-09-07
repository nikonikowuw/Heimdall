# 后端质量规范 (Quality Guidelines)

> 提交前门禁、代码风格约束、测试策略与核心系统契约。

---

## 1. 提交前必跑门禁 (Pre-Commit Gates)

代码提交前必须在本地依次执行并通过以下门禁：

```bash
# 1. 自动代码格式化
cargo fmt --all
if [ -d native ]; then
  find native -type f \( -name '*.c' -o -name '*.h' \) -exec clang-format -i {} +
fi

# 2. 格式化校验、Clippy 静态检查与单元测试
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
```

- **硬性约束**：`-D warnings` 必须全绿，禁止积累任何 Clippy 警告；
- **开发机全绿规则**：`cargo test --workspace` 在无 NPU、无摄像头的开发机必须全绿。任何依赖真实硬件或专属 SDK 的测试必须标注 `#[ignore]` 并加 `#[cfg(feature = "...")]`。

---

## 2. 必开的 Lint 规范 (`Cargo.toml`)

在 workspace 根目录统一配置：

```toml
[workspace.lints.rust]
unsafe_op_in_unsafe_fn = "deny"
missing_debug_implementations = "warn"

[workspace.lints.clippy]
undocumented_unsafe_blocks = "deny"   # 每一个 unsafe 块必须写 // SAFETY: 说明
todo = "deny"
dbg_macro = "deny"
print_stdout = "deny"                 # 严禁使用 println!，一律使用 tracing
unwrap_used = "warn"                  # 库 crate 严禁裸 unwrap，仅测试代码与启动入口可局部 allow
```

---

## 3. 测试分类与要求

| 变更类型 | 测试要求 | 核心断言点 |
|---------|---------|-----------|
| **纯计算逻辑**（NMS、坐标映射、时间计算） | 必须有单元测试 | 覆盖边界值、贴边检测框、非等比分辨率 |
| **数据库 Repository** | 集成测试 | 使用独立临时 SQLite 文件测试（不使用内存库，以匹配 WAL 行为） |
| **API Handler** | 契约测试 | 使用 `axum::body` + `tower::ServiceExt::oneshot` 验证，不起物理端口 |
| **FFI / C ABI 绑定** | 布局一致性测试 | 严格断言 POD 结构体的 `size_of` 与 `offset_of!` |
| **硬件平台后端** | 硬件集成测试 | 必须标注 `#[ignore]`，仅在真机环境下手动执行 |
| **Bug 修复** | 回归测试 | **必须遵循 TDD：先编写能稳定复现 Bug 的失败测试，再修改业务代码** |

### 坐标与几何计算测试铁律
预处理缩放（Letterbox/Padding）与后处理坐标还原最容易发生“轻微偏框”。所有涉及几何变换的代码必须覆盖：
1. 原图与目标图等比、非等比（如 1080p -> 640x640）缩放的往返一致性；
2. 极端贴边（`x=0.0`, `y=0.0`, `w=1.0`, `h=1.0`）及微小目标的有效保留与黑边精准剔除。

---

## 4. 代码审查核心关注点 (Priority Checklist)

1. **资源与句柄生命周期**：FFI 裸指针、DMA-BUF fd、显存池租约在所有路径（含 `?` 错误提前返回与 Panic）均有 RAII `Drop` 回收；
2. **并发与异步模型**：Tokio Worker 内严禁执行 FFI、阻塞系统调用或大于 1ms 的 CPU 计算；严禁持锁跨 `.await`；
3. **逐帧热路径开销**：常驻推理流线上严禁任何堆分配（`Vec::new` / `format!`）、严禁 CPU 像素拷贝、严禁 `info!` 日志；
4. **平台差异隔离**：平台 `#[cfg]` 绝不逃逸至 `infer` / `media` 之外；
5. **Unsafe 安全屏障**：每个 `unsafe` 块均有确切且符合逻辑的 `// SAFETY:` 注释。

---

## 5. 核心跨平台系统契约与避坑指南

### 5.1 系统存储清理器装配契约
- 应用启动时必须通过 `AppState::with_storage_cleaner(evidence_dir)` 注入与 `PipelineManager` 相同的证据目录，禁止在 Handler 中临时构造清理器；
- DTO 字段采用 camelCase（`totalGb`）；状态读取失败返回系统错误（`code: 51300`），严禁伪造默认成功数据。

### 5.2 macOS 平台系统指标采样契约
- **CPU 使用率采样**：macOS 必须调用 `host_processor_info` 且 flavor 固定为 `PROCESSOR_CPU_LOAD_INFO` (数值 `2`)，**严禁使用 flavor `1` (`PROCESSOR_BASIC_INFO`)**；两次采样计算 delta 比率并限制在 `0..=100`；
- **系统启动时间**：通过 `sysctl` 读取 `kern.boottime` 时，缓冲区必须分配完整的 `libc::timeval`（16 字节），通过 `now - tv_sec` 计算秒数，**严禁仅分配 8 字节导致内存越界破坏栈**；
- **运行时间单位**：`uptimeSeconds` 返回以秒为单位的浮点或整数，**绝不能返回当前 Unix 时间戳**。

### 5.3 macOS 网络网卡枚举契约
- macOS 下网络配置为只读模式：`canModifyIp`、`canSetDhcp` 必须为 `false`；
- 必须基于 `ifconfig -a` 解析物理/虚拟网卡（至少返回 `lo0`），结合 `networksetup` 补充 Wi-Fi/以太网与 DNS，外部命令必须在 `spawn_blocking` 中执行，严禁因无 Linux `nmcli` 而返回空列表。

---

## 6. 禁止事项 (Iron Rules)

- ❌ 提交未执行 `cargo fmt` 或带 Clippy 警告的代码（`-D warnings`）
- ❌ 提交包含 `dbg!`、`println!`、`todo!()` 的代码
- ❌ 依赖物理硬件设备却未标注 `#[ignore]` 的测试
- ❌ 修改现有测试的断言以迎合错误实现
- ❌ 在代码中引入不必要的大型第三方依赖库（严格评估二进制体积与编译开销）
- ❌ 将 `Cargo.lock` 从版本控制中排除（必须入 git）
