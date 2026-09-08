# PRD: macOS arm64 人脸识别算法包 (EdgeFace)

## 1. 概述

在 `algo-packages/macos/arm64/` 下新建 `face_recognition` 算法包，遵循现有 C ABI 算法包规范（`algo-sdk` + `export_algo!`），通过 Core ML / ANE 硬件加速提供人脸检测与特征提取两个纯感知能力。

本算法包作为 EdgeFace 系统（`docs/algo/EdgeFace.md`）的感知层实现，在 Mac 开发机上完成端到端验证：VideoToolbox 硬解 → CVPixelBuffer → Core ML (ANE) 推理。

## 2. 动机

- 在项目已有 `general_detection` (YOLO) 算法包之外，补充人脸识别垂类能力
- 通过 Mac 平台的 Core ML / ANE 硬件直通验证端到端零拷贝推理模式，积累的架构经验可平移到 RK3588 / Ascend
- C ABI 已预留 `av_algo_extract_face` 符号和 `AvFaceExtractInput` / `AvFaceExtractOutput` 结构体，本任务为其首个实现

## 3. 职责边界

### 算法包内（纯感知）

| 能力 | C ABI 入口 | Core ML 模型 | 输入 | 输出 |
|---|---|---|---|---|
| **人脸检测** | `instance_process()` | YOLOv5n-face `.mlpackage` | `AvFrameDesc` (NV12 CVPixelBuffer) | `AvAlgoResult { kind: RECOGNITION }` JSON 含 bbox + 5 关键点 + 质量分 |
| **特征提取** | `av_algo_extract_face()` | EdgeFace-s `.mlpackage` | `AvFaceExtractInput` (JPEG bytes) | `AvFaceExtractOutput { embedding[512], aligned_jpeg }` |

### 算法包外（宿主 pipeline 职责，不在本任务范围）

- ByteTrack 跟踪与 Track 成熟判定
- 主码流按需解码与高清人脸裁剪（`snapshot_readback_path`）
- 多帧 Embedding 融合（纯数学加权平均，无模型依赖）
- 底库管理、比对、身份决策
- 自适应阈值与拒识策略

## 4. 技术选型

### 4.1 模型

| 角色 | 模型 | 参数量 | 输入尺寸 | 输出 | 许可证 |
|---|---|---|---|---|---|
| 人脸检测 | YOLOv5n-face (deepcam-cn) | 1.7M | 640×384 (16:9 宽屏监控) | bbox + 5 点关键点 + score | GPL-3.0 |
| 特征提取 | **EdgeFace-s** (Idiap Research Institute, 2024) | ~1.8M | 112×112 RGB | **512D** embedding (L2 归一化) | **Apache 2.0** |

> **输入尺寸说明**: 监控视频流主流为 16:9 宽屏分辨率。YOLOv5n-face 采用 640×384 静态分辨率输入，可避免 640×640 带来的冗余填充与算力浪费，并支持 ANE 静态网格编译，前向检测时延压缩至极限。
> **许可证说明**: InsightFace 的**识别模型权重**（ArcFace / MobileFaceNet 等）不可商用，因此特征提取骨干网络选用 Apache 2.0 许可的 EdgeFace。人脸检测采用 YOLOv5n-face。

### 4.2 模型来源与转换

- YOLOv5n-face: ONNX → `coremltools` 转 `.mlpackage`
- EdgeFace-s: [github.com/otroshi/edgeface](https://github.com/otroshi/edgeface) 官方 PyTorch 权重 → ONNX 导出 → `coremltools` 转 `.mlpackage`
- 推理精度: ANE 原生 FP16，embedding 层考虑保持 FP32 防精度损失

### 4.3 平台

- 目标平台: `macos-arm64-coreml`
- 硬件加速: Apple Neural Engine (ANE) / GPU / CPU 自动调度（`MLComputeUnitsAll`）
- 输入零拷贝: CVPixelBuffer 直通 Core ML，与现有 `general_detection` 一致

## 5. 算法包结构

```
algo-packages/macos/arm64/face_recognition/
├── Cargo.toml
├── manifest.json                  # algorithm_type: "face_recognition"
├── config.schema.json             # 置信度阈值、最小人脸大小等
├── model/
│   ├── yolov5n_face.mlpackage/    # YOLOv5n-face 人脸检测模型
│   └── edgeface_s.mlpackage/      # EdgeFace-s 特征提取模型 (Apache 2.0)
├── testimage.jpg                  # 含人脸的测试图
├── src/
│   ├── lib.rs                     # export_algo! + av_algo_extract_face 导出
│   ├── plugin.rs                  # AlgoPlugin trait 实现
│   ├── config.rs                  # InstanceConfig 配置反序列化
│   ├── coreml.rs                  # CoreML 模型加载与推理（双模型）
│   ├── detect.rs                  # YOLOv5-face 后处理（grid 解码、NMS）
│   ├── align.rs                   # 5 点关键点仿射对齐 112×112
│   ├── quality.rs                 # 质量门控（姿态/大小/模糊/遮挡评分）
│   └── postprocess.rs             # 结果组装与 JSON 序列化
├── src/bin/
│   └── run_local.rs               # 本地评测工具（单帧检测 + 特征提取 + benchmark）
└── tests/
    └── hardware_infer_test.rs     # ANE 硬件集成测试 (#[ignore])
```

## 6. C ABI 契约

### 6.1 `instance_process()` — 人脸检测

输入：`AvFrameDesc`（NV12 CVPixelBuffer，子码流分辨率）

输出：`AvAlgoResult { kind: AV_RESULT_RECOGNITION }`，JSON 格式：

```json
{
  "event_id": "uuid",
  "faces": [
    {
      "bbox": [0.1, 0.2, 0.15, 0.2],
      "landmarks": [[0.15, 0.25], [0.22, 0.25], [0.18, 0.32], [0.14, 0.37], [0.21, 0.37]],
      "detection_score": 0.95,
      "quality": {
        "score": 0.82,
        "yaw": 12.5,
        "pitch": -5.0,
        "blur": 0.15,
        "face_size": 96
      }
    }
  ]
}
```

- `bbox` 归一化 `[x, y, w, h]` ∈ [0, 1]
- `landmarks` 归一化 5 点坐标 ∈ [0, 1]（双眼、鼻尖、双嘴角）
- `quality.score` 综合质量分 ∈ [0, 1]
- `quality.yaw` / `quality.pitch` 单位为度
- `quality.face_size` 为检测框在原图上的像素宽度
- `quality.blur` 模糊度 ∈ [0, 1]，越大越模糊

### 6.2 `av_algo_extract_face()` — 特征提取

使用已定义的 `AvFaceExtractInput` / `AvFaceExtractOutput` 结构体。

输入：JPEG 编码的人脸图像（来自宿主主码流高清裁剪）

处理流程：
1. JPEG 解码
2. 人脸检测（YOLOv5n-face），获取关键点
3. 基于 5 点关键点进行仿射对齐到 112×112
4. EdgeFace-s 特征提取 → 512D embedding（输出到 `embedding[0..512]`，`embedding_dim = 512`）
5. 对齐后的 112×112 人脸 JPEG 编码 → `aligned_jpeg_data` + `aligned_jpeg_len`

输出字段：
- `status_code`: 0=成功, 非0=失败
- `embedding[0..512]`: L2 归一化的 512D 特征向量
- `embedding_dim`: 512
- `bbox`: 检测到的人脸归一化框 [x, y, w, h]
- `quality_score`: 质量分 ∈ [0, 1]
- `detection_score`: 检测置信度
- `aligned_jpeg_data` + `aligned_jpeg_len`: 对齐后人脸缩略图

## 7. 配置 Schema

```json
{
  "detection_confidence_threshold": 0.5,
  "min_face_size": 30,
  "quality_thresholds": {
    "min_score": 0.3,
    "max_yaw": 45.0,
    "max_pitch": 30.0,
    "max_blur": 0.7
  }
}
```

## 8. 验收标准

### 功能验收

- [ ] `cargo build` 编译通过，产出 `libface_recognition.dylib`
- [ ] `manifest.json` 通过七步沙箱校验
- [ ] `instance_process()` 对含人脸的测试图正确输出 bbox、5 点关键点和质量分
- [ ] `instance_process()` 对无人脸图片不产出结果
- [ ] `av_algo_extract_face()` 输入含人脸 JPEG，输出 512D embedding（L2 范数 ≈ 1.0）
- [ ] `av_algo_extract_face()` 输出 aligned_jpeg 可解码为有效 112×112 JPEG
- [ ] 同一人不同照片的 embedding 余弦相似度 > 0.5
- [ ] 不同人照片的 embedding 余弦相似度 < 0.4
- [ ] `run_local` 单帧检测 + 特征提取端到端可运行并输出可视化结果
- [ ] `run_local --benchmark` 输出 P50/P99/FPS 性能数据

### 质量验收

- [ ] `cargo fmt --all -- --check` 通过
- [ ] `cargo clippy --all-targets -- -D warnings` 通过
- [ ] `cargo test --workspace` 全绿（硬件测试标记 `#[ignore]`）
- [ ] 无 `dbg!` / `println!` / `todo!()` 残留
- [ ] 每个 `unsafe` 块有准确的 `// SAFETY:` 注释
- [ ] 模型文件 `.mlpackage` 包含在算法包目录中

### 性能预期（Mac M 系列芯片）

- YOLOv5n-face 检测: < 5ms/帧 (ANE)
- EdgeFace-s 特征提取: < 5ms/次 (ANE)
- 端到端（检测 + 质量 + 对齐 + 提取）: < 15ms

## 9. 不做的事情

- 不在算法包内实现跟踪、多帧融合、底库比对
- 不修改宿主 pipeline（`crates/pipeline`）
- 不修改现有 C ABI 类型定义（`AvFaceExtractInput` / `AvFaceExtractOutput` 已定义）
- 不涉及前端 UI 变更
- 不涉及 RK3588 / Ascend 平台适配（后续独立任务）
- 不涉及训练或微调模型（使用 YOLOv5n-face 预训练检测权重 + EdgeFace-s Apache 2.0 预训练识别权重）

## 10. 依赖与前提

- YOLOv5n-face ONNX 模型
- EdgeFace-s PyTorch 权重 → ONNX 导出 → `coremltools` 转 `.mlpackage`
- `coremltools` + `torch` + `onnx` Python 工具链
- macOS 14.0+ 开发机（Apple Silicon）
- `algo-sdk` crate 现有能力（`CvEngine`、`SafeFrame`、`ResultEmitter`、`export_algo!`）

## 11. 风险

| 风险 | 影响 | 缓解 |
|---|---|---|
| YOLOv5n-face ONNX 含 `coremltools` 不支持的算子 | 转换失败 | 备选 YuNet (Apache 2.0) 或 RetinaFace (MIT) |
| EdgeFace-s PyTorch → ONNX 导出失败 | 转换阻塞 | 参照官方仓库 export 脚本，或退回 EdgeFace-base |
| ANE FP16 量化导致 embedding 精度损失 | 识别率下降 | EdgeFace-s 最后 FC 层指定 FP32 精度 |
| `AvFaceExtractOutput` 固定 65536 字节 aligned_jpeg 缓冲区不够 | JPEG 截断 | 112×112 JPEG 通常 < 10KB，风险极低 |
| 仿射对齐在非正方形输入上产生畸变 | 特征质量差 | 严格使用 InsightFace 标准对齐模板 |
