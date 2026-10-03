# SDK placement 注入与 RKNN 共享权重运行时实施计划

> 所属任务：`10-03-algo-sdk-rknn-shared-weights`（阶段 C）。

## 1. 实施检查清单

### Phase 1: 自动化测试用例先行 (T10, T28, T31–T38, T44)
- [x] 在 `crates/algo-sdk/tests/shared_weights_tests.rs`（或内部单元测试）中建立测试：
  - `test_init_context_with_placement_builder`: 验证 `InitContext` 链式构造与 `target_core_mask()` 提取；
  - `test_rknn_shared_weights_single_root_multi_child`: 模拟同一个模型权重路径，验证多次请求只触发一次 root 加载，派生多个 child 会话；
  - `test_child_session_destructor_before_root`: 验证乱序释放 child session，root 引用计数安全递减，无悬挂指针；
  - `test_rknn_set_core_mask_per_instance_without_env`: 验证设核为逐实例会话级，无进程环境变量副作用；
  - `test_yolo_10_callsites_compile_and_pass`: 确保 10 处 `yolo.rs` 测试构造点与本地 runner 行为与 D1 前逐位一致（T44）；
  - `test_self_test_requires_hardware_no_fallback`: 验证自检模式下即使有 placement 注入，无硬件加速器时依然严格报错（T41）。

### Phase 2: InitContext 扩展与 YOLO 构造点同步
- [x] 在 `crates/algo-sdk/src/plugin.rs` 增加 `wire_placement: Option<WirePlacementMetadata>` 字段与 `target_core_mask()` / `with_placement()` 方法。
- [x] 同步修改 `crates/algo-sdk/src/models/yolo.rs` 中的 10 处测试构造点，确保全部平滑编译。
- [x] 同步更新 `crates/algo-sdk/src/macros.rs` 中的 `InitContext` 组装逻辑。

### Phase 3: RknnSymbols 符号绑定与共享权重运行时
- [x] 在 `crates/algo-sdk/src/runtime/platforms/rockchip.rs` 中：
  - 增加 `PfnRknnDupContext` 函数指针原型及 `rknn_dup_context` 符号解析；
  - 实现 `RknnRootWeight`（RAII 托管 root context）；
  - 实现 `RknnChildSession`（实现 `Send + !Sync`，提供独占所有权胶囊）；
  - 实现 `RknnSharedWeightProvider`（全局单例弱引用池 `Weak<RknnRootWeight>`，支持并发安全派生）。

### Phase 4: 插件导出宏接入 Placement 回执
- [x] 在 `crates/algo-sdk/src/macros.rs` 中：
  - 实现 `av_algo_get_placement_extension` 导出；
  - 在 `av_algo_instance_create` 成功后填充 `AvAlgoInstanceReceiptPod`；
  - 在 `av_algo_instance_destroy` 析构时生成 `AvAlgoCleanupReceiptPod`。

### Phase 5: 移除人脸识别全局串行 Actor
- [x] 在 `algo-packages/rknn/rk3588/face_recognition`：
  - 废弃全局 `static SHARED_MODELS` 队列与单例 `InferenceWorker`；
  - 重构 `FaceRecognizer`：直接持有独立的 YOLOv8-Face 与 EdgeFace `RknnChildSession`；
  - 移除全局邮箱排队逻辑，实现真正多实例、多核完全独立的无锁并行推理。

### Phase 6: 全量质量门禁与跨平台验证
- [x] 运行 `cargo fmt --all -- --check`。
- [x] 运行 `cargo clippy --all-targets -- -D warnings`。
- [x] 运行 `cargo nextest run --workspace`。
- [x] 运行各算法包编译与测试：
  - `algo-packages/macos`
  - `algo-packages/rknn/rk3588`
  - `algo-packages/rknn/rk3576`
  - `algo-packages/rknn/rk3568`

---

## 2. 门禁命令与验收标准

### 格式化与静态检查
```bash
cargo fmt --all -- --check
cargo clippy -p heimdall-algo-sdk -p heimdall-infer --all-targets -- -D warnings
cargo clippy --manifest-path algo-packages/rknn/rk3588/Cargo.toml --workspace --all-targets -- -D warnings
```

### 自动化测试
```bash
cargo nextest run -p heimdall-algo-sdk
cargo nextest run --workspace
```

### 验收判定
1. 彻底移除人脸识别全局串行 Actor，实现多实例纯并行；
2. 物理权重单份驻留通过 `rknn_dup_context` 派生验证；
3. 全工作区与所有算法包平台 workspace 编译测试全绿。
