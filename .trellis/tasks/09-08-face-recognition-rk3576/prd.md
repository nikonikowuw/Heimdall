# PRD: 人脸识别算法包迁移到 RK3576

## 背景

当前 `algo-packages/macos/arm64/face_recognition` 是基于 macOS CoreML/ANE 的人脸识别算法包 MVP，使用 YOLOv5n-face 做检测、EdgeFace-s 做特征提取。需要迁移到 RK3576 NPU 平台，使用 Rockchip 推荐的模型和 RKNN Runtime。

## 迁移范围

**源包**: `algo-packages/macos/arm64/face_recognition`
**目标包**: `algo-packages/rknn/rk3576/face_recognition`（新建）
**参考实现**: `algo-packages/rknn/rk3576/general_detection`（现有 RKNN 包的架构模式）

## 模型选型

| 用途 | Mac 版 | RK3576 版 | 来源 |
|------|--------|-----------|------|
| 人脸检测 | YOLOv5n-face (.mlpackage) | **YOLOv8n-face** (`yolov8n-face-640x384_mixed.rknn`) | rknn_model_zoo/examples/yolov8n-face |
| 特征提取 | EdgeFace-s (.mlpackage) | **EdgeFace-xs** (`edgeface_xs_gamma_06_rk3576_fp16.rknn`) | rknn_model_zoo/examples/edgeface |

选择依据：
- 检测模型：YOLOv8n-face 是 RKNN 优化推荐，640×384 mixed precision 在 RK3576 上 ~7.7ms（112 FPS），支持 5 点关键点回归
- 嵌入模型：edgeface_xs_gamma_06 是 RKNN zoo 中已适配 RK3576 的 EdgeFace 变体，112×112 输入，512D 输出

## 已验证的技术事实

以下结论已在 RK3576 板子上实测确认：

1. **EdgeFace 输出未归一化**：L2 norm ≈ 0.273，必须在存储前做 L2 归一化，否则跨平台人脸比对失效
2. **YOLOv8n-face 输出 12 个 NCHW 张量**：3 尺度 × 4 分支（box[64ch DFL], score_sum[1ch], cls[1ch], kpt[15ch]）
3. **Kpt tensor 布局**：NCHW 格式，15 通道 = 5 landmarks × 3 (x_offset, y_offset, visibility_conf)
4. **所有张量在 Python API 下返回 float32**（mixed precision 模型自动反量化），但 Rust 通过 `RknnSession` 走 `want_float=0` 返回 INT8 + scale/zp

## 核心差异与实现要点

### 1. 检测解码（最大工作量）

Mac 版 YOLOv5n-face 输出单张量 `[1, N, 16]`，直接 `chunks_exact(16)` 遍历。
RKNN 版 YOLOv8n-face 输出 12 张量，需要全新解码流程：

- **DFL 解码**：box 的 64 通道是 16-bin 分布 logits，需 `softmax → 加权求和` 还原 `l, t, r, b`
- **关键点解码**：15 通道 = 5×(x_offset, y_offset, conf)，公式：
  - `kpt_x = (grid_x + offset_x * 2.0 - 0.5) * stride`
  - `kpt_y = (grid_y + offset_y * 2.0 - 0.5) * stride`
  - `kpt_conf = sigmoid(score_k)`
- **三尺度遍历**：P3(stride8, 48×80)、P4(stride16, 24×40)、P5(stride32, 12×20)
- **NMS**：类别无关，IoU 阈值 0.45，可复用现有 `nms()` + `RawFace::iou()`

参考 C++ 实现：`rknn_model_zoo/examples/yolov8n-face/cpp/postprocess.cc`
注意：C++ 参考代码**未实现 keypoint 解码**（只输出 bbox + score），我们需要自己实现。

### 2. 嵌入模型

- 输入：112×112 RGB NHWC UINT8
- 输出：512 维 float32 向量（未归一化）
- 需保留 `normalize_embedding()` 做 L2 归一化
- `cosine_similarity()` 可保持不变（归一化后直接点积）

### 3. 平台抽象层替换

| 模块 | Mac 版 | RK3576 版 |
|------|--------|-----------|
| 推理引擎 | CoreML (Objective-C FFI) | RKNN Runtime (librknnrt.so 动态加载) |
| 模型加载 | `.mlpackage` → `MLModel` | `.rknn` → `rknn_init` |
| 预处理 | CVPixelBuffer + Accelerate | RGA 硬件 letterbox（复用 algo-sdk） |
| 像素格式 | BGRA CVPixelBuffer | RGB NHWC UINT8 |
| 对齐变换 | CPU image crate 仿射 | RGA rotate+scale 或 CPU fallback |

### 4. 插件架构

`FaceRecognizer` 持有：
- `Arc<RknnRuntime>`（共享 librknnrt.so 句柄）
- 两个 `RknnSession`（detector + embedder），在 `open_shared_models` 钩子中加载
- 检测和嵌入串行执行，NPU 核可共享（`rknn_set_core_mask(3)` 双核）

## 不做的事

- 不修改 Mac 版代码（RK3576 版是独立的 algo-packages 目录）
- 不实现 DMA-BUF 零拷贝路径（先用 CPU 预处理 + `infer_with_host_bytes` 跑通，后续优化）
- 不做模型量化格式选择（固定使用 mixed precision fp16 模型）
- 不修改 algo-sdk 或通用检测包

## 验收标准

1. **编译通过**：`cargo build -p face-recognition-rknn` 无错误
2. **单元测试通过**：`cargo test -p face-recognition-rknn` 纯逻辑测试全绿
3. **硬件集成测试**：在 RK3576 板子上运行 `cargo test -p face-recognition-rknn -- --ignored`，成功完成检测 + 嵌入提取
4. **精度验证**：用 rknn_model_zoo 的测试图片，检测框和 landmarks 与 C++ 参考实现对齐；嵌入 cosine similarity 与 Python 参考一致
5. **C ABI 兼容**：`av_algo_extract_face()` 接口签名和输出格式与 Mac 版一致（embedding 512D 已归一化，bbox/landmarks 归一化到 [0,1]）
6. **manifest.json** 正确声明 `platform_id: "rknn-rk3576"` 和模型路径
