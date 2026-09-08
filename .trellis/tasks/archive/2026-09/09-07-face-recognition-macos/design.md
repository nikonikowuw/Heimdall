# Design: macOS arm64 人脸识别算法包 (EdgeFace)

## 1. 架构概览

```
algo-packages/macos/arm64/face_recognition/
│
├── 两个 C ABI 入口 ──────────────────────────────────────────────
│   │
│   ├── av_algo_get_abi() → AvAlgoAbi 虚表                       │
│   │   └── instance_process(frame) → AV_RESULT_RECOGNITION      │
│   │       │                                                    │
│   │       ▼  子码流 CVPixelBuffer NV12                         │
│   │   ┌──────────────┐  ┌──────────────┐  ┌────────────┐      │
│   │   │ AppleCvEngine │→│ YOLOv5n-face  │→│ 后处理+NMS  │      │
│   │   │ letterbox     │  │ ANE 推理      │  │ anchor解码  │      │
│   │   │ 640×384 BGRA  │  │ FP16         │  │ 框+关键点   │      │
│   │   └──────────────┘  └──────────────┘  └─────┬──────┘      │
│   │                                             │              │
│   │                                      ┌──────▼──────┐       │
│   │                                      │ 质量门控     │       │
│   │                                      │ 姿态/大小/   │       │
│   │                                      │ 模糊/遮挡    │       │
│   │                                      └──────┬──────┘       │
│   │                                             │              │
│   │                                      emit RECOGNITION JSON │
│   │                                                            │
│   ├── av_algo_extract_face(lib, input, output)                 │
│   │       │                                                    │
│   │       ▼  JPEG bytes (主码流高清裁剪)                        │
│   │   ┌──────────────┐  ┌──────────────┐  ┌────────────────┐   │
│   │   │ JPEG 解码     │→│ YOLOv5n-face │→│ 5点仿射对齐     │   │
│   │   │ image crate  │  │ (复用模型)    │  │ 112×112 RGB    │   │
│   │   └──────────────┘  └──────────────┘  └───────┬────────┘   │
│   │                                               │            │
│   │   ┌───────────────────────────────────────────▼──────────┐ │
│   │   │ EdgeFace-s CoreML (ANE, Apache 2.0)                  │ │
│   │   │ 112×112 RGB → 512D L2-normalized embedding           │ │
│   │   └───────────────────────────────────────────┬──────────┘ │
│   │                                               │            │
│   │                                        fill AvFaceExtractOutput
│   │                                        embedding + aligned_jpeg
│   │                                                            │
├── 共享模型权重 ────────────────────────────────────────────────
│   CoreMlFaceModels (Arc)
│   ├── detector: CoreMlRunner     # YOLOv5n-face 检测模型
│   └── facenet: CoreMlRunner      # EdgeFace-s 特征提取模型 (Apache 2.0)
│
│   library_open() 时加载两个模型，所有 instance 共享
│   extract_face() 通过 lib 句柄访问同一份模型
```

## 2. 模型管线设计

### 2.1 YOLOv5n-face 检测管线

**输入**: 640×384 BGRA CVPixelBuffer (由 `AppleCvEngine.letterbox()` 产出，16:9 宽屏静态分辨率优化)

**Core ML 模型输出**: YOLOv5-face 单张量，shape `[1, N, 16]`：
```
[ cx, cy, w, h, obj_conf, cls_conf,
  lm1_x, lm1_y, lm2_x, lm2_y, lm3_x, lm3_y, lm4_x, lm4_y, lm5_x, lm5_y ]
```
- 关键点顺序: 左眼、右眼、鼻尖、左嘴角、右嘴角

**后处理**:
1. 置信度阈值过滤 (obj_conf × cls_conf)
2. NMS (IoU-based)
3. Letterbox 逆映射回帧坐标 → 归一化到 [0,1]

> **与现有 `general_detection` 的关系**: 同属 YOLO 系列输出格式，可复用 NMS 逻辑，新增 landmark 解析。

### 2.2 EdgeFace-s 特征提取管线

**输入**: 112×112 RGB 图像

**预处理**:
- 通过仿射变换将 5 关键点对齐到 ArcFace 标准模板
- ArcFace 标准参考点 (112×112):
  ```
  [38.2946, 51.6963]   # 左眼
  [73.5318, 51.6963]   # 右眼
  [56.0252, 71.7366]   # 鼻尖
  [41.5493, 92.3655]   # 左嘴角
  [70.7299, 92.3655]   # 右嘴角
  ```
- 标准归一化: `(pixel / 255.0 - 0.5) / 0.5` → [-1, 1]

**模型**: EdgeFace-s (Idiap Research Institute, Apache 2.0)
- 骨干: EdgeNeXt 轻量变体, ~1.8M 参数
- 训练: ArcFace loss on MS1MV2
- 输出: **512D** 浮点向量

**Core ML 模型输出**: 512D 浮点向量

**后处理**: L2 归一化 → 单位超球面 embedding

### 2.3 仿射对齐实现

使用最小二乘法从 5 个源关键点到 5 个目标参考点估计 2×3 仿射矩阵：

```rust
/// 从 5 个源关键点和 5 个目标参考点估计仿射变换矩阵
/// 返回 [a, b, tx, c, d, ty] 使得:
///   dst_x = a * src_x + b * src_y + tx
///   dst_y = c * src_x + d * src_y + ty
fn estimate_affine(src: &[[f32; 2]; 5], dst: &[[f32; 2]; 5]) -> [f64; 6]
```

然后对输入图像执行仿射变换采样到 112×112 输出。

**选择 CPU 仿射变换的理由**:
- 输入只有一张人脸裁剪图（通常 < 200×200），数据量极小
- 仿射变换需要非规则采样（双线性插值），不适合用 `CvEngine.resize()` 直接完成
- 这条路径是 `snapshot_readback_path`（低频），不是 `infer_fast_path`（常驻），CPU 开销完全可接受
- 实测 112×112 双线性仿射变换 CPU 耗时 < 0.1ms

## 3. C ABI 导出设计

### 3.1 `export_algo!` 标准虚表

与 `general_detection` 完全一致，通过 `export_algo!` 宏自动生成 11 个入口:

```rust
export_algo!(
    FaceRecognizer,
    algo_id: "face_recognition",
    version: "1.0.0",
    algo_type: "face_recognition",
    alarm_type_id: "face_recognize"
);
```

### 3.2 `av_algo_extract_face` 独立符号导出

在 `lib.rs` 中额外手写一个 `#[no_mangle]` 导出:

```rust
#[no_mangle]
pub unsafe extern "C" fn av_algo_extract_face(
    lib: AvAlgoLibrary,
    input: *const AvFaceExtractInput,
    output: *mut AvFaceExtractOutput,
) -> c_int
```

该函数:
1. 从 `lib` 句柄 (即 `LibraryContext`) 中获取共享模型
2. 解码输入 JPEG → RGB
3. YOLOv5n-face 检测 → 取最大人脸 → 5 关键点
4. 仿射对齐 → 112×112
5. EdgeFace-s 推理 → embedding
6. 填充 `AvFaceExtractOutput` 所有字段
7. 编码对齐人脸为 JPEG → `aligned_jpeg_data`

**关键设计**: `lib` 句柄的扩展。当前 `export_algo!` 的 `LibraryContext` 只有 `package_root` 和 `platform_id`。`av_algo_extract_face` 需要访问模型。方案:

将模型引用存储在一个模块级全局变量（`OnceLock<Arc<CoreMlFaceModels>>`），在 `library_open` 时初始化，`library_close` 时清除。`av_algo_extract_face` 通过全局变量访问模型，不依赖 `LibraryContext` 内部结构。

```rust
static SHARED_MODELS: OnceLock<Arc<CoreMlFaceModels>> = OnceLock::new();
```

这比侵入式修改 `export_algo!` 宏的 `LibraryContext` 更安全，避免改动公共基础设施。

## 4. 质量门控设计

在 `process()` 路径的检测结果上附加质量评估，零额外模型开销:

```rust
struct FaceQuality {
    score: f32,       // 综合质量分 [0, 1]
    yaw: f32,         // 偏航角估计 (度)
    pitch: f32,       // 俯仰角估计 (度)  
    blur: f32,        // 模糊度 [0, 1]
    face_size: u32,   // 像素宽度
}
```

| 维度 | 计算方式 | 复杂度 |
|---|---|---|
| 姿态角 (yaw) | 5 关键点几何: 鼻尖偏移 / 眼间距 | O(1) 浮点运算 |
| 姿态角 (pitch) | 5 关键点几何: 鼻尖纵向偏移 | O(1) 浮点运算 |
| 人脸大小 | 检测框宽度 × 原图宽度 | O(1) |
| 模糊度 | 关键点置信度均值作代理（YOLOv5n-face 无独立模糊度输出时） | O(1) |
| 遮挡 | 关键点 score < 阈值的比例 | O(5) |
| 综合分 | 以上维度加权组合 | O(1) |

注意: 精确的模糊度估计需要 Laplacian 方差，但那需要对人脸 ROI 做像素级运算。在 `process()` 路径（`infer_fast_path`）上应避免额外像素读取。先用关键点置信度做代理，后续如有需要可在宿主 pipeline 的 readback 路径上补充。

## 5. 结果 JSON Schema

### 5.1 `process()` 输出 (AV_RESULT_RECOGNITION)

```json
{
  "event_id": "550e8400-e29b-41d4-a716-446655440000",
  "faces": [
    {
      "bbox": [0.1, 0.2, 0.15, 0.2],
      "landmarks": [[0.15, 0.25], [0.22, 0.25], [0.18, 0.32], [0.14, 0.37], [0.21, 0.37]],
      "detection_score": 0.95,
      "quality": {
        "score": 0.82,
        "yaw": 12.5,
        "pitch": -5.0,
        "blur": 0.15,
        "face_size": 96
      }
    }
  ]
}
```

### 5.2 `extract_face()` 输出

通过 `AvFaceExtractOutput` 结构体，不走 JSON。

## 6. 模型转换方案

### 6.1 YOLOv5n-face ONNX → Core ML

```python
import coremltools as ct
import onnx

model = onnx.load("yolov5n_face.onnx")
# YOLOv5n-face 输入: [1, 3, 640, 640] RGB, pixel / 255.0
mlmodel = ct.convert(
    model,
    inputs=[ct.ImageType(name="images", shape=(1, 3, 640, 640),
                         color_layout="RGB", bias=[0, 0, 0],
                         scale=1.0/255.0)],
    compute_precision=ct.precision.FLOAT16,
    compute_units=ct.ComputeUnit.ALL,
)
mlmodel.save("yolov5n_face.mlpackage")
```

### 6.2 EdgeFace-s PyTorch → ONNX → Core ML

```python
# Step 1: PyTorch → ONNX
import torch
from backbones import get_model  # github.com/otroshi/edgeface

model = get_model('edgeface_s_gamma_05')
model.load_state_dict(torch.load('edgeface_s_gamma_05.pt', map_location='cpu'))
model.eval()

dummy = torch.randn(1, 3, 112, 112)
torch.onnx.export(model, dummy, 'edgeface_s.onnx',
                  input_names=['input'], output_names=['embedding'],
                  dynamic_axes=None, opset_version=17)

# Step 2: ONNX → Core ML
import coremltools as ct
import onnx

onnx_model = onnx.load('edgeface_s.onnx')
# EdgeFace-s 输入: [1, 3, 112, 112] RGB, 归一化到 [-1, 1]
mlmodel = ct.convert(
    onnx_model,
    inputs=[ct.ImageType(name='input', shape=(1, 3, 112, 112),
                         color_layout='RGB', bias=[-1, -1, -1],
                         scale=1/127.5)],
    compute_precision=ct.precision.FLOAT16,
    compute_units=ct.ComputeUnit.ALL,
)
mlmodel.save('edgeface_s.mlpackage')
```

**注意**: EdgeFace-s 最后的 FC + BN 层如果 FP16 精度不够，可全模型 FP32:

```python
mlmodel = ct.convert(onnx_model, ..., compute_precision=ct.precision.FLOAT32)
```

## 7. 兼容性与约束

### 7.1 与 `algo-sdk` 的关系

- **完全复用**: `AlgoPlugin` trait, `export_algo!`, `SafeFrame`, `ResultEmitter`, `CvEngine`, `CvBuffer`, `NormBox`
- **新增依赖**: 无需修改 `algo-sdk`，人脸包内自行实现仿射对齐和 YOLOv5-face 后处理

### 7.2 与宿主 `infer` crate 的关系

- 宿主已有 `AV_ALGO_EXTRACT_FACE_SYMBOL` 常量和 `AvAlgoExtractFaceFn` 类型签名
- 宿主加载器 (`LoadedLib`) 目前只 lookup `av_algo_get_abi`，需要**在宿主侧扩展** lookup `av_algo_extract_face` 的能力（不在本任务范围，但预留接口兼容性）

### 7.3 Workspace Cargo.toml

新增 workspace member:
```toml
"algo-packages/macos/arm64/face_recognition"
```

## 8. 测试策略

| 测试层次 | 文件 | 验证内容 |
|---|---|---|
| 仿射对齐单元测试 | `align.rs` | 已知关键点 → 期望变换矩阵，恒等变换，边界情况 |
| 质量评估单元测试 | `quality.rs` | 正脸高分、大角度低分、极小脸拒绝 |
| YOLOv5n-face 后处理单元测试 | `detect.rs` | grid 解码、NMS、landmark 提取正确性 |
| CoreML 集成测试 | `tests/hardware_infer_test.rs` | 真机 ANE 推理，`#[ignore]` 标记 |
| 端到端自检 | `run_local` | 单帧检测 + 特征提取 + 余弦相似度验证 |
| ABI 布局断言 | workspace `c_abi_layout_tests` | `AvFaceExtractInput` 40B, `AvFaceExtractOutput` 67892B |
