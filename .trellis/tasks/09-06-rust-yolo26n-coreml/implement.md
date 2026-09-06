# 执行计划: 基于 algo-sdk 重构 yolo26n CoreML 算法包

## 执行步骤清单

### 1. 目录准备与资源迁移
- [x] 在 `algo-packages/macos/arm64/yolo26n/` 建立标准算法包目录。
- [x] 从 `/Users/niko/dev/go/argus/algo-packages/macos/arm64/yolo26n/` 迁移核心资源：
  - `model/yolo26n.mlpackage`
  - `testimage.jpg`
  - `manifest.json`
  - `config.schema.json`
- [x] 检查并确保 `manifest.json` 中 `platform_id`、`algorithm_id` 与架构定义精准合规。

### 2. Cargo 工程初始化与 Workspace 集成
- [x] 创建 `algo-packages/macos/arm64/yolo26n/Cargo.toml`，配置为 `cdylib` + `rlib`。
- [x] 在仓库根目录 `Cargo.toml` 的 `workspace.members` 中挂载 `"algo-packages/macos/arm64/yolo26n"`。
- [x] 引入 `algo-sdk = { path = "../../../../crates/algo-sdk" }`。

### 3. 配置与 COCO 掩码过滤实现 (`src/config.rs`)
- [x] 实现 `InstanceConfig` 反序列化（置信度阈值、IOU 阈值、目标类别列表、自定义告警标签）。
- [x] 实现 80 个标准 COCO 类别常量与 $O(1)$ 位掩码 `ClassMask`。
- [x] 编写配置单元测试验证默认值、有效性与掩码行为。

### 4. CoreML 纯 Rust 硬件推理执行器 (`src/coreml.rs`)
- [x] 编写轻量纯 Rust Objective-C Runtime 绑定，链接 `CoreML.framework` 与 `Foundation.framework`。
- [x] 实现 `MLModel` 动态编译（`.mlpackage` -> 编译 URL）与加载（启用 ANE + GPU 计算单元）。
- [x] 实现 `predict_pixelbuffer` 零拷贝前向推理：包装 `CVPixelBuffer` -> `MLFeatureValue` -> 特征字典 -> 获取 `var_911` 输出 MultiArray。
- [x] 编写针对真实 `.mlpackage` 的模型加载测试。

### 5. 后处理与黑边坐标还原 (`src/postprocess.rs`)
- [x] 解析 `[1, 300, 6]` 形状的张量数据。
- [x] 实现置信度阈值过滤与类别掩码过滤。
- [x] 对接 `algo_sdk::math::unmap_box`，根据 Letterbox 布局精准去除四周黑边填充并还原为原图归一化坐标。
- [x] 单元测试验证已知张量数据的过滤与反算精度。

### 6. 算法插件接入与 C ABI 导出 (`src/plugin.rs` & `src/lib.rs`)
- [x] 实现 `algo_sdk::plugin::AlgoPlugin` 与 `AlgoInstance`。
- [x] 串联 `AppleCvEngine.letterbox` 硬件预处理、`CoreMlRunner` 推理与 `ResultEmitter` 结果发射。
- [x] 使用 `export_algo!` 宏导出标准 C ABI 虚拟方法表与 Panic 隔离屏障。

### 7. 编译生成与沙箱集成校验
- [x] 运行 `cargo build -p general-detection --release` 生成 `libgeneral_detection.dylib` 并放入 `lib/` 目录。
- [x] 更新 `crates/infer/src/package.rs` 与 `crates/app/src/reconcile.rs`，确保 `algo-packages/macos/arm64` 自动穿透扫描。
- [x] 编写集成测试，调用 `infer::AlgoSandbox::validate_package` 对算法包进行完整的 7 步物理子进程沙箱验证。
- [x] 运行全工作区测试门禁：`cargo fmt`、`cargo clippy` 与 `cargo test --workspace`。

### 8. 交付生命周期、README 文档与压力测试
- [x] 提供标准 Makefile 指令：`build`、`test`、`run`、`benchmark`、`stress`、`package`、`clean`。
- [x] 修正 CoreML `MLComputeUnitsAll = 2` 枚举，启用 ANE 硬件加速；缓存选择器与 Accelerate NEON SIMD 批量转换，达到 ~200+ FPS。
- [x] 实现美化 JSON 输出与 `.env` 调试覆盖机制。
- [x] 编写规范 `README.md` 并包含在打包归档中，回填 `.trellis/spec/backend/algo-sdk-guidelines.md`。
