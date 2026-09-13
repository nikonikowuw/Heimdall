# 模型转换与量化记录 (Model Conversion Record)

## 转换来源与上游工件

- **模型研发来源**: `/home/nikoniko/work/tentcoo/rknn_model_zoo/examples/fire-detections-yolov8`
- **微调骨干网络**: YOLOv8n 烟火检测微调模型 (`model/best.pt`)
- **RKNN 优化导出**: 基于 [airockchip/ultralytics_yolov8](https://github.com/airockchip/ultralytics_yolov8) 导出
  - 去除 Decoupled Head（解耦头），后处理卸载到 CPU
  - 去除 DFL（Distribution Focal Loss）结构，NPU 内部全卷积加速
  - 增加 `score_sum` 输出分支，加速后处理快速过滤
- **原始 ONNX 工件**: `best.onnx` (输入尺寸 640x384 16:9)
- **转换工具**: RKNN-Toolkit2 (v2.x)

## 模型工件与 SHA-256 校验链

| 工件名称 | 精度模式 | 文件大小 | SHA-256 校验和 |
|---|---|---|---|
| `best_pure.rknn` | INT8 (Pure) | 4.2 MB | `47a08bce59b09580eeed58873b24d8a2eb9c8a90df5078c558edeacea96d4ea0` |
| `best_hybrid.rknn` | INT8 (Auto Hybrid) | 4.4 MB | `dada15bfef69ac01601641780c481bcc9ccc7879c8b83aad6ad06fd25cd6a624` |
| `best_fp16.rknn` | FP16 (未量化基准) | 7.1 MB | `20cc7121cafbcbba528e2baa726a60fbe208a77d3e5d5d4fd1c9598648fd62fd` |

## 模型输入与输出契约

### 1. 输入张量

- **名称**: `images`
- **布局**: `[1, 384, 640, 3]` (NHWC / RGB uint8)
- **归一化合同**:
  - `mean_values = [[0, 0, 0]]`
  - `std_values = [[255, 255, 255]]`
  - 外部传入原生 `[0, 255]` 像素，硬件底层自动映射至 `[0.0, 1.0]`

### 2. 输出张量 (9-Tensor 多分支优化结构)

| 张量序号 | 节点名称 | 维度布局 | 语义说明 |
|---|---|---|---|
| 0 | `conv2d_47` | `[1, 64, 48, 80]` | P3 特征层 BBox (DFL 16-bin, Stride 8) |
| 1 | `sigmoid` | `[1, 2, 48, 80]` | P3 特征层分类得分 (`fire`, `smoke`) |
| 2 | `clamp` | `[1, 1, 48, 80]` | P3 特征层 `score_sum` 快筛通道 |
| 3 | `conv2d_53` | `[1, 64, 24, 40]` | P4 特征层 BBox (DFL 16-bin, Stride 16) |
| 4 | `sigmoid_1` | `[1, 2, 24, 40]` | P4 特征层分类得分 |
| 5 | `clamp_1` | `[1, 1, 24, 40]` | P4 特征层 `score_sum` 快筛通道 |
| 6 | `conv2d_59` | `[1, 64, 12, 20]` | P5 特征层 BBox (DFL 16-bin, Stride 32) |
| 7 | `sigmoid_2` | `[1, 2, 12, 20]` | P5 特征层分类得分 |
| 8 | `clamp_2` | `[1, 1, 12, 20]` | P5 特征层 `score_sum` 快筛通道 |

## 转换复现命令

```bash
cd python/
# 导出 INT8 混合精度模型 (推荐，兼顾精度与速度)
python3 convert.py ../model/best.onnx rk3568 i8 ../model/best_hybrid.rknn
```

## 参考推理性能 (Rockchip RK3568 板端单核)

| 精度模式 | 延迟 (Latency) | 帧率 (FPS) |
|---|---|---|
| **INT8 Pure** | ~28~29 ms | ~34~36 FPS |
| **INT8 Auto Hybrid** | ~31~32 ms | ~31~32 FPS |
| **FP16** | ~83 ms | ~12 FPS |
