# 技术设计：人脸识别算法包 RK3576 迁移

## 架构总览

```
algo-packages/rknn/rk3576/face_recognition/
├── Cargo.toml              # 依赖：algo-sdk (features=["rga"]), libloading, image, serde, tracing, libc
├── manifest.json           # platform_id: "rknn-rk3576"
├── Makefile
├── config.schema.json
├── model/
│   ├── yolov8n-face-640x384_mixed.rknn
│   └── edgeface_xs_gamma_06_rk3576_fp16.rknn
├── lib/                    # 构建产物
├── src/
│   ├── lib.rs              # C ABI 导出 + 入口函数
│   ├── rknn.rs             # RKNN Runtime 绑定 + RknnSession（复用 general_detection 的模式）
│   ├── detect.rs           # YOLOv8n-face 12 张量解码（DFL + keypoint + NMS）
│   ├── align.rs            # 5 点仿射对齐（RGA 或 CPU）
│   ├── quality.rs          # 质量评估（复用 Mac 版逻辑，输入归一化 landmarks）
│   ├── postprocess.rs      # 输出格式化 + JSON emit
│   ├── config.rs           # 实例配置
│   └── plugin.rs           # AlgoPlugin 适配层
├── src/bin/
│   └── run_local.rs        # 本地测试入口
└── tests/
    └── hardware_infer_test.rs
```

## 文件级设计

### 1. `rknn.rs` — RKNN 绑定层

**策略**：从 `general_detection/src/rknn.rs` 复制并精简。保留核心能力：
- `RknnRuntime`：动态加载 librknnrt.so，符号绑定
- `RknnSession`：RAII 管理 rknn_context，支持 `infer_with_host_bytes`
- `RknnOutputsGuard`：确保输出张量释放

**删除**：`infer_with_dma_buf`（本次不做零拷贝）、`new_fallback`（不需要 CPU 回退模拟）

**修改**：
- `get_hardware_outputs` 的 `is_multi_int8` 判断改为检查 `output_attrs[0].type_` 而非 `n_out > 1`，避免 mixed precision 模型被误判
- 新增 `want_float` 参数控制是否让 runtime 自动反量化

### 2. `detect.rs` — YOLOv8n-face 解码（核心新代码）

**输入**：12 个 `RknnTensorOutput`（float32，来自 `want_float=1` 路径）

**输出**：`Vec<RawFace>`，结构体与 Mac 版保持一致：
```rust
pub struct RawFace {
    pub bbox: [f32; 4],           // [x, y, w, h] 归一化到 [0, 1]
    pub landmarks: [[f32; 2]; 5], // 归一化到 [0, 1]
    pub landmark_scores: [f32; 5], // sigmoid 后的置信度
    pub score: f32,                // 检测置信度
}
```

**内部函数**：
```rust
// DFL 解码：16-bin softmax 加权求和 → 单个偏移值
fn compute_dfl(logits: &[f32]) -> f32

// 单尺度解码：遍历 grid，解码 box + cls + kpt
fn decode_scale(
    box_tensor: &[f32],      // [64, H, W] 已展平
    score_sum: &[f32],       // [1, H, W]
    cls_tensor: &[f32],      // [1, H, W]
    kpt_tensor: &[f32],      // [15, H, W]
    grid_h: usize,
    grid_w: usize,
    stride: usize,
    conf_threshold: f32,
) -> Vec<RawFace>

// 三尺度合并 + NMS
pub fn decode_yolov8_face(
    outputs: &[RknnTensorOutput; 12],
    conf_threshold: f32,
    nms_threshold: f32,
) -> Vec<RawFace>
```

**NMS**：直接复用现有 `detect.rs` 的 `nms()` 和 `RawFace::iou()`，只需确保 `RawFace` 结构体兼容。

**坐标还原**：解码出的 box 和 landmarks 是原图像素坐标，需除以原图宽高归一化到 [0,1]。

### 3. `align.rs` — 5 点对齐

**策略**：先用 CPU 实现（`image` crate + 简单仿射变换），后续可优化为 RGA。

输入：原图 RGB + 5 个 landmarks（归一化坐标）
输出：112×12×3 RGB 字节流（送入 EdgeFace）

逻辑与 Mac 版 `align_face()` 一致：3 点仿射（左眼、右眼、鼻尖）→ 缩放 + 旋转 → 裁剪 112×112。

### 4. `lib.rs` — C ABI 入口

**保持与 Mac 版相同的 C ABI 签名**：
- `av_algo_extract_face(lib, input, output) → c_int`

**修改**：
- 移除所有 `#[cfg(target_os = "macos")]` 条件编译
- 移除 CoreML 依赖（`OwnedPixelBuffer`、`objc_send!` 等）
- `prepare_detector_input()` 改为直接 resize + letterbox（CPU），不走 CVPixelBuffer
- `shared_models()` 改为持有 `Arc<RknnRuntime>` + 两个 `RknnSession`
- `normalize_embedding()` 保留不变（已验证需要）

### 5. `plugin.rs` — 插件适配

```rust
pub struct FaceRecognizer {
    pub runtime: Arc<RknnRuntime>,
    pub detector: RknnSession,
    pub embedder: RknnSession,
    pub config: InstanceConfig,
}
```

`process()` 流程：
1. 从 `SafeFrame` 获取 RGB 数据
2. Letterbox 预处理到 640×384（CPU 或 RGA）
3. `detector.infer_with_host_bytes()` → 12 输出张量
4. `decode_yolov5_face()` → `Vec<RawFace>`
5. 质量过滤 + emit 检测结果

`av_algo_extract_face()` 流程：
1. JPEG 解码 → RGB
2. 检测 + 对齐 + 嵌入提取（完整流程）
3. L2 归一化 embedding
4. 编码对齐后 JPEG
5. 写入 `AvFaceExtractOutput`

### 6. `Cargo.toml`

```toml
[dependencies]
algo-sdk = { workspace = true, features = ["rga"] }
serde = { workspace = true }
serde_json = { workspace = true }
image = { workspace = true }
libloading = { workspace = true }
tracing = { workspace = true }
libc = { workspace = true }
uuid = { workspace = true }

[lib]
name = "face_recognition"
crate-type = ["cdylib", "rlib"]
```

## 数据流

```
输入 JPEG/Image
    ↓
[CPU] image::load_from_memory → RGB
    ↓
[CPU] prepare_detector_input → 640×384 letterbox RGB
    ↓
[RKNN NPU] detector.infer_with_host_bytes → 12 × RknnTensorOutput (float32)
    ↓
[CPU] decode_yolov8_face → Vec<RawFace> (bbox + landmarks + score)
    ↓
[CPU] quality::compute_quality → 质量过滤
    ↓ (仅 av_algo_extract_face 路径)
[CPU] align_face → 112×112 RGB
    ↓
[RKNN NPU] embedder.infer_with_host_bytes → [f32; 512]
    ↓
[CPU] normalize_embedding → L2 归一化
    ↓
[CPU] encode_aligned_jpeg → JPEG bytes
    ↓
C ABI 输出
```

## 风险与缓解

| 风险 | 影响 | 缓解 |
|------|------|------|
| DFL 解码精度与 C++ 不一致 | 检测框偏移 | 用相同输入数据对比 Rust vs C++ 输出 |
| Key point 解码公式错误 | landmarks 坐标错位 | 板子上对比 Python 参考实现 |
| INT8 量化损失导致检测率下降 | 漏检 | mixed precision 已缓解，阈值可调 |
| EdgeFace-xs 比 EdgeFace-s 精度低 | 误识率上升 | 后续可替换为更大模型，接口不变 |
| CPU letterbox 成为瓶颈 | 延迟增加 | 后续用 RGA 优化，接口不变 |
