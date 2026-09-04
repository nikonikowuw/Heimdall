# C ABI 算法包宿主与沙箱加载体系 执行落地计划 (Subtask 1)

## 执行步骤与阶段划分

```
Step 1: 资产与目录准备 ──> Step 2: C ABI 结构体映射与断言测试 ──> Step 3: 动态库加载与虚表封装
                                                                           │
Step 6: 端到端集成与门禁 ──<── Step 5: 真实 CoreML 推理自测 ──<── Step 4: 七步沙箱校验器实现
```

---

### Step 1: 算法包资产与目录拓扑就绪
- [x] 在仓库根目录建立 `algo-packages/macos-arm64/general_detection/` 标准拓扑；
- [x] 同步编译生成的 `libgeneral_detection.dylib`、`manifest.json`、`config.schema.json`、`testimage.jpg` 及 `model/yolo26n.mlpackage`；
- [x] 验证目录结构与文件权限完整性。

### Step 2: C ABI 1:1 结构体映射与内存对齐断言
- [x] 在 `crates/infer/src/c_abi/types.rs` 实现全部核心 C 结构体（`av_frame_desc`、`av_algo_abi`、`av_algo_instance_args` 等）；
- [x] 严格对齐 `#[repr(C)]`，按 64 位平台 8 字节对齐规范排布；
- [x] 编写测试 `tests/c_abi_layout_tests.rs`，断言 `size_of` 和关键 `offset_of!` 与 `sdk/include/argus/` 完全一致；
- [x] 运行 `cargo test -p infer --test c_abi_layout_tests` 验证 100% 通过。

### Step 3: 基于 `libloading` 的动态库加载器与 RAII 抽象
- [x] 在 `crates/infer/Cargo.toml` 中引入 `libloading = "0.8"`、`image = "0.25"`（用于测试图解码）；
- [x] 实现 `crates/infer/src/c_abi/loader.rs`：安全打开动态库，寻址并提取 `_av_algo_get_abi` 符号，完成虚表校验；
- [x] 封装 RAII 生命周期：`LoadedLib`、`RawAlgoLibrary`、`AlgoInstance`，确保 `Drop` 自动回收资源。

### Step 4: 七步沙箱校验器与物理子进程隔离工作流 (`AlgoSandbox`)
- [x] 实现 `crates/infer/src/sandbox.rs`：
  1. 路径与文件结构安全性校验（防路径穿越）；
  2. SHA256 完整性校验；
  3. `manifest.json` 平台标签校验（非当前平台拒绝加载，支持短标签别名兼容）；
  4. `config.schema.json` 格式校验；
  5. 物理子进程派生与超时看门狗调度（`Command::new(current_exe).arg("__verify-algo")`）；
  6. 崩溃捕获：子进程如果出现段错误（`SIGSEGV`）或超时，主进程能安全捕获并返回友好错误；
  7. 注册到 `AlgoRegistry`。

### Step 5: 真实测试图 Core ML 前向自检推理 (Self-Test Worker)
- [x] 实现自测试子进程执行体：
  - 加载 `testimage.jpg`，转为 BGRA / RGBA 像素内存并构造 `av_frame_desc`；
  - 在 macOS 上使用 `NativePixelBuffer` 渲染原生 NV12 `CVPixelBuffer`；
  - 以 `AV_INSTANCE_INSTALL_SELF_TEST` 模式创建实例，执行 `instance_process`；
  - 在 `on_result` 回调中解析 JSON 结果，断言返回的 detection 框准确有效（置信度 $> 0.25$）；
- [x] 编写自测集成测试用例，在 macOS ARM64 环境下验证一次性跑通子进程物理隔离全链路。

### Step 6: 接入 `InferenceBackend` 与代码质量全量门禁
- [x] 让 `AlgoInstance` 实现 `InferenceBackend` trait，使 `Pipeline` 能直接使用动态算法包；
- [x] 实现全局可用算法包注册表 `AlgoRegistry`；
- [x] 更新 `crates/infer/src/lib.rs` 导出；
- [x] 运行门禁检查：
  ```bash
  cargo fmt --all -- --check
  cargo clippy --all-targets -- -D warnings
  cargo test -p infer
  cargo test --workspace
  ```

---

## 验收核对表 (Checklist)

- [x] 确定 C ABI 规范契约（兼容原系统 `sdk/include/argus/algo.h`）。
- [x] C ABI 结构体 `size_of` 与内存偏移测试 100% 通过。
- [x] 成功将 `algo-packages/macos-arm64/general_detection` 载入 Rust 宿主。
- [x] 沙箱成功对 `testimage.jpg` 执行 Core ML 真实推理并获得目标框。
- [x] Clippy 无警告，测试无报错，Unsafe 严格包含 `// SAFETY:` 注释。
