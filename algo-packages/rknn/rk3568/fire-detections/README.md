# 烟火检测算法包 (Rockchip RK3568 RKNN)

基于女娲（Nuwa）架构规范与 `algo-sdk` 构建的 Rockchip RK3568 边缘端轻量级烟火识别算法插件。

## 算法概览

- **算法 ID**: `fire_smoke_detection`
- **目标硬件**: Rockchip RK3568 SoC (4×Cortex-A55 @ 2.0GHz, RKNPU2 单核 ~1.0 TOPS @ INT8)
- **图形硬件加速**: RGA2 硬件引擎（等比缩放 Letterbox 与色彩空间转换）
- **模型骨干**: YOLOv8n 微调模型，输入分辨率 `640x384` (16:9 比例)
- **检测目标**: `fire`（火焰，类别 0）与 `smoke`（烟雾，类别 1）
- **数据流路径**:
  - `infer_fast_path`: MPP 硬解 -> NV12 DMA-BUF -> RGA2 Letterbox -> RGB DMA-BUF -> NPU 单核零拷贝推理 -> CPU 9-branch fast NMS -> 时序命中确认
  - `debug_cpu_fallback_path`: x86/无 NPU 环境下的单浮点回退推理仿真

## 模型来源与转换溯源

- **研发源目录**: `/home/nikoniko/work/tentcoo/rknn_model_zoo/examples/fire-detections-yolov8`
- **ONNX 工件**: `model/best.onnx` (去 Decoupled Head，去 DFL，9-tensor 多分支优化导出)
- **量化配置**:
  - `best_pure.rknn`: 纯 INT8 量化 (~28ms, ~35 FPS)
  - `best_hybrid.rknn`: INT8/FP16 混合精度量化 (~31ms, ~32 FPS)
- **详细量化说明**: 参见 [model/CONVERSION.md](model/CONVERSION.md)

## 目录结构

```
fire-detections/
├── Cargo.toml                  # Cargo 包配置 (cdylib + rlib)
├── Makefile                    # 交叉编译与打包工程脚本
├── README.md                   # 本说明文档
├── manifest.json               # 女娲规范算法描述元数据
├── config.schema.json          # 前端动态配置表单 JSON Schema
├── .env.example                # 局部私有环境变量配置模版
├── model/                      # 模型工件目录
│   ├── CONVERSION.md           # 模型转换记录与 SHA-256
│   ├── best_pure.rknn          # 纯 INT8 模型
│   ├── best_hybrid.rknn        # 混合精度模型
│   └── fire_smoke_2_labels_list.txt
├── python/                     # 模型转换与量化校准脚本
│   ├── convert.py
│   └── fire-dataset/           # 量化校准图片数据集
├── src/                        # 算法插件源码
│   ├── bin/run_local.rs        # 单机评测与 Benchmark 工具
│   ├── config.rs               # 实例参数配置与三级优先级管理
│   ├── lib.rs                  # C ABI 导出入口
│   ├── plugin.rs               # 插件主循环实现
│   ├── rknn.rs                 # librknnrt 动态符号绑定与 DMA-BUF 缓存
│   └── temporal_verifier.rs    # M-out-of-N 时序防误报验证器
└── tests/                      # 集成测试
    └── hardware_infer_test.rs
```

## 配置项说明

可通过运行时任务配置或算法包私有 `.env` 进行覆盖（优先级：任务显式配置 > 私有 `.env` > 代码默认值）：

| 参数名 | 类型 | 默认值 | 描述 |
|---|---|---|---|
| `confidence_threshold` | float | `0.25` | 检测框置信度过滤阈值 |
| `iou_threshold` | float | `0.45` | NMS 交并比阈值 |
| `target_classes` | array | `["fire", "smoke"]` | 监控类别，可单独设为 `["fire"]` 或 `["smoke"]` |
| `custom_alarm_label` | string | `""` | 自定义业务标签覆盖 |
| `confirm_window` | int | `5` | 时序确认滑动窗口大小（帧） |
| `confirm_threshold` | int | `3` | 窗口内需命中的最小帧数 (M-out-of-N) |
| `temporal_variance_threshold` | float | `50.0` | 像素亮度方差过滤阈值（仅在真实读回时有效） |

## 编译与测试

```bash
# 1. 运行本地单元测试
make test

# 2. 本机编译动态库
make host

# 3. 运行本地单图推理
make run

# 4. 运行 100 轮性能基准测试
make benchmark

# 5. 针对 RK3568 交叉编译并打包
make package
```
