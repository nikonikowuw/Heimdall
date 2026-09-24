# 模型转换记录

> **当前部署模型清单以 [README.md](README.md) 为准。**
> 人脸检测全链路采用 **SCRFD-2.5G BNKPS**（见 §2），人体检测采用 **YOLOv8n COCO**（见 §3），特征提取采用 **EdgeFace**（见 §1）。

## 转换来源

### 1. EdgeFace (人脸特征提取)
- **模型文件**: `edgeface_s_gamma_05_rk3568_fp16.rknn`
- **原始模型**: edgeface_s_gamma_05 (InsightFace rknn_model_zoo)
- **输入**: `[1, 112, 112, 3]` RGB24 (NHWC)
- **输出**: `[1, 512]` Float32 512 维特征向量
- **精度**: FP16 (`do_quantization=False`)
- **模型 SHA-256**: `350e4808a553494d18a27c8eb331a0aa7963549bc1362f345481ac471f914a69` (8221104 字节)
- **历史版本**: `edgeface_xs_gamma_06_rk3568_fp16.rknn` (4977461 字节，因类间间隔较窄已升级为 S 系列)

### 2. SCRFD-2.5G BNKPS（人脸检测）
- **视频流模型**: `scrfd_2.5g_bnkps_640x384_rk3568_mixed.rknn` (16:9 横屏监控优化)
- **注册底库模型**: `scrfd_2.5g_bnkps_640x640_rk3568_mixed.rknn` (9:16 手机自拍与正方形寸照优化)
- **来源**: rknn_model_zoo SCRFD-2.5G BNKPS
- **输入/输出契约与精度**: 见 [README.md](README.md) 与 `config.schema.json`；输出为 9 个张量
  （3 尺度 × [score (已 Sigmoid), bbox 4 维距离偏移, kps 10 维 5 点关键点偏移]）
- **选型优势**: SCRFD 具备优秀的微观关键点定位精度与紧框能力，且不需要复杂的逐尺度校准覆盖约束。

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

## 目录结构

```
face_recognition/
├── model/
│   ├── scrfd_2.5g_bnkps_640x384_rk3568_mixed.rknn   # 视频流人脸检测（在用）
│   ├── scrfd_2.5g_bnkps_640x640_rk3568_mixed.rknn   # 注册底库人脸检测（在用）
│   ├── edgeface_s_gamma_05_rk3568_fp16.rknn         # 特征提取（在用）
│   ├── yolov8n-640x384-rk3568.rknn                  # 人体检测（在用）
│   ├── CONVERSION.md
│   └── README.md
├── python/
│   ├── convert_edgeface.py
│   └── edgeface_dataset.txt
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

### YOLOv8n COCO 人体检测
```python
rknn.config(
    mean_values=[[0, 0, 0]],
    std_values=[[255, 255, 255]],
    target_platform='rk3568'
)
rknn.build(do_quantization=True, dataset='/path/to/calibration.txt')
```

## 参考性能 (RK3568)

| 模型 | 延迟 | FPS |
|------|------|-----|
| EdgeFace FP16 | ~20 ms | ~50 |
| SCRFD 2.5G mixed | ~24 ms | ~41 |
