# RK3568 人脸识别算法包 (EdgeFace Face Recognition)

基于 Rockchip RK3568 平台的工业级人脸识别算法包，集成 **YOLOv8n-face** 检测器（含 5 点关键点）与 **EdgeFace-xs** 512 维特征提取器。

## 平台契约

- **平台**: Rockchip RK3568 (NPU 1.0 TOPS @ INT8)
- **操作系统**: Linux aarch64 (Debian / Ubuntu / Buildroot)
- **运行时支持**: librknnrt.so >= 1.5.0
- **算法 ABI**: 女娲标准算法插件 C ABI (`av_algo_get_abi`)

## 模型清单

- `model/yolov8n-640x384-rk3568.rknn`: YOLOv8n COCO 人体检测模型（640×384，9 个 NCHW 输出，类别 0 为 person）
- `model/yolov8n-face-640x384_rk3568_mixed_face.rknn`: 人脸检测与 5 点关键点模型（640×384 混合精度）
- `model/edgeface_xs_gamma_06_rk3568_fp16.rknn`: 512 维人脸特征提取模型（112×112 FP16）

## 算法核心流水线

本算法包提供两条能力与全套端侧优化流水线：

- `instance_process()`：从同一原始帧生成一份 640×384 Letterbox RGB 输入，同时供人脸和人体检测模型使用；DMA-BUF 路径只保留一份 RGA 输出，再分别绑定两个 RKNN session，结合空间几何关联挂载与特写虚拟躯干 (Pseudo-body) 保底，由纯 Rust `ByteTracker` 维护稳定航迹。
- `BestShotManager` 动态抓拍与时域超球面特征融合：
  1. **低频算力门控**：初次入镜合格人脸立即触发特征提取；后续帧仅在姿态质量显著改善（$\Delta Q > 0.08$）或满足采样间隔（$\ge 6$ 帧）且未达上限（最多 4 帧）时触发 EdgeFace，彻底避免 RK3568 1.0 TOPS 算力逐帧空转；
  2. **超球面加权聚合**：按质量平方对 512 维单位特征向量增量加权累加，并重新 L2 归一化投影至单位超球面；
  3. **防漂移校验 (Anti-Drift Outlier Defense)**：新提取特征与当前融合特征的余弦相似度必须 $\ge 0.55$，拦截遮挡误检或跟踪漂移对特征池的污染；
  4. **低频抓拍侧载**：best-shot 目标在 `face.embedding` 携带 512 维 Float32 的 Base64 sidecar，仅供后端识别对账消费。
- `av_algo_extract_face()`：C ABI 独立特征提取符号，供宿主低频抓拍证据路径传入单帧 JPEG，执行检测、五点仿射对齐和 EdgeFace-xs 提取，返回 L2 归一化 512D embedding 与 112×112 JPEG。

## C ABI 输出规范

`instance_process()` 输出符合女娲标准规范的检测 Envelope：

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

## 验证指令

开发机检查与单元测试：
```bash
cargo check -p face-recognition-rk3568-rknn --all-targets
cargo test -p face-recognition-rk3568-rknn --lib
cargo clippy -p face-recognition-rk3568-rknn --all-targets -- -D warnings
```

在具备物理 RK3568 NPU 的硬件设备上运行：
```bash
# 硬件前向集成测试
cargo test -p face-recognition-rk3568-rknn -- --ignored --nocapture

# 本地端到端自测工具
cargo run -p face-recognition-rk3568-rknn --bin face_recognition_rk3568_rknn_run_local -- testimage.jpg --loops 20
```
