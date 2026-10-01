# Implementation Plan: PyTorch-like Composable Edge AI Algorithm Ecosystem

> **文档状态**：2026-10-01 按实际落地情况校准。
> Phase 1-6 已交付并验证；Phase 7 未启动，已交接至独立任务（见 §4）。

## 1. 执行阶段规划 (Phased Milestones)

```text
[x] Phase 1: 硬件预处理 Transforms 算子链落地 (HwLetterbox)
[x] Phase 2: 通用 YOLO 检测器骨架 GenericYoloDetector & YoloDecoder 策略模式实现
[x] Phase 3: algo-sdk 核心单测与完整 C ABI export_algo! 生命周期集成测试
[x] Phase 4: 脚手架与开发文档同步更新 (crates/algo-sdk/GUIDE.md, spec)
[x] Phase 5: 全平台质量门禁验证 (四平台 workspace 及根 workspace 门禁测试全绿通过)
[x] Phase 6: safetyhelmet_detection 基于 GenericYoloDetector 极简重构 (M2 落地)
[~] Phase 7: 目标平台硬件驱动级适配 (Ascend DVPP / NVIDIA TensorRT SPI) — 移出本任务，见 §4
[~] Phase 8: 级联多模型与 CvBuffer 池化 SDK 抽象 (M3) — 移出本任务，见 §4
```

---

## 2. 已交付步骤 (Delivered Steps)

### Step 1: 硬件预处理 Transforms 抽象 [已完成]
- **文件**: `crates/algo-sdk/src/cv/transforms.rs`（75 行）
- **交付**:
  - `Transform` trait（`apply(&SafeFrame) -> (CvBuffer, PreprocessMode)`）；
  - `HwLetterbox`（`new` / `with_fill`），经 `cv::active_engine()` 分派到实例作用域引擎；
  - `algo_sdk::cv` 与 `prelude` 导出 `Transform`, `HwLetterbox`。
- **未交付**: `HwCrop`、`HwAffineWarp5Points`（见 §4 交接项）。
- **验证**: `cv::transforms::tests::test_hw_letterbox_basic` 通过。

### Step 2: 架构分层解耦与通用 YOLO 检测器骨架 [已完成]
- **文件**: `crates/algo-sdk/src/models/yolo.rs`（566 行）、`src/runtime/mod.rs`（129 行）、`src/runtime/platforms/rockchip.rs`
- **交付**:
  - **模块正交归位**：`models` 提升为顶层模块；`rknn.rs` 下沉为 `runtime/platforms/rockchip.rs`（`pub use ... as rknn` 保留别名）；`cv` 回归纯 2D 预处理层；
  - **统一异构运行时抽象**：`InferenceOutput`（`MultiBranch` / `SingleFloat`）与 `NpuSession` trait；`RuntimeSession` 自动路由 + 无驱动 `CpuFallbackSession` 闭环；
  - **解码策略与参数收敛**：`YoloSpec`、`YoloDecoder` + `YoloDecodeContext`、`Yolov8SpecDecoder<S>`；
  - **安全生命周期**：`custom_label: Option<String>` 托管配置覆盖标签，消除 `'static` 内存泄漏；
  - **默认依赖收缩**：`Cargo.toml` 保持 `default = []`，避免非 Rockchip 平台被强拉 `libloading`。
- **验证**: `cargo test -p algo-sdk --lib`（116 passed），含 `test_generic_yolo_detector_lifecycle`、`test_custom_decoder_strategy`。

### Step 3: algo-sdk 单元测试与 C ABI 集成测试 [已完成]
- **文件**: `crates/algo-sdk/tests/generic_yolo_lifecycle.rs`
- **交付**: `test_generic_yolo_c_abi_export_lifecycle` 覆盖 `av_algo_get_abi` → `library_open` → `library_query` → `instance_create` → `instance_process` → `instance_update_config` → `instance_destroy` → `library_close`。
- **验证**: `cargo test --test generic_yolo_lifecycle` 通过。

### Step 4: 开发文档同步与规范沉淀 [已完成]
- **文件**: `crates/algo-sdk/GUIDE.md`（501 行）、`.trellis/spec/algo-sdk/backend/algo-sdk-guidelines.md`
- **交付**: GUIDE 新增范式 A（声明式检测器 + `YoloDecoder` 策略 + `USE_SCORE_SUM` / `CLS_IS_LOGITS` 判定表）与范式 B（底层手工编排）；spec 权威定义表登记 `models` / `cv::transforms` / `runtime`。
- **验证**: 文档中的 API 引用（`prelude` 导出、`algo_sdk::*` 路径）与源码一一对应。

### Step 5: 全平台质量门禁闭环 [已完成开发机门禁]
- **已验证**: `cargo fmt --all -- --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test --workspace`（根工作区 700+ 测试）、四大算法平台 workspace（`macos`, `rk3568`, `rk3576`, `rk3588`）格式化 + Clippy + 测试、`git diff --check`。
- **待硬件实测**: 目标板（RK3568/RK3576）真实硬件驱动与物理 DMA-BUF 零拷贝直通。
- **说明**: 本仓无 CI workflow 配置，「CI 门禁全绿」实指上述本地命令在开发者机器上全部通过。

### Step 6: safetyhelmet_detection 基于 GenericYoloDetector 重构 (M2) [已完成]
- **文件**:
  - `algo-packages/rknn/rk3568/safetyhelmet_detection/src/plugin.rs` → **24 行**（原 189 行）
  - `src/config.rs` → 别名复用 SDK `StandardYoloConfig` 为 `InstanceConfig`
  - `src/postprocess.rs` → 委托 `Yolov8SpecDecoder::<SafetyHelmetSpec>`
  - `src/bin/run_local.rs` → 解除平台绑定，支持 CPU 回退模拟与 `--benchmark`
  - `crates/algo-sdk`: `RuntimeSession::is_fallback()`
- **代码缩减口径**（可复现）:

  | 口径 | 数值 | 复核命令 |
  |---|---|---|
  | `plugin.rs` 文件总行数 | 24 | `wc -l .../src/plugin.rs` |
  | `impl YoloSpec for SafetyHelmetSpec` 块（含文档注释与空行） | 11 | `awk '/^impl YoloSpec for SafetyHelmetSpec \{/,/^\}/' .../src/plugin.rs \| wc -l` |
  | 同上，去注释与空行 | 9 | 减去 2 行 `///` 注释（L16、L18） |

  > 历史记录说明：`prd.md` 早期验收口径曾记录为「实测 21 行」，该数字与当前源码不符且无法复现（块总计 11 行）。
  > 现以本节表格为准——**不修改 `prd.md` 中的历史改写记录**，以保留审计线索，本表为权威复核口径。
  >
  > 「~15 行交付」是**声明式交付的心智模型口径**，指开发者需要手写的声明式代码量级；
  > 与「文件总行数」不是同一度量，两者不可互相替代，引用时须标注口径。
- **验证**:
  - `cargo fmt --manifest-path algo-packages/rknn/rk3568/Cargo.toml --all -- --check`：通过；
  - `cargo clippy --manifest-path algo-packages/rknn/rk3568/Cargo.toml -p safetyhelmet-detection-rk3568-rknn --all-targets -- -D warnings`：0 告警；
  - `cargo test --manifest-path algo-packages/rknn/rk3568/Cargo.toml -p safetyhelmet-detection-rk3568-rknn`：全部通过（包内 9 个用例：`config.rs` 4 + `postprocess.rs` 5）；
  - `make test`：通过；
  - `safetyhelmet_detection_run_local --benchmark`：CPU 回退基准完成（~400 ABI FPS）。
- **遗留**: `postprocess.rs`（261 行）已不在生产 C ABI 路径上，仅服务 `run_local.rs` 的分阶段计时与自身单测；其中 `parse_and_unmap_output` 保留「解码错误降级为本帧无目标」的吞错语义，与 SDK 路径的向上抛错不一致。

### Step 7: 评审回归修复（logits 语义）[已完成]
- **新增能力**: `YoloSpec::CLS_IS_LOGITS` + `ClassActivation`，解码器按 `sigmoid` 还原置信度并将阈值换算到 logit 空间。
- **修复的缺陷**:
  - 阈值量化由四舍五入改为向下取整——对整数原始值，`v > t` 严格等价于 `v > floor(t)`，舍入会抬高阈值并静默丢弃边界网格（同时影响既有 9-tensor 概率路径）；
  - `Yolov8RknnConfig::from_spec` 成为唯一构造入口（字段私有化），在类型层面拒绝「9-tensor + logits」组合；
  - 包内 `postprocess` 不再以 `unwrap_or_default()` 吞掉解码错误（指 `try_parse_and_unmap_output`）。
- **验证**: 新增判别性用例（原始值 0 在两种语义下结论相反），SDK 与包作用域测试全绿；`--target aarch64-unknown-linux-gnu` 交叉检查补充覆盖了本机不编译的 Linux 专有分支。

---

## 3. 文档校准步骤 [已完成 2026-10-01]

### Step 8: as-built 校准与漂移记录 [已完成]
- **文件**: `design.md`（全文重写为 as-built 版）、`implement.md`（本文）、`crates/algo-sdk/src/cv/transforms.rs`（模块文档）
- **改动**:
  - `design.md` 删除与实现不符的示例代码（`crate::cv::models::yolo`、`RknnSession::infer_with_dma_buf`、`Yolov8StandardDecoder<const...>`、`GenericDetector<S, D>` 等），改为可直接对应源码的片段；新增 §2.3（`CLS_IS_LOGITS`）、§3.3（级联与池化未交付清单）、§7（14 项漂移对照表）；
  - `implement.md` 修正 Step 2 路径（`src/runtime/*` 而非 `src/rknn.rs`）、Step 3 路径（`src/models/yolo.rs` 而非 `src/cv/models/yolo.rs`）、Step 6 中 `postprocess.rs` 的错误描述，并补录 logits 步骤为 Step 7；
  - `cv/transforms.rs` 模块文档移除对 `Crop` 的隐含承诺，明确 ROI/仿射的当前归属。
- **验证**: 文档中每个代码片段与路径均可 `grep` 到对应源码。

---

## 4. 移出本任务的交接项 (Handoff)

以下两项经评估不属于「文档收尾」范围，独立建任务承接，**不挂为本任务的 subtask**（归档会移动父目录，子任务链接将悬空）。

### H1 — 级联多模型与 CvBuffer 池化 SDK 抽象（原 M3 / Phase 8）
- **背景**: 级联能力已在 `rk3568`/`rk3588` 的 `face_recognition`（各 818 行）中以包内手工编排形态生产运行，含「一份 RGA 输出 → 两个 RKNN session」的 DMA-BUF 复用验证；缺的是 SDK 契约抽象与自动化测试。
- **范围**:
  1. `CascadePipeline` 契约设计（或明确决定不抽象，转而在 GUIDE 中固化手工编排范式）；
  2. `CvBuffer` 池化能力上提至 SDK 公共层（当前池为 `RgaCvEngine` 私有）；
  3. 设备侧中间流转的正确性测试：验证 「检测 → 抠图 → 二级推理」链路无 CPU 像素拷贝、无内存泄漏、无池耗尽；
  4. 复核 `MAX_CACHED_POOLS` 从 16 上调至 64 后，`rk3568`/`rk3588` face_recognition 注释与「档位只增不减」设计论证的前提是否仍成立。
- **注意**: `CvBufferKind` 为 `pub(crate)` 密封，改动需评估 ABI 与跨平台影响。

### H2 — Ascend DVPP / NVIDIA TensorRT 平台适配（原 Phase 7）
- **背景**: `CvBufferKind::AscendDeviceMemory` 与 `AV_OPAQUE_ASCEND_DEVICE_MEMORY` 已就位，但无 `AscendVpcEngine`、无 `CudaCvEngine`、无 `CudaDeviceMemory` 变体。
- **前置**: 需要真机（昇腾 Atlas / NVIDIA 卡）才能启动。
- **关联**: 分核策略与宿主协同见 `10-01-npu-core-allocation`。

### H3 — CPU fallback 静默降级的可观测性（风险登记，未立项）
- **问题**: `RuntimeSession::open` → `RknnSession::open_or_fallback` 在 `librknnrt` **加载失败**时静默降级为 `CpuFallbackSession` 并返回固定模拟检测框；`is_fallback()` 全仓唯一消费者是 `run_local.rs`，宿主 `crates/infer` 无任何校验；`RknnSessionOptions` 无「禁止降级」开关。
- **风险**: 部署错误（`.so` 缺失 / ABI 不匹配 / `package_root` 拼错）会使算法包「看起来正常运行」并稳定输出固定假框，违反 AGENTS.md「严禁伪装为硬件加速」与「故障不得静默」原则。
- **建议**: 跨 `algo-sdk` 与 `infer` 两层改造，建议单独立项并定级 P1。

---

## 5. 复用清单 (Reusable Artifacts)

后续同类「声明式算法包」交付可直接复用：

- `YoloSpec` + `GenericYoloDetector<S>` + `YoloDecoder` 策略骨架；
- `algo_config!` 三级优先级配置宏（宿主 > `.env` > 默认）；
- `HwLetterbox` + `active_engine()` 实例级引擎作用域；
- `FailureTracker` 硬件段失败计数（阈值 30 帧 ≈ 1s @30fps，`AtomicU64`）；
- `LocalPluginRunner` + `scripts/algo-new.sh` 脚手架；
- `run_local.rs` 分阶段计时基准（preprocess / inference / postprocess / end-to-end / ABI）。
