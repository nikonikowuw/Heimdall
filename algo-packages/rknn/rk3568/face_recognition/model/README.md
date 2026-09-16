# RK3568 人脸识别模型包说明

本目录包含适用于 Rockchip RK3568 平台的 NPU 模型权重（由 RKNN-Toolkit2 编译）。

> **模型转换记录**: 详见 [CONVERSION.md](CONVERSION.md)

## 模型列表

| 模型文件名 | 任务 | 输入尺寸 | 格式 / 精度 | 来源 |
| :--- | :--- | :--- | :--- | :--- |
| `yolov8n-640x384-rk3568.rknn` | COCO 人体检测 (person class 0) | 1×384×640×3 | RGB24, 9 个 NCHW INT8 输出（3 个尺度） | 修正输出元数据后的 rknn_model_zoo YOLOv8n 640×384 |
| `scrfd_2.5g_bnkps_640x384_rk3568_mixed.rknn` | 视频流人脸检测与 5 点定位 (16:9 横屏优化) | 1×384×640×3 (NHWC) | RGB24, INT8/FP16 混合精度 (9 个输出) | rknn_model_zoo SCRFD-2.5G BNKPS |
| `scrfd_2.5g_bnkps_640x640_rk3568_mixed.rknn` | 底库注册人脸检测与 5 点定位 (9:16/正方形照片优化) | 1×640×640×3 (NHWC) | RGB24, INT8/FP16 混合精度 (9 个输出) | rknn_model_zoo SCRFD-2.5G BNKPS |
| `edgeface_s_gamma_05_rk3568_fp16.rknn` | 人脸特征提取 (512D) | 1×112×112×3 (NHWC) | RGB24, FP16 | rknn_model_zoo EdgeFace-S (InsightFace) |

## 模型契约与说明

1. **YOLOv8n COCO 人体检测器**：
   - 输入为 `[1,384,640,3]` NHWC RGB，使用 `640×384` 专用模型。
   - 输出为 3 个尺度的 box、class、score 分支，共 9 个 NCHW INT8 张量：`[1,64,48,80]`、`[1,80,48,80]`、`[1,1,48,80]`，以及对应的 `24×40`、`12×20` 两组。
   - Rust 后处理对 box 分支执行 INT8 反量化、DFL 和 stride 解码，仅保留 COCO class `0 = person`，最后执行 NMS。

2. **SCRFD-2.5G BNKPS 检测器（双尺寸架构）**：
   - **流式检测（640×384）**：面向视频流 16:9 监控画面，无黑边高效推理。
   - **注册检测（640×640）**：面向手机 9:16 竖屏自拍与正方形寸照，大幅减少黑边留白，保留人脸 67% 以上的分辨率与关键点精度。
   - 包含 3 个尺度检测头，输出 9 个张量（3 尺度 × [score 1 维 (已 Sigmoid), bbox 4 维距离偏移, kps 10 维 5 点关键点偏移]）。
   - 每个网格配置 2 个 anchors，无 DFL 展开冗余，关键点定位极其稳定。
   - 输入归一化由 RKNN 硬件层完成：`mean = [127.5, 127.5, 127.5]`, `std = [128.0, 128.0, 128.0]`。

3. **EdgeFace-S 特征提取器**：
   - 输入为人脸仿射对齐后标准尺寸 112×112 RGB24。
   - 输入归一化由 RKNN 硬件层完成：`mean = [127.5, 127.5, 127.5]`, `std = [127.5, 127.5, 127.5]`。
   - 输出为 512 维 FP32 特征向量（已做 L2 范数归一化，模长为 1.0）。
