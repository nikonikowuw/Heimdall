# 模型转换记录

## 转换来源

### 1. EdgeFace (人脸特征提取)
- **原始模型**: edgeface_xs_gamma_06
- **精度**: FP16

### 2. YOLOv8n-Face (人脸检测)
- **原始模型**: yolov8n-face (640x384)
- **精度**: INT8 mixed

### 3. YOLOv8n COCO 人体检测
- **模型文件**: `yolov8n-640x384-rk3568.rknn`
- **输入**: `[1,384,640,3]` RGB24 (NHWC)
- **输出**: 9 个 NCHW 张量，按尺度排列为：
  - `[1,64,48,80]` + `[1,80,48,80]` + `[1,1,48,80]`
  - `[1,64,24,40]` + `[1,80,24,40]` + `[1,1,24,40]`
  - `[1,64,12,20]` + `[1,80,12,20]` + `[1,1,12,20]`
- **后处理**: 80 类 COCO 分类分支，类别 0 为 person；box 分支使用 DFL，三尺度合并后执行 NMS
- **模型 SHA-256**: `1a4c1b705165179c063097085e09ea961532faad62ea955a5cbff13280d31fe4`
- **转换工具**: RKNN-Toolkit2 `2.3.2`，RK3568 INT8
- **来源**: `yolov8n-640x384.onnx`；原始 ONNX 的输出 shape 元数据声明错误，已修正为上述 9 个分支后重新转换
- **修正 ONNX SHA-256**: `9560ab93f618c4680c216050517875e4055398faafcdae7cfa9032492ce5da73`

## 目录结构

```
face_recognition/
├── model/
│   ├── edgeface_xs_gamma_06_rk3568_fp16.rknn
│   ├── yolov8n-640x384-rk3568.rknn
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

### YOLOv8n COCO 人体检测
```python
rknn.config(
    mean_values=[[0, 0, 0]],
    std_values=[[255, 255, 255]],
    target_platform='rk3568'
)
rknn.build(do_quantization=True, dataset='/path/to/calibration.txt')
```

人体模型的 ONNX 输出声明必须与 `640×384` 的三个检测尺度一致；不能复用仍声明为 `640×640` 的单输出元数据。
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
