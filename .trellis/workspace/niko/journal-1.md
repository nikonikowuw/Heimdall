# Journal - niko (Part 1)

> AI development session journal
> Started: 2026-10-01

---



## Session 1: 统一前端抽屉组件与两轴 code review 修复

**Date**: 2026-10-01
**Task**: 统一前端抽屉组件与两轴 code review 修复
**Package**: web
**Branch**: `dev`

### Summary

新增共享 Drawer 组合层并迁移 7 个业务抽屉（实体材质、small/compact/medium/wide 尺寸、统一 header/唯一滚动主体/可选固定工具栏与底栏）。两轴 code review 后修复 10 项发现：移除从未渲染的 Drawer ariaLabel 死参数（ModalOverlay 名称契约改为「至少提供其一」类型联合）、新增 titleTooltip 回填摄像头长设备名提示、.drawer-footer > :only-child 接管单一操作项对齐、清除抽屉主体内与实体面板重复的 backdrop-blur/frosted-glass、以 CSS 规则级测试守护固定头尾与唯一滚动区，并修正 AccountPanelDrawer.test.tsx 断言他组件内部类名的问题。Web 门禁全绿：479 tests / lint --max-warnings=0 / typecheck / check:cycles / build。亮暗主题与窄视口视觉检查由开发者人工完成。

### Main Changes

(Add details)

### Git Commits

| Hash | Message |
|------|---------|
| `7d32088` | (see git log) |

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 2: 归档 Bootstrap Guidelines

**Date**: 2026-10-01
**Task**: 归档 Bootstrap Guidelines
**Package**: infer
**Branch**: `dev`

### Summary

按用户要求归档 00-bootstrap-guidelines；抽屉实现提交 7d32088 已由先前 journal 记录，本条不重复引用。NPU 多核分配任务继续处于 planning，模板哈希清单的未提交改动予以保留。

### Main Changes

(Add details)

### Git Commits

(No commits - planning session)

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 3: 归档事件驱动 NVR 录像任务

**Date**: 2026-10-01
**Task**: 归档事件驱动 NVR 录像任务
**Package**: media
**Branch**: `dev`

### Summary

核验 09-30-event-nvr-recording 交付现状，补齐上下文配置并验证 media/pipeline/db/api/web 全套门禁与测试，成功将已完成任务归档至 archive/2026-10/。

### Main Changes

(Add details)

### Git Commits

(No commits - planning session)

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 4: Numeric input blur validation

**Date**: 2026-10-02
**Task**: Numeric input blur validation
**Package**: algo-sdk
**Branch**: `dev`

### Summary

实现共享 NumericField 与 numericDraft，迁移录像、GB28181、网络和存储数值输入，增加失焦/提交兜底及交互测试，并固化前端 spec。Web 自动门禁通过（75 个测试文件、563 项测试；format、lint、typecheck、cycles、build 均通过）。真实浏览器下的三语、键盘焦点及设置流程人工核对未执行。

### Main Changes

(Add details)

### Git Commits

| Hash | Message |
|------|---------|
| `acee8197cf1a31a5895344ec3c476417c071c75c` | (see git log) |
| `4598408e0a6554b4155aeb80e488a6db1e078201` | (see git log) |

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 5: NPU fallback observability hard gate
<!-- trellis-session: v=2 fp=17d76d0b866ac037 -->

**Date**: 2026-10-02
**Task**: NPU fallback observability hard gate
**Branch**: `dev`

### Summary

Implemented explicit hardware fallback policy, platform_id-based resolution, and an unbypassable install self-test gate. Added actionable ModelLoad errors, three-state availability reporting, explicit local-tool Allow overrides, regression tests, and updated specs.

### Main Changes

- RKNN sessions now distinguish hardware-required failure from simulated fallback; self-test cannot be overridden.
- Development tools explicitly opt into simulated fallback without relying on .env.
- Documented policy precedence, 512-byte last_error ordering, and validation contract.

### Git Commits

| Hash | Message |
|------|---------|
| `1cae773` | feat(algo-sdk): enforce hardware-required fallback policy |
| `8d2599b` | docs(spec): document hardware fallback contract |
| `e57ced1` | chore(task): finalize NPU fallback observability artifacts |

### Testing

- [OK] Workspace: 977 tests passed; algo-sdk with rknn feature: 174 passed.
- [OK] macOS, RK3568, RK3576, RK3588 workspaces: 37 / 88 / 34 / 67 tests passed.
- [OK] fmt, workspace/platform clippy, Linux cross-target checks, mutation gate check, and git diff --check passed.

### Status

[OK] **Completed**

### Next Steps

- Continue the separate NPU core-allocation task from its planning phase.

## [2026-10-03] NPU 跨层协议基线、有界清理与路线 A/B 验证 (10-03-npu-abi-protocol-baseline)

### Summary

实现了 NPU 多核分配与卡亲和架构的基线协议（Subtask 1），包括 Worker 线程有界退出与隔离管理（QuarantineSupervisor）、C ABI Placement 扩展（`AvAlgoPlacementExtensionV1` 与双侧 POD 对齐断言）、Wire 放置配置安全剥离保护（`__heimdall_placement`），以及独立的板端验证探针工具（`tools/probe_rknn_dup_context/`）。

### Main Changes

- **Worker 有界清理与隔离**：重构 `crates/infer/src/worker.rs`，在 Worker 线程内部显式释放 runtime 与 backend 后再发送退出完成信号；对停机超时及通道断开引入 `QuarantineSupervisor`，移交后台非阻塞 Reaper 回收，杜绝无界 join() 与误判完成。
- **C ABI Placement 可选扩展**：在 `crates/infer/src/c_abi/types.rs` 与 `crates/algo-sdk/src/c_abi.rs` 中定义 `AvAlgoPlacementExtensionV1` 及 POD 结构（`AvAlgoPlacementCapsPod`、`AvAlgoInstanceReceiptPod`、`AvAlgoCleanupReceiptPod`），维持基础 `AvAlgoAbi` 96 字节不变，双侧通过静态与单元测试严格断言尺寸、对齐与字段偏移。
- **Wire 协议兼容性与配置安全剥离**：在 `crates/algo-sdk/src/macros.rs` 中增加 `deserialize_config_stripping_placement`，在插件配置反序列化前单次剥离 `__heimdall_placement`，保护标记有 `#[serde(deny_unknown_fields)]` 的业务插件。
- **T42 板端独立探针**：编写无宿主依赖的最小 C 探针工具 `tools/probe_rknn_dup_context/`，提供完整 Makefile、多线程并发压测、路线 A（独占移交）与路线 B（加锁 dup）双路线测试及内存查询。
- **自动化测试与回归矩阵**：落地 T01（停机超时隔离）、T02（创建失败隔离）、T03（通道断开隔离）、T13（Wire 兼容性与配置剥离）、T14（回执防御性解析）、T41（自检硬件硬门）全项测试。
- **规范同步**：更新 `.trellis/spec/algo-sdk/backend/algo-sdk-guidelines.md` 与 `.trellis/spec/infer/backend/inference-backends.md`。

### Testing

- [OK] `cargo fmt --all -- --check` 通过。
- [OK] `cargo clippy --all-targets -- -D warnings` 全工作区通过。
- [OK] `cargo nextest run --workspace` 1001 项测试全绿。
- [OK] `algo-packages/macos` 37 项测试全绿。
- [OK] `algo-packages/rknn/{rk3568, rk3576, rk3588}` cargo clippy 全部无告警通过。
- [OK] `probe_rknn_dup_context` 本地编译与静态自测运行通过。

### Status

[OK] **Completed**

### Next Steps

- 推进子任务 2 (`10-03-npu-host-placement-ledger`)：构建宿主 NPU 拓扑探测、核心分配账本与世代屏障。
