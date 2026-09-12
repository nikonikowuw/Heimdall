# algo-sdk

Heimdall 独立 Rust 算法包开发套件。零依赖主工程业务 crate，支持在独立仓库闭环开发与测试。

## 模块结构

```
algo_sdk
├── c_abi          C ABI 类型定义与常量（AvFrameDesc / AvRule / AvAlgoResult 等）
├── plugin         AlgoPlugin trait 与 InitContext
├── macros         export_algo! 宏（C ABI 虚表生成与 panic 隔离）
├── frame          SafeFrame 安全帧视图与 ABI 校验
├── emitter        ResultEmitter 结果发射器（JSON 序列化 + 图片请求）
├── error          AlgoError 统一错误类型
├── model          ModelWeights / InferenceSession / SharedWeights 多核心调度
├── math           NormBox / IoU / NMS / Letterbox 坐标反算
├── testing        MockFrame / MockEmitter / MockSession 测试脚手架
└── cv             视觉预处理硬件抽象层
    ├── engine             CvEngine trait（letterbox / resize 统一门面）
    ├── buffer             CvBuffer 统一内存容器（Host / DMA-BUF / CVPixelBuffer）
    ├── layout             Letterbox 布局计算
    ├── platforms          平台引擎实现
    │   ├── rockchip       RGA 2D 硬件加速 + DMA-BUF 池
    │   ├── apple          vImage / CVPixelBuffer 加速
    │   └── cpu            CPU 回退引擎
    └── postprocess        模型推理输出后处理工具库
        ├── quantize       INT8 量化 / 反量化（通用）
        ├── dfl            DFL 解码（YOLOv8 系列通用）
        └── yolov8_rknn    YOLOv8 RKNN 多分支 INT8 解析器
```

## 快速开始

```rust
use algo_sdk::prelude::*;

// 实现 AlgoPlugin trait
struct MyDetector { /* ... */ }

impl AlgoPlugin for MyDetector {
    type Config = MyConfig;

    fn init(ctx: &InitContext<'_>, config: MyConfig) -> Result<Self, AlgoError> {
        // 加载模型、初始化推理会话
        Ok(Self { /* ... */ })
    }

    fn process(&mut self, frame: SafeFrame<'_>, emitter: &mut ResultEmitter<'_>) -> Result<(), AlgoError> {
        // 1. 预处理
        let (buf, mode) = cv::letterbox(&frame, 640, 384, [0, 0, 0])?;
        // 2. 推理 + 后处理
        // 3. 发射结果
        emitter.emit_detections(&boxes)?;
        Ok(())
    }
}

// 导出 C ABI
export_algo!(MyDetector, algo_id: "my_detector", version: "1.0.0",
    algo_type: "object_detection", alarm_type_id: "object_detect");
```

## 后处理工具库 (`cv::postprocess`)

提供与具体模型无关的量化、解码原语，以及特定模型架构的组合解析器。算法包无需自行实现通用后处理逻辑。

### 原语模块

| 模块 | 函数 | 说明 |
|------|------|------|
| `quantize` | `dequant_i8(val, zp, scale)` | INT8 反量化：`(val - zp) * scale` |
| `quantize` | `quant_f32(val, zp, scale)` | INT8 量化：`val / scale + zp` |
| `dfl` | `decode_dfl(slice, out)` | DFL softmax 加权求和解码，支持任意 bin 数 |

### 模型解析器

| 解析器 | 输入 | 说明 |
|--------|------|------|
| `yolov8_rknn::parse_yolov8_int8` | RKNN 多分支 INT8 输出 | 参数驱动 9-tensor（含 score_sum 快筛）/ 6-tensor（无 score_sum）两种路径 |

```rust
use algo_sdk::cv::postprocess::{parse_yolov8_int8, Yolov8ParseContext, Yolov8RknnConfig};

let config = Yolov8RknnConfig {
    model_input_w: 640.0,
    model_input_h: 384.0,
    dfl_bins: 16,
    num_classes: 2,       // fire / smoke
    use_score_sum: true,  // 9-tensor 优化版
};

let ctx = Yolov8ParseContext {
    branches: &rknn_outputs,
    config: &config,
    conf_threshold: 0.25,
    iou_threshold: 0.45,
    labels: &["fire", "smoke"],
    label_fn: None,
    mode: &preprocess_mode,
    orig_w: 1920,
    orig_h: 1080,
};

let boxes = parse_yolov8_int8(&ctx);
```

### 扩展新模型

在 `cv/postprocess/` 下新增文件，组合现有原语或实现新的解码逻辑：

```
postprocess/
├── mod.rs           统一 re-export
├── quantize.rs      量化原语
├── dfl.rs           DFL 解码
├── yolov8_rknn.rs   YOLOv8 RKNN 解析器
└── rt_detr.rs       ← 新增：RT-DETR 解析器（示例）
```

## 特性开关

| Feature | 说明 |
|---------|------|
| `testing-image` | 启用 `image` 依赖，提供 `MockFrameBuilder::from_image_hardware` |
| `testing-hardware` | 等于 `testing-image`，标记硬件依赖测试 |
| `rga` | 启用 Rockchip RGA 2D 硬件加速引擎（依赖 `tracing`） |

## 依赖策略

- 生产 `cdylib` 插件零冗余体积：所有测试/图像依赖均为 `optional`
- `tracing` 仅 `rga` feature 引入；无 `rga` 时完全无日志框架依赖
- 无 Tokio / async / 业务 crate 依赖，算法包可在独立仓库编译测试
