# Implementation Plan: PyTorch-like Composable Edge AI Algorithm Ecosystem

## 1. 执行阶段规划 (Phased Milestones)

```text
[x] Phase 1: 硬件预处理 Transforms 算子链落地 (HwLetterbox)
[x] Phase 2: 通用 YOLO 检测器骨架 GenericYoloDetector & YoloDecoder 策略模式实现
[x] Phase 3: algo-sdk 核心单测与完整 C ABI export_algo! 生命周期集成测试
[x] Phase 4: 脚手架与开发文档同步更新 (crates/algo-sdk/GUIDE.md, spec)
[x] Phase 5: 全平台 Workspace CI 质量门禁验证 (四平台 workspace 及根 workspace 门禁测试全绿通过)
[x] Phase 6: safetyhelmet_detection 基于 GenericYoloDetector 极简重构与本地 Runner 跨平台适配验证 (M2 落地)
[ ] Phase 7: 目标平台硬件驱动级适配 (Ascend DVPP / NVIDIA TensorRT SPI 真实设备对接)
```

---

## 2. 详细执行步骤 (Detailed Steps)

### Step 1: 硬件预处理 Transforms 抽象 [已完成]
- **文件**: `crates/algo-sdk/src/cv/transforms.rs`, `crates/algo-sdk/src/cv/mod.rs`
- **改动**:
  - 定义 `Transform` trait；
  - 实现 `HwLetterbox`，内部调用 `default_engine().letterbox(...)`；
  - 在 `algo_sdk::cv` 中导出 `Transform`, `HwLetterbox`。
- **验证**: `cargo test -p algo-sdk --lib cv::transforms` 测试通过。

### Step 2: 架构分层解耦与通用 YOLO 检测器骨架 [已完成]
- **文件**: `crates/algo-sdk/src/models/mod.rs`, `crates/algo-sdk/src/models/yolo.rs`, `crates/algo-sdk/src/runtime/mod.rs`, `crates/algo-sdk/src/runtime/platforms/rockchip.rs`
- **改动**:
  - **模块正交归位**：将 `models` 提升为顶层模块，将 `rknn.rs` 下沉为 `runtime/platforms/rockchip.rs`，`cv` 回归纯粹的 2D 视觉预处理层；
  - **统一异构运行时抽象**：定义 `InferenceOutput` 与 `NpuSession` trait，上层检测器不硬编码具体芯片；提供 `RuntimeSession` 自动路由与无驱动 CPU Fallback 闭环；
  - **解码策略与参数收敛**：定义 `YoloSpec`、`YoloDecoder` 策略 trait 与 `Yolov8SpecDecoder`；在 `yolo.rs` 内部收敛 2 参数私有 `decode_single_float`，消除冗余 Context 抽象；
  - **安全生命周期**：`GenericDetector` 使用 `Option<String>` 托管配置覆盖标签，彻底消除 `'static` 内存泄漏；
  - **默认依赖收缩**：恢复 `Cargo.toml` 中 `default = []`，避免非 Rockchip 平台（如 macOS）被强拉 `libloading`。
- **验证**: `cargo test -p algo-sdk --lib models::yolo` 及 `cargo test -p algo-sdk --all-features` 测试全部通过。

### Step 3: algo-sdk 单元测试与 C ABI 集成测试 [已完成]
- **文件**: `crates/algo-sdk/src/cv/models/yolo.rs`, `crates/algo-sdk/tests/generic_yolo_lifecycle.rs`
- **改动**:
  - 单元测试：`test_generic_yolo_detector_lifecycle`、`test_custom_decoder_strategy`；
  - C ABI 全流程测试：`test_generic_yolo_c_abi_export_lifecycle`（覆盖 `export_algo!`、`av_algo_get_abi`、`library_open`、`library_query`、`instance_create`、`instance_process`、`instance_update_config`、`instance_destroy`、`library_close`）。
- **验证**: `cargo test --test generic_yolo_lifecycle` 测试通过。

### Step 4: 开发文档同步与规范沉淀 [已完成]
- **文件**: `crates/algo-sdk/GUIDE.md`, `.trellis/spec/algo-sdk/backend/algo-sdk-guidelines.md`
- **改动**:
  - `GUIDE.md` 增加范式 A（Composable 声明式检测器与 `YoloDecoder` 策略）与范式 B（底层手工编排）；
  - 更新工程规范索引，建立统一的模型与变换层文档。
- **验证**: 文档与源码完全对应，无虚构 API。

### Step 5: 全局质量门禁闭环 [已完成开发机门禁]
- **已验证**:
  - `cargo fmt --all -- --check`：100% 格式对齐通过；
  - `cargo clippy --all-targets -- -D warnings`：0 warnings；
  - `cargo test --workspace`：根工作区 700+ 测试全绿；
  - 四大算法平台工作区（`macos`, `rk3568`, `rk3576`, `rk3588`）`cargo test` 与 `cargo clippy` 100% 通过；
  - `git diff --check`：通过。
- **待后续硬件实测**:
  - 目标板（RK3568/RK3576）真实硬件驱动与物理 DMA-BUF 零拷贝直通。

### Step 6: safetyhelmet_detection 基于 GenericYoloDetector 重构 (M2 达成) [已完成]
- **文件**:
  - `algo-packages/rknn/rk3568/safetyhelmet_detection/src/plugin.rs`
  - `algo-packages/rknn/rk3568/safetyhelmet_detection/src/config.rs`
  - `algo-packages/rknn/rk3568/safetyhelmet_detection/src/postprocess.rs`
  - `algo-packages/rknn/rk3568/safetyhelmet_detection/src/bin/run_local.rs`
- **改动**:
  - `plugin.rs`：声明 `SafetyHelmetSpec` 实现 `YoloSpec`（640x384, 2 类标签, `USE_SCORE_SUM = false` 对应官方 6-tensor 结构, `CLS_IS_LOGITS = true` 对应 sigmoid 在图外），定义 `SafetyHelmetDetector = GenericYoloDetector<SafetyHelmetSpec>`，核心源码收敛至 21 行（`impl YoloSpec for SafetyHelmetSpec` 块实测 21 行，代码缩减 ~90%）；
  - `config.rs`：别名并复用 SDK `StandardYoloConfig` 为 `InstanceConfig`，保留完整配置反序列化与三级优先级测试；
  - `postprocess.rs`：委托至 `Yolov8SpecDecoder::<SafetyHelmetSpec>`，消除冗余的 INT8 与 FP32 解析代码；
  - `run_local.rs`：解除平台绑定，通过 `detector.transform.apply` 与 `detector.session.infer_with` 支持开发机 CPU 回退模拟运行与 `--benchmark` 压测；
  - `algo-sdk`：为 `RuntimeSession` 补充 `is_fallback()`，在 CPU Fallback 模式下无缝驱动推理闭环。
- **验证**（包作用域，非 workspace 粒度）：
  - `cargo fmt --manifest-path algo-packages/rknn/rk3568/Cargo.toml --all -- --check`：通过；
  - `cargo clippy --manifest-path algo-packages/rknn/rk3568/Cargo.toml -p safetyhelmet-detection-rk3568-rknn --all-targets -- -D warnings`：0 告警；
  - `cargo test --manifest-path algo-packages/rknn/rk3568/Cargo.toml -p safetyhelmet-detection-rk3568-rknn`：全部通过；
  - `make test`：通过；
  - `safetyhelmet_detection_run_local --benchmark`：本地 CPU 回退基准测试顺利完成（~400 ABI FPS）。
- **注意（本轮修复的回归）**:
  - 包作用域构建（`make host` / `make test` / `cargo build -p`）曾因 `run_local` 无条件引用 `MockFrameBuilder::from_image_hardware`（需 `algo-sdk/testing-hardware`）而 exit 101；仅 workspace 粒度靠特性统一侥幸通过。现由 `build_mock_frame()` 按 `testing-hardware` 特性分派，两种特性组合均可编译运行；
  - `FailureTracker` 已从 `cv/platforms/rockchip/diagnostic.rs` 上移至跨平台的 `cv/diagnostic/mod.rs`，并重新接入 `GenericDetector`；计数边界为硬件段（预处理 + 推理），结果发射失败不计入。

### Step 7: 评审回归修复（logits 语义）[已完成]
- **新增能力**: `YoloSpec::CLS_IS_LOGITS` + `ClassActivation`，解码器按 `sigmoid` 还原置信度并将阈值换算到 logit 空间
- **修复的缺陷**:
  - 阈值量化由四舍五入改为向下取整——对整数原始值，`v > t` 严格等价于 `v > floor(t)`，舍入会抬高阈值并静默丢弃边界网格（同时影响既有 9-tensor 概率路径）；
  - `Yolov8RknnConfig::from_spec` 成为唯一构造入口（字段私有化），在类型层面拒绝“9-tensor + logits”组合；
  - 包内 `postprocess` 不再以 `unwrap_or_default()` 吞掉解码错误。
- **验证**: 新增判别性用例（原始值 0 在两种语义下结论相反），SDK 与包作用域测试全绿；`--target aarch64-unknown-linux-gnu` 交叉检查补充覆盖了本机不编译的 Linux 专有分支。
