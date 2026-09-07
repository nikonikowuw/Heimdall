# RK3576 RKNN 通用目标检测算法包设计方案

## 1. 架构与边界

本算法包位于 `algo-packages/rknn/rk3576/general_detection`，通过实现 `crates/algo-sdk` 暴露的标准插件 trait `AlgoPlugin` 与 `AlgoInstance`，编译为独立的 `cdylib` 动态链接库（`libgeneral_detection.so`）。

宿主主程序（`crates/infer`）通过标准 C ABI（`AvPluginVTable` 与 `AvPluginInstanceVTable`）加载和调度该动态库，算法包与宿主主业务 crate 完全解耦，对 `infer` / `types` / `pipeline` / `db` 零依赖。

```
                    ┌────────────────────────────┐
                    │      Host Process          │
                    │   (crates/infer Loader)    │
                    └─────────────┬──────────────┘
                                  │ C ABI (export_algo!)
                                  ▼
    ┌───────────────────────────────────────────────────────────────┐
    │     algo-packages/rknn/rk3576/general_detection               │
    │                                                               │
    │  ┌────────────────┐     ┌──────────────────────────────────┐  │
    │  │  lib.rs /      │     │  config.rs                       │  │
    │  │  plugin.rs     │◄────┤  (COCO classes, ClassMask, Conf) │  │
    │  └───────┬────────┘     └──────────────────────────────────┘  │
    │          │                                                    │
    │          ├───────────────────────┬────────────────────────┐   │
    │          ▼                       ▼                        ▼   │
    │  ┌───────────────┐      ┌─────────────────┐      ┌────────────┤
    │  │ RgaCvEngine   │      │ rknn.rs         │      │postprocess │
    │  │ (Hardware     │      │ (libloading     │      │(NMS & unmap│
    │  │  Letterbox)   │      │  librknnrt.so)  │      │ boxes)     │
    │  └───────────────┘      └─────────────────┘      └────────────┘
    └───────────────────────────────────────────────────────────────┘
```

## 2. 模块细化设计

### 2.1 目录组织
```text
algo-packages/rknn/rk3576/general_detection/
├── Cargo.toml
├── manifest.json
├── config.schema.json
├── testimage.jpg
├── model/
│   └── yolov8n-640x384-rk3576.rknn
└── src/
    ├── lib.rs          # 导出宏 export_algo!
    ├── plugin.rs       # AlgoPlugin / AlgoInstance 核心流转
    ├── config.rs       # 运行时配置、COCO 类别表与 ClassMask 掩码
    ├── rknn.rs         # RKNN 运行时动态加载与 RAII 会话封装
    └── postprocess.rs  # NCHW 5040 张量解码、阈值过滤、fast_nms 与 unmap_box
```

### 2.2 RKNN 动态绑定与运行时 (`src/rknn.rs`)
- 使用 `libloading::Library` 加载 `librknnrt.so`。候选路径包括：
  1. 算法包本地 `lib/librknnrt.so`；
  2. 系统动态库目录 `/usr/lib/librknnrt.so`、`/usr/lib64/librknnrt.so`、`/usr/local/lib/librknnrt.so`；
  3. `LD_LIBRARY_PATH` 默认解析。
- 定义核心 C ABI 结构体：
  - `rknn_context`
  - `rknn_input`（支持 `RKNN_TENSOR_UINT8`，`RKNN_TENSOR_NHWC`）
  - `rknn_output`（支持 `want_float = 1`）
  - `rknn_tensor_mem`（用于 `rknn_create_mem_from_fd` 与 `rknn_set_io_mem` DMA-BUF 零拷贝直通）
- 封装安全句柄 `RknnSession`，实现 `Drop` 自动释放会话，防止显存泄漏。

### 2.3 预处理与推理执行流水线 (`src/plugin.rs`)
1. **预处理**：
   - 构造单实例 `RgaCvEngine`，调用 `letterbox(&frame, 640, 384, [114, 114, 114])`；
   - 输出 `(CvBuffer, PreprocessMode)`。
2. **推理输入分发**：
   - 若 `buf.as_dma_buf_fd()` 为 `Some(fd)` 且硬件支持零拷贝：
     - 调用 `rknn_create_mem_from_fd` 创建显存句柄，并通过 `rknn_set_io_mem` 绑定至输入 0；
     - 执行 `rknn_run`。
   - 若为 Host 内存（`buf.as_host_bytes()` 存在）：
     - 组装 `rknn_input` 结构体，调用 `rknn_inputs_set` 内存复制输入；
     - 执行 `rknn_run`。
3. **输出提取**：
   - 依据模型输出分支数自适应配置：
     - 若为多分支（如官方推荐的 6 分支解耦模型），配置 `want_float = 0`，保留底层量化 INT8 切片，省去 NPU 设备端反量化开销；
     - 若为单张量（如 `debug_cpu_fallback_path` 或端到端反量化模型），配置 `want_float = 1`，提取 FP32 浮点切片；
   - 通过 `RknnOutputsGuard` RAII 守卫严格保证在作用域结束或发生 Panic 时自动调用 `rknn_outputs_release`。

### 2.4 后处理与非极大值抑制 (`src/postprocess.rs`)
- 支持两种输出张量形态的统一解码：
  1. **多分支解耦 INT8 张量 (`parse_multi_branch_int8`)**：
     - 对应 3 个尺度（stride 8, 16, 32）的 box 分支与 cls 分支；
     - 在 CPU 端基于输入置信度阈值反向快筛量化 INT8 得分（`quant_f32` 防御处理）；
     - 对候选特征点执行 DFL 积分计算 `[x1, y1, x2, y2]` 物理边界框并归一化；
  2. **单浮点切片 `[1, 84, 5040]` (`parse_single_float`)**：
     - 遍历 5040 个锚点，按通道偏移提取 `cx, cy, w, h` 及 80 个类别得分并过滤候选框。
- 候选框汇总后统一调用 `algo_sdk::math::fast_nms(&mut boxes, config.iou_threshold)` 抑制重叠框。
- 对每个保留的框调用 `algo_sdk::math::unmap_box` 扣除 Letterbox 上下/左右黑边，还原至原始视频分辨率真实比例。

## 3. 兼容性与安全防御

1. **Panic 隔离**：FFI 入口处强制 `catch_unwind`，发生异常统一转换为 `AV_STATUS_ERR_INTERNAL`。
2. **资源泄漏防御**：输入 DMA-BUF 和输出内存均受 Rust RAII guard 严格保护，在异常退出分支也能安全释放。
3. **坐标有效性保证**：强制校验 `is_finite()`，杜绝 NaN 或 Inf 导致的计算异常。
