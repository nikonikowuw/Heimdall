# 模型转换记录

## 转换来源

### 1. EdgeFace (人脸特征提取)
- **原始模型**: edgeface_xs_gamma_06
- **精度**: FP16

### 2. YOLOv8n-Face (人脸检测)
- **视频流模型**: `yolov8n-face-640x384_rk3568_mixed_face.rknn` (16:9 横屏监控优化)
- **注册底库模型**: `yolov8n-face-640x640_rk3568_mixed_face.rknn` (9:16 手机自拍与正方形寸照优化)
- **精度**: INT8 mixed (`auto_hybrid=True`)
- **校准集**: 必须同时覆盖 stride 8/16/32 三个尺度，见「量化校准集要求」
- **模型 SHA-256**:
  - 640x384: `59cc6c4ca1a15f2304141ee31748a89b2c79c8c1050d4f86685a56755950a3f6` (4676269 字节，合并校准集)
  - 640x640: `9fab3d57a042b723c497463c069db5822e433703b449955b8ee3468263ce963f` (5018925 字节，合并校准集)
  - 历史版本（仅 `face_calibration` 校准，stride 8/16 分数被截断，已弃用）:
    - 640x384: `687ce89a2f76d239cb6a9fe58aa1413bbc028228da8d49191f71edb36063ca52`
    - 640x640: `dc3ef7f9f9e61555ec1d0043d5fbf672209cd39f08296bb58900b5306d23ec53`

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
│   ├── yolov8n-face-640x640_rk3568_mixed_face.rknn
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
rknn.build(do_quantization=True, auto_hybrid=True, dataset=calibration_list)
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

# YOLOv8n-Face（dtype=mixed 即 auto_hybrid；--dataset 必填）
cd python/
python3 convert_yolov8n_face.py /path/to/yolov8n-face-640x384.onnx rk3568 mixed \
    ../model/yolov8n-face-640x384_rk3568_mixed_face.rknn --dataset /path/to/calib_merged.txt
python3 convert_yolov8n_face.py /path/to/yolov8n-face-640x640.onnx rk3568 mixed \
    ../model/yolov8n-face-640x640_rk3568_mixed_face.rknn --dataset /path/to/calib_merged.txt
```

## 量化校准集要求（重要）

分数分支对校准集的**尺度覆盖**极其敏感。`yolov8n-face` 三个检测尺度的响应分布差异很大：

- 只有近距离大脸才会在 stride 8（scale 0）产生高置信度；
- 中距人脸响应 stride 16（scale 1）；
- 远景小脸只在 stride 32（scale 2）响应。

若校准集缺少某个尺度的样本，该尺度分数分支的量化范围会被钉在低值区，推理时真实分数
（可达 0.85）在量化阶段即被截断，外部表现为**人脸置信度恒为 0.5**。

实测（RK3568 + RKNN-Toolkit2 2.3.2 + librknnrt 2.3.2，同一份 RGB888 原始输入，
RKNN 模拟器与板端运行时逐位一致）：

| 输入 | ONNX 真值 (s0/s1/s2) | `face_calibration`(14) 单独 | `COCO(20)+face(14)` 合并 |
|------|---------------------|-----------------------------|--------------------------|
| 1920x1080 监控帧（近距大脸） | 0.7347 / 0.7088 / 0.00003 | **0.5000 / 0.5000 / 0** ✗ | 0.7765 / 0.7301 / 0 ✓ |
| 640x427 远景小脸 | 0.00016 / 0.0001 / 0.869 | 0 / 0.0009 / 0.849 ✓ | 0 / 0 / 0.849 ✓ |
| 640x736 注册照（stride 16） | 0.00002 / 0.8531 / 0.0005 | **0 / 0.5000 / 0** ✗ | 0 / 0.7758 / 0 ✓ |

结论：**必须使用覆盖三个尺度的合并校准集**（`datasets/COCO/coco_subset_20.txt` 的 20 张
通用场景 + `datasets/face_calibration` 的 14 张人脸场景，合计 34 张）：

- 仅 `face_calibration`：stride 8 无覆盖，近距大脸被截断到 0.5；
- 仅 `COCO/coco_subset_20`：stride 32 无覆盖，远景小脸被截断到 0.5；
- 合并两者：三个尺度均有覆盖，所有场景与 ONNX 一致。

校准图片数据本身不在本仓库（位于 RKNN Model Zoo 的 `datasets/` 下），合并清单需按
清单文件所在目录书写有效相对路径或绝对路径。`python/yolov8n_face_dataset.txt` 中记录的
`../../../datasets/COCO/coco_subset_20.txt` 在本仓库中不存在，不要直接使用。

### 转换后必须验证

出现截断时，量化参数会呈现明显的“天花板”特征：分数分支的量化范围上限恰等于校准集
在该尺度的最大响应（例如 `scale=0.00196078` 对应上限 0.5）。验证方法：

1. 用 ONNX Runtime 对同一张输入图跑原模型，记录三个尺度的分数最大值；
2. 用 RKNN 模拟器（`rknn.inference`，无需板子）或板端 `rknn_outputs_get` 跑转换后模型；
3. 两者在同一尺度上的分数最大值偏差应小于 0.05，否则说明该尺度被截断。

模拟器与板端 `librknnrt` 对同一模型的输出逐位一致（本项目实测），因此校准/尺度问题
可以完全离线定位，无需反复上板。

### 转换产物的哈希不可复现

RKNN-Toolkit2 的输出**不是逐字节可复现**的：同一 ONNX + 同一校准集 + 同一参数多次转换，
文件大小相同但内容存在少量差异（实测 4.6 MB 文件中约 210 字节不同，应为内嵌元数据）。
因此 CONVERSION.md 中记录的 `.rknn` SHA-256 仅用于标识“当前部署的就是哪一仝文件”，
不能当作可复现构建凭据；正确性必须以「分数分支与 ONNX 的逐尺度对比」为准。

## 参考性能 (RK3568)

| 模型 | 延迟 | FPS |
|------|------|-----|
| EdgeFace FP16 | ~20 ms | ~50 |
| YOLOv8n-Face mixed | ~30 ms | ~33 |
