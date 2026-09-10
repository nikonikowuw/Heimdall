# RK3568 人脸识别模型包说明

本目录包含适用于 Rockchip RK3568 平台的 NPU 模型权重（由 RKNN-Toolkit2 编译）。

## 模型列表

| 模型文件名 | 任务 | 输入尺寸 | 格式 / 精度 | 来源 |
| :--- | :--- | :--- | :--- | :--- |
| `yolov8n-face-640x384_rk3568_mixed_face.rknn` | 人脸检测与 5 点关键点定位 | 1×384×640×3 (NHWC) | RGB24, INT8/FP16 混合精度 | rknn_model_zoo YOLOv8n-face (WiderFace) |
| `edgeface_xs_gamma_06_rk3568_fp16.rknn` | 人脸特征提取 (512D) | 1×112×112×3 (NHWC) | RGB24, FP16 | rknn_model_zoo EdgeFace-xs (InsightFace) |

## 模型契约与说明

1. **YOLOv8n-face 检测器**：
   - 包含 3 个尺度检测头，输出 12 个张量（box 64 维 DFL + 1 维 cls 置信度 + 15 维关键点坐标与置信度）。
   - 输入归一化由 RKNN 硬件层完成：`mean = [0, 0, 0]`, `std = [255, 255, 255]`。

2. **EdgeFace-xs 特征提取器**：
   - 输入为人脸仿射对齐后标准尺寸 112×112 RGB24。
   - 输入归一化由 RKNN 硬件层完成：`mean = [127.5, 127.5, 127.5]`, `std = [127.5, 127.5, 127.5]`。
   - 输出为 512 维 FP32 特征向量（已做 L2 范数归一化，模长为 1.0）。
