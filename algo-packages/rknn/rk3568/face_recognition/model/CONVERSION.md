# 模型转换记录

## 转换来源

### 1. EdgeFace (人脸特征提取)
- **原始模型**: edgeface_xs_gamma_06
- **精度**: FP16

### 2. YOLOv8n-Face (人脸检测)
- **原始模型**: yolov8n-face (640x384)
- **精度**: INT8 mixed

## 目录结构

```
face_recognition/
├── model/
│   ├── edgeface_xs_gamma_06_rk3568_fp16.rknn
│   ├── yolov8n-face-640x384_rk3568_mixed_face.rknn
│   ├── CONVERSION.md
│   └── README.md
├── python/
│   ├── convert_edgeface.py
│   ├── convert_yolov8n_face.py
│   ├── edgeface_dataset.txt
│   └── yolov8n_face_dataset.txt
└── src/
```

## 转换配置

### EdgeFace
```python
rknn.config(
    mean_values=[[127.5, 127.5, 127.5]],
    std_values=[[127.5, 127.5, 127.5]],
    target_platform='rk3568'
)
rknn.build(do_quantization=False)  # FP16
```

### YOLOv8n-Face
```python
rknn.config(
    mean_values=[[0, 0, 0]],
    std_values=[[255, 255, 255]],
    target_platform='rk3568'
)
rknn.build(do_quantization=True, auto_hybrid=True)
```

## 输入归一化合同

### EdgeFace
```
人脸图像 [0, 255] (RGB, 112x112)
    ↓
归一化: (x - 127.5) / 127.5 → [-1, 1]
    ↓
NPU推理 (FP16)
    ↓
512D特征向量 (L2归一化)
```

### YOLOv8n-Face
```
原始图像 [0, 255] (RGB)
    ↓
RGA Letterbox (等比缩放+填充)
    ↓
RKNN输入 [0, 1] (RGB，除以255)
    ↓
NPU推理 (INT8混合精度)
    ↓
人脸框 + 5点关键点
```

## 转换命令

```bash
# EdgeFace
cd python/
python3 convert_edgeface.py /path/to/edgeface_xs_gamma_06.onnx rk3568 fp ../model/edgeface_xs_gamma_06_rk3568_fp16.rknn

# YOLOv8n-Face
cd python/
python3 convert_yolov8n_face.py /path/to/yolov8n-face-640x384.onnx rk3568 mixed ../model/yolov8n-face-640x384_rk3568_mixed_face.rknn
```

## 参考性能 (RK3568)

| 模型 | 延迟 | FPS |
|------|------|-----|
| EdgeFace FP16 | ~20 ms | ~50 |
| YOLOv8n-Face mixed | ~30 ms | ~33 |
