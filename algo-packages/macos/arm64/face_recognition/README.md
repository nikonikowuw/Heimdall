# EdgeFace macOS arm64 人脸识别算法包

本算法包提供两条能力：

- `instance_process()`：在同一个 640x384 `CVPixelBuffer` 上运行 `yolo26n` 人体检测和 `yolov8_face` 人脸/五点检测，完成当前帧空间关联与质量门控；best-shot 在当前算法 worker 内通过 Core Image 从原生 `CVPixelBuffer` 同步仿射到 112x112 BGRA surface，再直接交给 EdgeFace/ANE，不执行整帧 D2H readback。
- `av_algo_extract_face()`：接受宿主在 `snapshot_readback_path` 产生的 JPEG，执行检测、五点对齐和 EdgeFace-s 特征提取，返回 L2 归一化 512D embedding 与 112x112 JPEG。

模型输入与常驻检测均在 CoreML 中配置 `MLComputeUnitsAll`，由 Apple Silicon 自动调度 ANE/GPU/CPU。JPEG 特征提取属于低频证据路径，允许 CPU 解码、对齐和 JPEG 编码；常驻检测和 best-shot 特征提取均保持原生 `CVPixelBuffer` 设备侧流转。best-shot 的可选 embedding sidecar 只在后端内存中消费，不进入 WebSocket/HTTP DTO。

## 模型文件

构建前需要把运行时模型包放在：

```text
model/yolo26n.mlpackage
model/yolov8_face.mlpackage
model/edgeface_s.mlpackage
```

仓库中的 `weights/` 目录只保存转换输入权重，不会被运行时自动当作 CoreML 模型加载。模型转换参数、输入归一化和许可证信息见任务设计文档与 `docs/algo/EdgeFace.md`。

## 开发命令

```bash
make models  # Python 环境中重新转换并验证 CoreML 模型
make models-sync
make build
make test
make run
make benchmark
make package
```

`make test` 运行纯 Rust 单元测试；依赖真实 macOS CoreML 模型和 ANE 的集成测试使用 `#[ignore]`，需要显式执行：

```bash
cargo test -p face-recognition -- --ignored
```

## C ABI 输出

`instance_process()` 使用标准检测 Envelope：

```json
{
  "schema_version": 1,
  "objects": [
    {
      "class_id": 0,
      "label": "person",
      "confidence": 0.95,
      "bbox": [0.1, 0.2, 0.4, 0.8],
      "face": {
        "bbox": [0.15, 0.22, 0.25, 0.35],
        "confidence": 0.92,
        "quality_score": 0.82,
        "embedding": "dHkevWKSYDxR4/K8..."
      }
    }
  ]
}
```

`bbox` 是归一化的人体框 `[x1, y1, x2, y2]`，`face` 是关联且通过质量门控的精细人脸详情。以人体为主体锚定宿主 `SimpleTracker` 航迹，提供连续鲁棒的 IoU 追踪；`face` 包含人脸检测框、置信度、质量分与低频 best-shot embedding 供抓拍与人脸比对。空 `objects` 是合法成功结果。插件不输出 `trackId`、`eventId`、`tracks` 或 `isPseudoBody`；宿主 `PipelineManager` 是跨帧航迹和规则状态的唯一所有者。best-shot 目标可以在 `face.embedding` 带有固定 512 维 little-endian Float32 的 Base64 sidecar，但该字段只供后端识别链路使用，绝不转发到浏览器。

`face_recognition_run_local` 是单帧后端验证工具：每个通过质量门控的目标都会在当前同步调用中立即执行一次设备侧 EdgeFace，并在本地 JSON 的该目标上附带 `embedding` Base64 sidecar。这个 sidecar 只用于后端验证/识别，不代表浏览器或 WebSocket 会收到 embedding。

### 时域超球面特征融合与防漂移机制

算法包在 `BestShotManager` 中实现了基于 ByteTrack 航迹生命周期的动态特征融合策略：
1. **时域采样与算力约束**：初次入镜合格人脸立即触发特征提取；后续帧仅在质量显著超越历史最高（$\Delta Q > 0.08$）或满足采样间隔（$\ge 6$ 帧）且单目标融合帧数未达上限（最多 4 帧）时触发 EdgeFace 推理，彻底杜绝 NPU 逐帧无谓空转；
2. **超球面加权聚合**：将提取的 512 维单位特征向量按质量分平方（$w_i = Q_i^2$）进行增量加权累加，并重新 L2 归一化投影至单位超球面（$\|\mathbf{v}_{\text{fused}}\|_2 = 1$）；
3. **防漂移校验 (Anti-Drift Outlier Defense)**：新提取的特征与当前融合特征的余弦相似度必须 $\ge 0.55$，若低于门限（怀疑追踪器漂移、严重遮挡或目标混淆），自动拦截并放弃融合，确保特征池绝对纯净。

`AvFaceExtractOutput::aligned_jpeg_data` 的最大容量为 65536 字节，超限会返回错误而不会截断 JPEG。
