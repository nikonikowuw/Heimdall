# 基于 algo-sdk 重构 macOS arm64 yolo26n CoreML 算法包

## Goal

将 legacy C++ yolo26n 算法包使用 Rust 与 `crates/algo-sdk` 重构为纯 Rust cdylib 动态库插件，落地于 `algo-packages/macos/arm64/yolo26n`，支持 Apple ANE (Apple Neural Engine) / GPU 统一显存零拷贝硬件推理，并通过 `crates/infer` 7 步沙箱自检。

## Requirements

1. **工程架构与规范落地**：
   - 算法包放置于 `algo-packages/macos/arm64/yolo26n/`，作为独立 `cdylib` crate 并纳入根 `Cargo.toml` workspace。
   - 依赖 `crates/algo-sdk`，利用 `export_algo!` 宏导出标准 C ABI 虚拟方法表 (`av_algo_get_abi`)，包含 Panic 隔离屏障与双向 ABI 校验。
   - 保留原算法包元数据文件：`manifest.json`、`config.schema.json`、`model/yolo26n.mlpackage` 和 `testimage.jpg`。
2. **Apple 统一显存零拷贝硬件加速预处理**：
   - 依托 `algo-sdk` 的 `AppleCvEngine`，将输入的 `CVPixelBuffer` NV12 格式直接通过 Apple Accelerate vImage 转换为 `640 × 384` BGRA `CVPixelBuffer`，四周填充 `[114, 114, 114]` Letterbox 黑边。
   - 全链路保持在 Apple Unified Memory 中流转，严禁发生任何 CPU 像素拷贝或 Host Readback。
3. **CoreML 纯 Rust 安全推理执行器**：
   - 基于 Apple Objective-C Runtime（`CoreML.framework` + `Foundation.framework`）实现纯 Rust CoreML 运行时封装。
   - 支持动态定位并编译 `.mlpackage` 为 `.mlmodelc` 缓存，或直接加载已编译模型。
   - 将预处理输出的 `CVPixelBuffer` 作为 `image` 特征喂入模型，提取输出张量 `var_911`（`[1, 300, 6]`）。
4. **高效向量化后处理与结果发射**：
   - 解析 300 个候选目标，提取 `[x1, y1, x2, y2, score, class_id]`。
   - 依据 `config.schema.json` 规范，实现 `confidence_threshold`（默认 0.45）与 `target_classes`（COCO 80 类别掩码）过滤。
   - 使用 `algo-sdk::math::unmap_box` 准确剔除 Letterbox 黑边并还原为归一化原图坐标 `[0.0, 1.0]`。
   - 使用 `ResultEmitter::emit_detections` 流式零分配发射告警事件与全景大图抓拍请求。
5. **系统兼容与沙箱自检闭环**：
   - 编译输出 `algo-packages/macos/arm64/yolo26n/lib/libgeneral_detection.dylib`。
   - 支持 `crates/infer` 中的 7 步物理子进程沙箱安全校验与前向推理自检。
   - 兼容宿主 `reconcile_and_seed_algorithms` 算法目录自愈扫描。

## Acceptance Criteria

- [x] `algo-packages/macos/arm64/yolo26n` 编译生成 `libgeneral_detection.dylib` 无 warnings。
- [x] 算法插件通过 `algo-sdk` 的 `export_algo!` 导出完整的 11 个 C ABI 虚函数，ABI 布局与 `crates/infer` 完全一致。
- [x] 算法包能够通过 `infer::AlgoSandbox::validate_package(&pkg_dir, true)` 全部 7 步物理沙箱测试（含静态元数据校验、架构检查、符号导出、前向推理自测与报告生成）。
- [x] 单测覆盖：配置解析、CoreML 模型加载与推理、后处理过滤与坐标反算。
- [x] 全工作区集成测试通过：`cargo test --workspace` 全绿。

## Constraints

- 仅在 macOS Apple Silicon (arm64) 环境下启用硬件 CoreML 推理与 Accelerate 预处理。
- 严禁向 FFI 宿主逃逸 Rust Panic。
- 常驻推理主路径严格维持纯显存零拷贝，无 CPU 像素内存分配。
