# RK3588 人脸识别模型包说明

本目录包含适用于 Rockchip RK3588 平台的 NPU 模型权重（由 RKNN-Toolkit2 编译）。

## 模型列表

| 模型文件名 | 任务 | 输入尺寸 | 格式 / 精度 | 来源与特性 |
| :--- | :--- | :--- | :--- | :--- |
| `yolov6n_384x640_rk3588_i8.rknn` | COCO 人体检测 (person class 0) | 1×384×640×3 (NHWC) | RGB24, INT8, 直出 4 通道 | YOLOv6n 结构，无 DFL Softmax 开销 |
| `scrfd_500m_384x640_rk3588_mixed.rknn` | 视频流人脸检测与 5 点定位 (16:9 横屏轻量优化) | 1×384×640×3 (NHWC) | RGB24, INT8/FP16 混合精度 (9 个输出) | SCRFD-500M 骨干，三尺度 Score 保留 Float16，消除 0.5 截断 |
| `scrfd_2.5g_bnkps_640x640_rk3588_mixed.rknn` | 底库注册人脸检测与 5 点定位 (9:16/正方形照片优化) | 1×640×640×3 (NHWC) | RGB24, INT8/FP16 混合精度 (9 个输出) | SCRFD-2.5G BNKPS，三尺度 Score 保留 Float16 |
| `facelivtv2_l_realistic_final_super_rk3588_fp16.rknn` | 人脸特征提取 (FaceLiVTv2-L，监控退化自蒸馏微调版，**默认主选**) | 1×112×112×3 (NHWC) | RGB24, 纯 FP16 (512 维) | 12×MHLA 大容量模型，Super 8×8，0 CPU 回退 |
| `facelivtv2_m_realistic_final_super_rk3588_fp16.rknn` | 人脸特征提取 (FaceLiVTv2-M，监控退化自蒸馏微调版，可选轻量) | 1×112×112×3 (NHWC) | RGB24, 纯 FP16 (512 维) | 6×MHLA 空间对齐，0 CPU 回退，极致帧率备选 |

## 模型契约与说明

1. **YOLOv6n COCO 人体检测器**：
   - 输入为 `[1, 384, 640, 3]` NHWC RGB。
   - 输出为 3 个尺度的 box 与 cls 分支，box 分支直接输出 4 通道归一化坐标，无需 DFL CPU 算力损耗。
   - Rust 后处理仅保留 COCO class `0 = person`，最后执行 NMS。

2. **SCRFD 混合精度人脸检测器**：
   - **流式检测（640×384）**：面向视频流 16:9 监控画面，无黑边高效推理。
   - **注册检测（640×640）**：面向手机 9:16 竖屏自拍与正方形寸照。
   - 包含 3 个尺度检测头，输出 9 个张量（3 尺度 × [score 1 维, bbox 4 维距离偏移, kps 10 维 5 点关键点偏移]）。
   - **混合精度保真**：三尺度 Score 分支保留为 Float16，消除 INT8 饱和导致的 0.5000 截断缺陷。

3. **FaceLiVTv2-M / L 特征提取器**：
   - 输入为人脸仿射对齐后标准尺寸 112×112 RGB24。
   - 输入归一化由 RKNN 硬件层完成：`mean = [127.5, 127.5, 127.5]`, `std = [127.5, 127.5, 127.5]`。
   - 输出为 512 维 FP32 特征向量（已做 L2 范数归一化，模长精准为 1.000000）。
   - **度量学习铁律**：坚持纯 FP16 量化，绝不使用 INT8 混合精度，避免超球面坍缩。
   - **Super 8×8 空间对齐**：Stage 2 空间从 7×7 补齐为 8×8 并融合 BMM 算子，实现纯 NPU 硬件运行与 0 CPU 回退。
