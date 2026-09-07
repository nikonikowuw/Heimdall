# RK3576 RKNN 通用目标检测算法包开发

## Goal

为 Rockchip RK3576 平台开发高性能通用目标检测算法包（`general_detection`），基于已有的官方优化版 `yolov8n-640x384-rk3576.rknn` 模型，按照 `crates/algo-sdk` 规范和 `algo-packages/macos/arm64/general_detection` 模式，实现标准 C ABI 导出、RGA 硬件预处理、NPU 零拷贝推理接入、向量化后处理以及自测与沙箱适配。

## Requirements

1. **工程结构与命名规范**：
   - 算法包根目录置于 `algo-packages/rknn/rk3576/general_detection`。
   - `Cargo.toml` 包名命名为 `general-detection-rknn`，编译库名声明为 `general_detection`（产物为 `libgeneral_detection.so`）。
   - 加入根目录 `Cargo.toml` 的 workspace `members`。
   - 包含完整的包规范文件：`manifest.json`（`platform_id: "linux-rknn"`）、`config.schema.json`、`testimage.jpg`。

2. **RKNN 运行时动态加载 (`src/rknn.rs`)**：
   - 使用 `libloading` 动态加载 `librknnrt.so`，不使用编译期动态链接，确保在开发机和未安装特定 SDK 的环境能顺利进行 `cargo check`。
   - 定义标准的 RKNN 极薄 C ABI 接口与数据结构（`rknn_init`, `rknn_inputs_set`, `rknn_create_mem_from_fd`, `rknn_set_io_mem`, `rknn_run`, `rknn_outputs_get`, `rknn_destroy` 等）。

3. **预处理与双模自适应流水线 (`src/plugin.rs`)**：
   - 采用 `algo_sdk::cv::platforms::rockchip::RgaCvEngine` 执行硬件加速等比缩放与 Letterbox 填充（`640x384`，填充色 `[114, 114, 114]`）。
   - **双模自适应支持**：
     - 常驻硬件流：当帧携带 DMA-BUF 句柄且硬件可用时，调用 `rknn_create_mem_from_fd` 将 RGA 输出 DMA-BUF 直通 NPU，维持全链路纯设备侧零拷贝；
     - 自测试/保底流：当传入 Host 内存图像（如沙箱自测图 `testimage.jpg`）时，自动降级调用 `rknn_inputs_set` 内存复制接口。

4. **输出张量解析与向量化后处理 (`src/postprocess.rs`)**：
   - **多分支与单张量自适应解析**：
     - 打包模型 `yolov8n-640x384-rk3576.rknn` 针对 RKNN NPU 极致性能采用官方推荐的 6 分支解耦输出（`box_stride8/16/32` 与 `cls_stride8/16/32`），输出层维持 INT8（`want_float = 0`）以省去 NPU 反量化开销，并在后处理中高效完成 DFL Softmax 积分与锚点解码；
     - 同时支持单张量 FP32 `[1, 84, 5040]`（`want_float = 1`）解析，满足 `debug_cpu_fallback_path` 开发调试与标准端到端模型接入；
   - 解析坐标参数及 80 个 COCO 类别得分；
   - 依据实例配置的 `confidence_threshold` 与目标类别掩码（`ClassMask`）快筛候选框；
   - 调用 `algo_sdk::math::fast_nms` 执行类别感知非极大值抑制（基于 `iou_threshold`）；
   - 调用 `algo_sdk::math::unmap_box` 精准剔除四周黑边，将坐标还原为视频帧原始 `[0.0, 1.0]` 真实比例。

5. **标准 C ABI 导出与沙箱防御 (`src/lib.rs`)**：
   - 通过 `algo_sdk::export_algo!` 宏导出标准虚表；
   - 所有 FFI 入口被 `std::panic::catch_unwind` 隔离，严禁 panic 逃逸；
   - 满足沙箱七步安全检查规范。

## Acceptance Criteria

- [x] 算法包目录结构规范，包含 `model/yolov8n-640x384-rk3576.rknn`、`manifest.json`、`config.schema.json`、`testimage.jpg` 与 `Cargo.toml`。
- [x] 动态库在根 workspace 下顺利编译为 `libgeneral_detection.so`。
- [x] 后处理测试覆盖完整：验证 `parse_and_unmap_detections` 能够正确解析 `[1, 84, 5040]` 张量并完成 NMS 与黑边去除。
- [x] 通过 `cargo test` 验证算法包内部单元测试全绿。
- [x] 通过 `cargo clippy --all-targets -- -D warnings` 与 `cargo fmt --all -- --check` 代码门禁。
