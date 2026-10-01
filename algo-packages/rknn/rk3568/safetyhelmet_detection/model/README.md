# yolov8n-hard-hat-detection

## Table of contents

- [1. Description](#1-description)
- [2. Current Support Platform](#2-current-support-platform)
- [3. Pretrained Model](#3-pretrained-model)
- [4. Convert to RKNN](#4-convert-to-rknn)
- [5. Python Demo](#5-python-demo)
- [6. C/C++ Demo](#6-cc-demo)
- [7. Expected Results](#7-expected-results)
- [8. Model Details](#8-model-details)
- [9. Quantization Analysis](#9-quantization-analysis)



## 1. Description

YOLOv8n 微调用于安全帽检测模型。

模型检测 2 个类别：`Hardhat`（安全帽）、`NO-Hardhat`（未戴安全帽）

输入尺寸：**384×640（16:9）**，适配常见的摄像头比例。

原始模型来源：https://huggingface.co/keremberke/yolov8n-hard-hat-detection

RKNN 优化版 ONNX 使用 https://github.com/airockchip/ultralytics_yolov8 导出：
- 去除解耦头（Decoupled Head），移除后处理到 CPU
- 去除 DFL（Distribution Focal Loss）结构，提升 NPU 推理速度
- 增加 score-sum 输出分支，加速后处理过滤



## 2. Current Support Platform

RK3562, RK3566, RK3568, RK3576, RK3588, RV1126B, RV1109, RV1126, RK1808, RK3399PRO



## 3. Pretrained Model

Pre-trained model: `model/best.pt`

### 导出 RKNN 优化版 ONNX

```bash
cd /path/to/ultralytics_yolov8
PYTHONPATH=./ python -c "
from ultralytics import YOLO
model = YOLO('/path/to/best.pt')
model.export(format='rknn', imgsz=[360, 640])  # 16:9
"
```

### 预训练模型

| Model | Precision | Size | Input | 状态 |
|-------|-----------|------|-------|------|
| `yolov8_hard_hat.rknn` | INT8 | 4.0 MB | 384×640 | **当前使用**（6-tensor，logits 分类头） |
| best_fp16.rknn | FP16 | 7.1 MB | 384×640 | 历史 |
| best_hybrid.rknn | INT8 (auto_hybrid) | 4.2 MB | 384×640 | 历史（9-tensor，含 score_sum） |
| best_pure.rknn | INT8 (pure) | 4.2 MB | 384×640 | 历史 |

> 历史模型文件已不再入库，其契约（9-tensor / 图内 sigmoid / score_sum 快筛）
> 仅在本节保留作参照；当前算法包由 `MODEL_PATH` 指向 `yolov8_hard_hat.rknn`。
> 当前模型的输入输出契约与转换参数见 [`CONVERSION.md`](./CONVERSION.md)。



## 4. Convert to RKNN

使用 `datasets/safetyhelmet_dataset`（19 张安全帽图片）进行量化校准：

```shell
cd python
python convert.py <onnx_model> <TARGET_PLATFORM> <dtype(optional)> <output_rknn_path(optional)>

# 示例:
python convert.py ../model/best.onnx rk3568
# 输出: ../model/best.rknn
```

参数说明：
- `<onnx_model>`: ONNX 模型路径
- `<TARGET_PLATFORM>`: NPU 平台，如 `rk3588`
- `<dtype>(optional)`: `i8`（默认，INT8量化）、`u8`、`fp`（不量化）
- `<output_rknn_path>(optional)`: 输出路径，默认 `../model/best.rknn`

## 5. Python Demo

```shell
cd python

# ONNX 推理
python yolov8n_hard_hat.py --model_path ../model/best.onnx --img_show

# RKNN 推理
python yolov8n_hard_hat.py --model_path ../model/best.rknn --target rk3568 --img_show
```

## 6. C/C++ Demo

### 编译

```shell
./build-linux.sh -t rk3568 -a aarch64 -d yolov8n-hard-hat-detection
```

### 推送并运行

```shell
adb push install/rk356x_linux_aarch64/rknn_yolov8_hard_hat_demo/ /data/
adb shell
cd /data/rknn_yolov8_hard_hat_demo
export LD_LIBRARY_PATH=./lib
./rknn_yolov8_hard_hat_demo model/best_pure.rknn model/test.jpg
```

## 7. Expected Results

```
Hardhat @ (120 85 200 180) 0.923
NO-Hardhat @ (300 100 400 250) 0.876
```

## 8. Model Details

### 输入输出（当前模型 `yolov8_hard_hat.rknn`）

```
输入:  images  [1, 3, 384, 640]  RGB
  mean = 0 / std = 255（已固化进计算图，宿主侧不得再做除 255）

输出 (6 个 tensor, 官方标准解耦):
  box_8    [1, 64, 48, 80]   ← box P3 (48×80, stride=8)
  score_8  [1,  2, 48, 80]   ← cls P3 (logits)
  box_16   [1, 64, 24, 40]   ← box P4 (24×40, stride=16)
  score_16 [1,  2, 24, 40]   ← cls P4 (logits)
  box_32   [1, 64, 12, 20]   ← box P5 (12×20, stride=32)
  score_32 [1,  2, 12, 20]   ← cls P5 (logits)
```

> 无 score_sum 分支。`score_*` 为**未激活 logits**（sigmoid 已移出计算图），
> 解码器按 `sigmoid(dequant(raw))` 还原置信度并换算阈值——详见
> [`CONVERSION.md`](./CONVERSION.md) 与 `src/plugin.rs` 的 `CLS_IS_LOGITS`。

### 后处理

1. 对每个检测头（P3/P4/P5），用 DFL 解码 box 坐标（`box_*` 分支）
2. 取 `score_*` 各通道最大 logit，与换算到 logit 空间的阈值比较
3. 对入选候选执行 NMS 去除重叠框
4. 置信度输出按 sigmoid 还原为概率

### RK3568 NPU 参考性能

当前模型板端实测（RK3568 EVB1 DDR4 V10，`librknnrt` 2.3.2，含 RGA 预处理与后处理）：

| 模型 | 大小 | 延迟 | FPS |
|------|------|------|-----|
| `yolov8_hard_hat.rknn`（当前） | 4.0 MB | 36.2 ms (E2E) | 27.4 |

历史模型公开参考值（口径不含后处理，与上表不完全可比）：

| 模型 | 大小 | 延迟 | FPS |
|------|------|------|-----|
| FP16 | 7.1 MB | ~83 ms | ~12 |
| INT8 hybrid | 4.2 MB | ~31 ms | ~32 |
| INT8 pure | 4.2 MB | ~29 ms | ~34 |

## 9. Quantization Analysis

连接 RK 设备后，可运行量化精度分析：

```shell
cd python

# 完整分析（需要连接 RK 设备）
python quantization_analysis.py ../model/best.onnx rk3568 ../../datasets/safetyhelmet_dataset full

# 快速对比（无需设备，x86 模拟器）
python quantization_analysis.py ../model/best.onnx rk3588 ../../datasets/safetyhelmet_dataset quick
```
