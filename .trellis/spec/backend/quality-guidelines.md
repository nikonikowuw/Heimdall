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

## 待验证事项

- [ ] CI 环境能否覆盖三个平台的交叉编译检查（至少 `cargo check`）
- [ ] 是否引入 `cargo-deny` 做许可证与安全公告检查
- [ ] 是否需要基准测试（`criterion`）跟踪每帧路径的性能回归
