# EdgeFace macOS arm64 人脸识别算法包

本算法包提供两条能力：

- `instance_process()`：使用 YOLOv5n-face 在子码流 `CVPixelBuffer` 上执行人脸检测、5 点关键点解析和质量门控，输出 `AV_RESULT_RECOGNITION` JSON。
- `av_algo_extract_face()`：接受宿主在 `snapshot_readback_path` 产生的 JPEG，执行检测、ArcFace 五点对齐和 EdgeFace-s 特征提取，返回 L2 归一化 512D embedding 与 112x112 JPEG。

模型输入与推理均在 CoreML 中配置 `MLComputeUnitsAll`，由 Apple Silicon 自动调度 ANE/GPU/CPU。JPEG 特征提取属于低频证据路径，允许 CPU 解码、对齐和 JPEG 编码；常驻检测路径通过 `AppleCvEngine` 保持 CVPixelBuffer 设备侧流转。

## 模型文件

构建前需要把转换后的模型包放在：

```text
model/yolov5n_face.mlpackage
model/edgeface_s.mlpackage
```

仓库中的 `weights/` 目录只保存转换输入权重，不会被运行时自动当作 CoreML 模型加载。模型转换参数、输入归一化和许可证信息见任务设计文档与 `docs/algo/EdgeFace.md`。

## 开发命令

```bash
make models  # Python 环境中从 weights/ 重新转换并验证两个 .mlpackage
make build
make test
make run
make benchmark
make package
```

`make test` 只运行纯 Rust 单元测试；依赖真实 macOS CoreML 模型和 ANE 的集成测试使用 `#[ignore]`，需要显式执行：

```bash
cargo test -p face-recognition -- --ignored
```

## C ABI 输出

检测 JSON 的核心形状为：

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

`bbox`、`landmarks` 全部归一化到 `[0, 1]`。`AvFaceExtractOutput::aligned_jpeg_data` 的最大容量为 65536 字节，超限会返回错误而不会截断 JPEG。
