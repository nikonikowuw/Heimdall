# 架构与技术设计文档: 基于 algo-sdk 重构 yolo26n CoreML 算法包

## 1. 总体定位与架构

本项目将原位于 `/Users/niko/dev/go/argus/algo-packages/macos/arm64/yolo26n` 的 C++/CMake 算法包重构为纯 Rust 语言编写的动态库 crate，直接安家在当前 Heimdall 代码仓库的 `algo-packages/macos/arm64/yolo26n/` 目录下。

```text
Host Runtime (crates/infer, crates/pipeline)
          │
          │ Standard C ABI (av_algo_get_abi)
          ▼
┌────────────────────────────────────────────────────────┐
│ algo-packages/macos/arm64/yolo26n/lib/libgeneral_detection.dylib │
│                                                        │
│  ┌──────────────────────────────────────────────────┐  │
│  │ export_algo! C ABI 虚表胶水层 (algo-sdk)           │  │
│  │  - Panic 绝对隔离屏障 (catch_unwind)              │  │
│  │  - 152 字节 AvFrameDesc 内存布局校验              │  │
│  └──────────────────────────────────────────────────┘  │
│                           │                            │
│  ┌────────────────────────▼─────────────────────────┐  │
│  │ GeneralDetector / DetectorInstance (AlgoPlugin)  │  │
│  │                                                  │  │
│  │  1. 硬件预处理 (AppleCvEngine)                   │  │
│  │     SafeFrame (NV12 CVPixelBuffer)               │  │
│  │       → vImage Accelerate (640x384 BGRA)         │  │
│  │       (零 CPU 拷贝，纯 Apple 统一显存流转)        │  │
│  │                                                  │  │
│  │  2. 模型推理 (CoreMlRunner)                       │  │
│  │     CVPixelBufferRef → MLModel (yolo26n)         │  │
│  │       → var_911 [1, 300, 6] MultiArray           │  │
│  │                                                  │  │
│  │  3. 后处理与发射 (Postprocessor)                  │  │
│  │     置信度/类别掩码过滤 → unmap_box 扣除黑边     │  │
│  │       → ResultEmitter 零分配 JSON 回调           │  │
│  └──────────────────────────────────────────────────┘  │
└────────────────────────────────────────────────────────┘
```

---

## 2. 模块划分与职责

### 2.1 `Cargo.toml`
- `[lib] name = "general_detection", crate-type = ["cdylib", "rlib"]`。
- 构建生成 `libgeneral_detection.dylib` 并直接输出/同步至 `algo-packages/macos/arm64/yolo26n/lib/`。
- 依赖项：`algo-sdk`（`path = "../../../../crates/algo-sdk"`）、`serde`、`serde_json`。

### 2.2 `src/config.rs` (参数解析与 COCO 掩码)
- 定义 `InstanceConfig` 结构体，支持反序列化：
  - `confidence_threshold`: `f32` (默认 0.45)
  - `iou_threshold`: `f32` (默认 0.45)
  - `target_classes`: `Vec<String>` (默认 6 类: person, car, motorcycle, bicycle, bus, truck)
  - `custom_alarm_label`: `Option<String>`
- 提供 `ClassMask([u64; 2])`：对 80 类别提供 $O(1)$ 位运算过滤；若 `target_classes` 为空则默认全开。
- 提供 80 个标准 COCO 类别字符串常量表。

### 2.3 `src/coreml.rs` (Apple CoreML 安全 FFI 绑定)
- 纯 Rust 对接 Apple `CoreML.framework` 与 `Foundation.framework` 的 Objective-C Runtime：
  - `objc_getClass` / `sel_registerName` / `objc_msgSend` 安全封装。
  - 模型动态编译：若传入 `.mlpackage` 且未编译，调用 `[MLModel compileModelAtURL:error:]`，产出编译缓存目录；若已编译则直接加载。
  - 模型实例创建：`MLModelConfiguration.computeUnits = MLComputeUnitsAll` (自动调度 Neural Engine ANE 与 GPU)，加载 `[MLModel modelWithContentsOfURL:configuration:error:]`。
  - 零拷贝前向推理：
    - `[MLFeatureValue featureValueWithPixelBuffer:cv_buffer]`
    - `[MLDictionaryFeatureProvider initWithDictionary:]`
    - `[MLModel predictionFromFeatures:error:]`
    - 提取 `output_provider` 的 `var_911` feature，读取 `MLMultiArray` 的 `dataPointer`。
    - 将 300 × 6 = 1800 个浮点数安全拷贝到 Rust 缓冲区。

### 2.4 `src/postprocess.rs` (后处理与黑边还原)
- 输入：`&[f32]` (长度 1800)、`&InstanceConfig`、`&PreprocessMode`、原图宽 `orig_w`、原图高 `orig_h`。
- 处理流水线：
  1. 遍历 300 个候选检测（每组 6 个 float：`[x1, y1, x2, y2, score, cls_id]`）。
  2. 校验数字有效性（`is_finite`）与 `score >= config.confidence_threshold`。
  3. `cls_id` 范围校验 `[0, 79]`，并检查 `class_mask.is_enabled(cls_id)`。
  4. 将 `[x1, y1, x2, y2]` 归一化到模型尺度 640 × 384。
  5. 调用 `algo_sdk::math::unmap_box`，根据 Letterbox 布局精准剔除四周黑边偏移，线性还原至原始视频帧的 `[0.0, 1.0]` 归一化空间。
  6. 组装为 `NormBox`，附加标签。

### 2.5 `src/plugin.rs` & `src/lib.rs`
- 实现 `AlgoPlugin` 与 `AlgoInstance` trait。
- 实例在 `process` 中串联预处理、推理与发射。
- 调用 `export_algo!(GeneralDetector, algo_id: "general_detection", ...)` 导出 C ABI。

---

## 3. 宿主目录与扫描兼容性

- 在 `crates/app/src/reconcile.rs` 中，确保扫描候选路径追加 `base_algo_dir.join("macos").join("arm64")`。
- 在 `crates/infer/src/package.rs` 的 `discover_package_dirs` 中，确保能递归穿透到 `algo-packages/macos/arm64/yolo26n`，与原有 `algo-packages/macos-arm64/general_detection` 并存并优先匹配。
- 同时在 `algo-packages/macos-arm64/general_detection/lib/` 保留软链接或同步最新编译产物，确保现有测试不发生回归。

---

## 4. 零拷贝性能与内存保障

1. **统一显存直通**：
   - 原始帧以 `SafeFrame` 接收 `CVPixelBuffer`。
   - `AppleCvEngine` 内部使用 `vImageConvert_420Yp8_CbCr8ToARGB8888` 与 `vImageScale_ARGB8888` 直接在 CVPixelBuffer surface 间操作。
   - 推理直接向 CoreML 提交目标 `CVPixelBuffer`，CoreML 内部直接映射至 ANE 物理总线，整条链条 CPU 零读取像素。
2. **RAII 资源生命周期**：
   - 目标 `CVPixelBuffer` 遵循 CoreVideo `CVPixelBufferRelease` 规则。
   - Objective-C 对象由 `@autoreleasepool` 对应 Rust RAII 守卫或显式 release 释放，杜绝单帧推理显存泄漏。
