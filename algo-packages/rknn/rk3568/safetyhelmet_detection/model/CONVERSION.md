# 模型转换记录

## 当前模型

| 项 | 值 |
|---|---|
| 文件 | `yolov8_hard_hat.rknn` |
| SHA-256 | `c71e0b6d5578950754265b22a0b579d93cf0214c026f0739b606aaed693e847d` |
| 大小 | 4,186,010 字节 |
| `target_platform` | **`rk3568`** |
| Toolkit2 版本 | `2.3.2` |

> 历史模型 `best_hybrid.rknn`（9-tensor，`target_platform=rk3568`，
> SHA-256 `27c2bc7615dbeb010821aa2e53530091605274c3057be7aca5066a9e27817a8a`）
> 已由本模型取代。**其文件已不再入库**，不可作为回退路径：回退需从历史提交
> 或转换流水线重新取得，并同时将 `USE_SCORE_SUM` 置为 `true`、`CLS_IS_LOGITS` 置为 `false`。

## 转换来源

- **原始模型**: [keremberke/yolov8n-hard-hat-detection](https://huggingface.co/keremberke/yolov8n-hard-hat-detection)
- **RKNN 优化导出**: [airockchip/ultralytics_yolov8](https://github.com/airockchip/ultralytics_yolov8)
- **转换工具**: RKNN-Toolkit2 `2.3.2`

## 转换配置

```python
# python/convert.py
rknn.config(
    mean_values=[[0, 0, 0]],
    std_values=[[255, 255, 255]],
    target_platform='rk3568'
)

rknn.build(
    do_quantization=True,
    dataset='safetyhelmet_dataset/dataset.txt'
)
```

## 输入归一化合同

```
原始图像 [0, 255] (RGB)
    ↓
RGA Letterbox (等比缩放+填充)
    ↓
RKNN输入 [0, 1] (RGB，除以255)
    ↓
NPU推理 (INT8)
    ↓
检测结果
```

模型内嵌元数据（`rknn.config()` 实际生效值）实测确认：

```
输入张量: images [1, 3, 384, 640]  NCHW float32
  mean      = [0, 0, 0]
  std       = [255, 255, 255]
  range     = [0, 1]
  rgb2bgr   = False
```

**归一化落点**：`mean`/`std` 已固化进计算图（Toolkit2 配置），
宿主侧仅需提供 letterbox 后的 `[0, 255]` RGB 数据，**不得**再做除 255 处理。

## 输出结构（6-tensor 分层）

模型为官方标准解耦输出，**无 score_sum 分支**：

| idx | 名称 | shape | 说明 |
|---|---|---|---|
| 0 | `box_8` | `[1, 64, 48, 80]` | P3 DFL 16-bin × 4 边 |
| 1 | `score_8` | `[1, 2, 48, 80]` | P3 分类 logits |
| 2 | `box_16` | `[1, 64, 24, 40]` | P4 |
| 3 | `score_16` | `[1, 2, 24, 40]` | P4 |
| 4 | `box_32` | `[1, 64, 12, 20]` | P5 |
| 5 | `score_32` | `[1, 2, 12, 20]` | P5 |

## ⚠️ 分类分支为 logits（sigmoid 已移出计算图）

导出 ONNX 时 **sigmoid 被移出计算图**，`score_*` 输出的原始量化范围含负值：

| 分支 | 量化范围 | 语义 |
|---|---|---|
| `score_8` | `[-13.8221, 0.8984]` | logits |
| `score_16` | `[-11.5188, 0.8325]` | logits |
| `score_32` | `[-9.7197, -0.0835]` | logits（**全负**） |

对比：旧模型 `best_hybrid.rknn` 的 `sigmoid`/`clamp` 分支范围为 `[0, 0.75]` 等
非负区间，即 sigmoid 保留在图内。

因此算法包必须声明 `CLS_IS_LOGITS = true`（`src/plugin.rs`）。解码器据此：

1. 按 `sigmoid(dequant(raw))` 还原置信度输出；
2. 将阈值换算到 logit 空间（`ln(p / (1 - p))`）后再量化比较——
   因 sigmoid 单调，`sigmoid(x) > p ⟺ x > ln(p/(1-p))`，故 INT8 极值快速路径得以保留。

若错误声明为 `false`，`score_32` 分支将因 logits 恒小于阈值而导致 P5 尺度**零检出**，
且其余尺度的置信度系统性偏低。

## 解码配置（src/plugin.rs）

```rust
impl YoloSpec for SafetyHelmetSpec {
    const INPUT_DIM: (u32, u32) = (640, 384);
    const NUM_CLASSES: usize = 2;
    const LABELS: &'static [&'static str] = &["Hardhat", "NO-Hardhat"];
    const MODEL_PATH: &'static str = "model/yolov8_hard_hat.rknn";
    const USE_SCORE_SUM: bool = false;   // 官方 6-tensor 结构
    const CLS_IS_LOGITS: bool = true;    // sigmoid 在图外
    const DFL_BINS: usize = 16;
}
```

## 转换命令

```bash
cd python/
python3 convert.py /path/to/best.onnx rk3568 i8 ../model/yolov8_hard_hat.rknn
```

## 检测类别

- **Hardhat** (class 0) — 已佩戴安全帽
- **NO-Hardhat** (class 1) — 未佩戴安全帽

## 参考性能（RK3568）

### 实测环境

- 板卡：RK3568 EVB1 DDR4 V10（`rockchip,rk3568-evb1-ddr4-v10`）
- Kernel：5.10.226（aarch64, Debian 11 bullseye）
- `librknnrt`：2.3.2 (429f97ae6b@2025-04-09) —— 与模型转换 Toolkit2 2.3.2 同版本
- RKNPU driver：v0.9.8；librga：1.10.1；RGA driver：v1.3.4
- 测试帧：`testimage.jpg`（MockFrameBuilder 硬件帧），200 次迭代 / 20 次预热

### `yolov8_hard_hat.rknn`（当前模型，`infer_fast_path`）

| 阶段 | Avg | P50 | P99 |
|---|---|---|---|
| Preprocess (RGA) | 7.56 ms | 6.71 ms | 13.08 ms |
| Inference (NPU) | 28.52 ms | 27.73 ms | 40.50 ms |
| Postprocess | 0.13 ms | 0.12 ms | 0.18 ms |
| **End-to-end** | **36.21 ms** | 35.09 ms | 52.07 ms |
| **ABI process** | **36.48 ms (27.4 FPS)** | 35.10 ms | 50.57 ms |

60 秒满载压力测试：**1611 帧 @ 26.8 FPS（37.25 ms/帧）**，无衰减、无崩溃。

### 新旧模型板端对比

| 模型 | Inference | **Postprocess** | End-to-end | FPS | 检出数 |
|---|---|---|---|---|---|
| `yolov8_hard_hat`（6-tensor, logits） | 28.52 ms | **0.13 ms** | 36.21 ms | **27.4** | 1 |
| `best_hybrid`（9-tensor, score_sum） | 29.71 ms | **96.23 ms** | 133.83 ms | 7.5 | 1366 |

> **旧模型后处理耗时异常（~96 ms）**：该模型在 `conf=0.45` 下产生 **1366 个候选框**，
> 进入 O(n²) NMS 后约 187 万次 IoU 比较。提高阈值至 0.9 时降为 0 检出，
> 证实阈值链路正常，异常源于该权重本身在测试图上的候选密度。
> **该现象属历史模型的权重特性，非解码器缺陷**；新模型无此问题。

### 历史模型（`best_hybrid.rknn` 更早的公开参考值）

| 模型 | 大小 | 延迟 | FPS |
|------|------|------|-----|
| INT8 hybrid | 4.2 MB | ~31 ms | ~32 |
| INT8 pure | 4.2 MB | ~29 ms | ~34 |
| FP16 | 7.1 MB | ~83 ms | ~12 |

> 上表为公开参考值，与本节实测口径（含 RGA 预处理与后处理）不完全可比。
