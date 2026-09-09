# RK3576 Face Recognition

该算法包面向 Rockchip RK3576，使用 YOLOv8n-face 做人脸检测和质量门控，使用 EdgeFace-xs 做 512 维特征提取。

## 运行契约

- 常驻 `AlgoPlugin::process` 只执行子码流检测、关键点质量评估和结果发射。
- 常驻结果中的 `embedding` 为 `null`，`is_best_shot` 为 `false`。特征提取通过 `av_algo_extract_face` 低频抓拍接口执行，避免每帧 CPU readback 和 embedding 推理。
- 在带 RGA 的 Linux/RK3576 构建中，预处理结果优先沿 DMA-BUF 进入 RKNN。Runtime 支持 imported memory 且 stride 兼容时使用 `rknn_create_mem_from_fd` / `rknn_set_io_mem`；否则进入显式 mmap 兼容路径，并执行 `DMA_BUF_IOCTL_SYNC`。
- Host 路径只用于开发机 CPU 回退或 Runtime 不支持直接 DMA-BUF 绑定的情况，不能视为设备侧纯零拷贝。
- RKNN detector、embedder context 由固定 OS worker 线程独占；请求队列容量为 2，满载时淘汰最旧请求并让被淘汰调用收到 `Timeout`，不在 Tokio worker 上执行 FFI。

## 模型契约

`manifest.json` 是模型路径、SHA-256、输入尺寸、输入布局和输出形状的唯一运行时来源。当前已校验的模型摘要：

| 模型 | 输入 | 输出 | SHA-256 |
| --- | --- | --- | --- |
| `yolov8n-face-640x384_mixed_face.rknn` | `1x3x384x640`, RGB, uint8 | 12 tensors: `48x80`, `24x40`, `12x20` 三尺度 | `14d7db34c4f79fc441db30ed703d739fc90e47767dfadc42cd7fb27d3ce4894a` |
| `edgeface_xs_gamma_06_rk3576_fp16.rknn` | `1x3x112x112`, RGB, uint8 | 512D FP32 view | `ed99d6e416fb24eb1f7a77beb8f1929159018b57da3a87a6cd7dbb8ff569d85a` |

输入的模型布局是 NCHW；Rust 提交的 packed RGB buffer 使用 RKNN `Nhwc` input descriptor，由 Runtime 负责输入格式转换和量化。输入均设置 `pass_through = 0`，因此实际量化/归一化必须与转换模型时的 `mean_values`、`std_values` 保持一致。

## 构建和测试

宿主机只需执行不依赖板端 Runtime 的检查：

```bash
cargo fmt --all
cargo check -p face-recognition-rknn --all-targets
cargo test -p face-recognition-rknn --lib
cargo clippy -p face-recognition-rknn --all-targets -- -D warnings
```

板端需要 `librknnrt.so`、RGA/DRM DMA-BUF 环境和匹配的 BSP：

```bash
cargo test -p face-recognition-rknn -- --ignored --nocapture
```

本地 runner 会加载 manifest 校验的两个模型，并通过 worker 执行检测与 embedding：

```bash
cargo run -p face-recognition-rknn --bin face_recognition_rknn_run_local -- image.jpg --loops 20
```

`aarch64-unknown-linux-gnu` 交叉检查需要额外安装 Rust target、链接器和 RKNN/RGA 目标侧库；macOS 工作站不具备硬件推理验证条件。

## 打包

`make package` 会构建 release 动态库，并把 `manifest.json`、`config.schema.json`、`README.md`、`model/` 和 `lib/` 打入 `face_recognition.tar.gz`。源代码、Cargo 元数据和本地测试工具不会进入部署包。
