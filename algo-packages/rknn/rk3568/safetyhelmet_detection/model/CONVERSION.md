# 模型转换记录

## 转换来源

- **原始模型**: [keremberke/yolov8n-hard-hat-detection](https://huggingface.co/keremberke/yolov8n-hard-hat-detection)
- **RKNN优化导出**: [airockchip/ultralytics_yolov8](https://github.com/airockchip/ultralytics_yolov8)
- **转换工具**: RKNN-Toolkit2

## 目录结构

```
safetyhelmet_detection/
├── model/
│   ├── best_hybrid.rknn        # INT8 auto_hybrid, 4.2MB
│   ├── CONVERSION.md           # 本文档
│   └── README.md
├── python/
│   ├── convert.py              # RKNN转换脚本
│   ├── dataset.txt
│   └── safetyhelmet_dataset/   # 校准图片
└── src/
```

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

## 转换命令

```bash
cd python/
python3 convert.py /path/to/best.onnx rk3568 i8 ../model/best_hybrid.rknn
```

## 检测类别

- **Hardhat** (class 0) — 已佩戴安全帽
- **NO-Hardhat** (class 1) — 未佩戴安全帽

## 参考性能 (RK3568)

| 模型 | 延迟 | FPS |
|------|------|-----|
| INT8 hybrid | ~31 ms | ~32 |
| INT8 pure | ~29 ms | ~34 |
| FP16 | ~83 ms | ~12 |
