# Implementation Plan: macOS arm64 人脸识别算法包 (EdgeFace)

## 前置条件

- [x] PRD 已确认
- [x] Design 已确认
- [x] YOLOv5n-face ONNX → `.mlpackage` 转换完成
- [x] EdgeFace-s PyTorch → ONNX → `.mlpackage` 转换完成

> **模型转换需用户在 Python 环境中执行**（见 design.md §6），产出 `.mlpackage` 放入算法包 `model/` 目录。代码实现不依赖特定模型输出维度（运行时自省），但自检和测试需要模型文件存在。

---

## Phase A: 脚手架与配置 (无模型依赖)

### A1. 创建算法包目录与 Cargo.toml

- [x] 创建 `algo-packages/macos/arm64/face_recognition/` 目录结构
- [x] `Cargo.toml`: lib name = `face_recognition_coreml`, crate-type = `["cdylib", "rlib"]`, deps = `algo-sdk`, `serde`, `serde_json`, `image`, `uuid`
- [x] 添加到 workspace `Cargo.toml` members
- [x] 验证: `cargo check -p face-recognition`

### A2. manifest.json

- [x] `algorithm_id: "face_recognition"`, `algorithm_type: "face_recognition"`, `alarm_type_id: "face_recognize"`, `platform_id: "macos-arm64-coreml"`

### A3. config.rs + config.schema.json

- [x] `InstanceConfig` 结构体: `detection_confidence_threshold`, `min_face_size`, `quality_thresholds`
- [x] 对应 JSON Schema
- [x] 验证: serde 反序列化 round-trip 单元测试

### A4. lib.rs 骨架

- [x] `export_algo!` 宏调用
- [x] `av_algo_extract_face` 符号存根 (返回 `AV_ERR_NOT_IMPLEMENTED`)
- [x] 验证: `cargo build -p face-recognition` 产出 `.dylib`, 包含两个导出符号

---

## Phase B: 核心算法模块 (纯 Rust, 无 CoreML 依赖)

### B1. align.rs — 仿射对齐

- [x] InsightFace arcface 标准参考点常量 (112×112)
- [x] `estimate_affine(src: &[[f32; 2]; 5], dst: &[[f32; 2]; 5]) -> [f64; 6]` — 最小二乘仿射估计
- [x] `apply_affine(image: &[u8], w: u32, h: u32, matrix: &[f64; 6], out_size: u32) -> Vec<u8>` — 双线性插值变换
- [x] 单元测试:
  - 恒等变换 (源点 == 目标点) → 矩阵 ≈ [1,0,0, 0,1,0]
  - 已知旋转 → 验证变换正确性
  - 边界像素不越界

### B2. quality.rs — 质量门控

- [x] `estimate_yaw(landmarks: &[[f32; 2]; 5]) -> f32` — 关键点几何估算偏航角
- [x] `estimate_pitch(landmarks: &[[f32; 2]; 5]) -> f32` — 关键点几何估算俯仰角
- [x] `compute_quality(landmarks: &[[f32; 2]; 5], scores: &[f32; 5], face_w: f32, config: &QualityThresholds) -> FaceQuality`
- [x] 单元测试:
  - 标准正脸关键点 → yaw ≈ 0, pitch ≈ 0, 高质量分
  - 极度偏头关键点 → 大 yaw, 低质量分
  - 小脸 → face_size 低于阈值

### B3. detect.rs — YOLOv5n-face 后处理

- [x] `decode_yolov5_face(raw: &[f32], conf_thresh: f32) -> Vec<RawFace>` — 解析 `[1, N, 16]` 输出
- [x] `nms(faces: &mut Vec<RawFace>, iou_thresh: f32)` — IoU NMS
- [x] `RawFace { bbox: [f32; 4], landmarks: [[f32; 2]; 5], score: f32 }`
- [x] `unmap_letterbox(faces: &mut [RawFace], mode: &PreprocessMode, orig_w: u32, orig_h: u32)` — letterbox 逆映射 + 归一化
- [x] 单元测试:
  - 合成 anchor 输出 → 验证解码正确
  - 重叠框 → NMS 只保留一个
  - letterbox unmap → 坐标还原正确

### B4. postprocess.rs — 结果 JSON 序列化

- [x] `FaceDetectionResult` 序列化结构 (对齐 PRD §6.1 JSON schema)
- [x] `emit_face_detections(emitter: &mut ResultEmitter, faces: &[DetectedFace])` — 发射 `AV_RESULT_RECOGNITION`
- [x] 验证: JSON 输出与 schema 一致

---

## Phase C: CoreML 推理集成 (需要 macOS + 模型文件)

### C1. coreml.rs — 双模型加载器

- [x] `CoreMlFaceModels { detector: CoreMlRunner, facenet: CoreMlRunner }` — 持有两个模型
- [x] `CoreMlFaceModels::load(package_root: &Path) -> Result<Self, AlgoError>` — 自动发现 `model/` 下的 `.mlpackage`
- [x] YOLOv5n-face 推理: `predict_detector(pixelbuffer: *mut c_void) -> Result<Vec<f32>, AlgoError>`
- [x] EdgeFace-s 推理: `predict_edgeface(pixelbuffer: *mut c_void) -> Result<Vec<f32>, AlgoError>`
- [x] 模型输出维度运行时自省 (不硬编码元素数量)
- [x] `SHARED_MODELS: OnceLock<Arc<CoreMlFaceModels>>` 全局共享

### C2. plugin.rs — AlgoPlugin 实现

- [x] `FaceRecognizer { models: Arc<CoreMlFaceModels>, config: InstanceConfig }`
- [x] `init()`: 从 `SHARED_MODELS` 获取或初始化模型
- [x] `process()`:
  1. `AppleCvEngine.letterbox(frame, 640, 640, [0,0,0])` → BGRA CVPixelBuffer
  2. YOLOv5n-face 推理 → raw output
  3. `decode_yolov5_face()` + `nms()``
  4. `unmap_letterbox()` → 归一化坐标
  5. `compute_quality()` → 质量分
  6. `emit_face_detections()` → AV_RESULT_RECOGNITION
- [x] `update_config()`: 更新阈值

### C3. lib.rs — extract_face 完整实现

- [x] `av_algo_extract_face()`:
  1. 校验输入 (`validate_abi_header`)
  2. JPEG 解码 (`image::load_from_memory`)
  3. 构造临时 CVPixelBuffer (BGRA, letterbox 到 640×640) 或直接在 RGB 上做
  4. YOLOv5n-face 检测 → 取最大/最佳人脸
  5. 如无人脸 → `status_code = AV_ERR_INFERENCE_FAILED`, 提前返回
  6. 仿射对齐 → 112×112 RGB
  7. 构造 112×112 CVPixelBuffer → EdgeFace-s 推理
  8. L2 归一化 embedding → 填入 `output.embedding[0..dim]`, `output.embedding_dim`
  9. 编码对齐人脸为 JPEG → `output.aligned_jpeg_data`, `output.aligned_jpeg_len`
  10. 填入 `bbox`, `quality_score`, `detection_score`
- [x] catch_unwind Panic 隔离

### C4. lib.rs — library_open/close hook 增强

- [x] 在 `export_algo!` 展开的 `library_open` 后，通过包装层触发 `SHARED_MODELS.get_or_init()`
- [x] 在 `library_close` 前清理模型引用（`Arc` 引用计数自动回收）
- [x] 方案: 使用自定义 `init_models` / `cleanup_models` 函数 hook

---

## Phase D: 本地评测工具

### D1. run_local.rs

- [x] 单帧检测模式: 加载 testimage.jpg → process() → 输出检测结果 + 标注框 result.jpg
- [x] 特征提取模式: 加载图片 → extract_face() → 打印 embedding 前 10 维 + 质量分
- [x] 相似度测试: 两张图 → 分别 extract_face() → 余弦相似度
- [x] Benchmark 模式 (`--benchmark`): 循环推理 → P50/P99/Avg/FPS
- [x] 从 `.env` 读取 `TEST_IMAGE`, `CONFIDENCE_THRESHOLD` 等参数

---

## Phase E: 测试与验证

### E1. 单元测试

- [x] `align.rs` 仿射变换测试
- [x] `quality.rs` 质量评估测试
- [x] `detect.rs` YOLOv5n-face 后处理测试
- [x] `config.rs` 配置解析测试

### E2. 集成测试

- [x] `tests/hardware_infer_test.rs` — 完整 ANE 推理测试 (`#[ignore]`)
- [x] 沙箱自检 (self-test mode) 通过

### E3. 质量门禁

- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test --workspace` 全绿

---

## 验证指令

```bash
# 编译
cargo build -p face-recognition

# 格式化
cargo fmt --all

# Lint
cargo clippy --all-targets -- -D warnings

# 非硬件测试
cargo test --workspace

# 硬件集成测试 (需要 macOS + 模型文件)
cargo test -p face-recognition -- --ignored

# 本地单帧评测
cargo run -p face-recognition --bin run_local

# 性能 benchmark
cargo run -p face-recognition --bin run_local -- --benchmark
```

---

## 回滚点

| 阶段 | 回滚动作 |
|---|---|
| Phase A | 删除 `algo-packages/macos/arm64/face_recognition/` 目录，移除 workspace member |
| Phase B | 各模块独立，只需 revert 对应文件 |
| Phase C | CoreML 集成出问题时 B 阶段的纯 Rust 逻辑仍可保留 |
| Phase D | run_local 是独立 binary，不影响库功能 |
