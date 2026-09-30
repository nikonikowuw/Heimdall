# algo-sdk 算法包开发指南

本指南帮助开发者从零创建一个符合 Heimdall 规范的算法包。以 RK3568 RKNN 平台的烟火检测为例，覆盖完整开发流程。

## 前置条件

- Rust toolchain（stable）
- 目标平台交叉编译工具链（如 RK3568 的 `aarch64-linux-gnu-gcc`）
- 算法模型文件（`.rknn` / `.onnx` / `.pt`）

## 快速开始：一键脚手架创建

Heimdall 提供了快速脚手架命令，可直接生成包含完整配置隔离、测试用例与本地评测的标准算法包工程：

```bash
# 为 RK3568 创建吸烟检测算法包
make algo-new PLATFORM=rk3568 PKG=smoking_detection ALARM=ALARM_SMOKING

# 支持平台: rk3568, rk3576, rk3588, macos
```

命令会自动将算法包注册到对应平台的 Cargo workspace 并生成就绪工程。

## 目录结构

得益于 `algo-sdk` 提供的声明式宏与沉淀的运行时抽象，算法包无需再手写近千行重复的 `rknn.rs` 绑定代码，核心仅需 3 个源码文件：

```
algo-packages/rknn/rk3568/my-algorithm/
├── Cargo.toml              # 包定义与依赖
├── manifest.json           # 算法包元数据（宿主加载时校验）
├── config.schema.json      # 配置 JSON Schema（供前端动态表单渲染）
├── .env                    # 包私有环境变量覆盖模板
├── model/
│   ├── model.rknn          # RKNN 模型文件
│   └── labels.txt          # 类别标签（可选）
├── lib/
│   └── librknnrt.so        # RKNN 运行时库（目标平台可选私有覆盖）
├── testimage.jpg           # 自检测试图片
├── src/
│   ├── lib.rs              # C ABI 统一导出入口 (export_algo!)
│   ├── plugin.rs           # AlgoPlugin 核心逻辑（使用 algo_sdk::rknn / cv）
│   ├── config.rs           # 配置定义与三级优先级宏 (algo_config!)
│   └── bin/
│       └── run_local.rs    # 本地极简评测工具 (LocalPluginRunner)
└── tests/
    └── ...
```

## Step 1: Cargo.toml

```toml
[package]
name = "my-algorithm-rk3568-rknn"
version = "1.0.0"
edition = "2021"

[lib]
crate-type = ["cdylib", "rlib"]

[[bin]]
name = "run_local"
path = "src/bin/run_local.rs"

[dependencies]
algo-sdk = { path = "../../../../crates/algo-sdk" }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
tracing = "0.1"
```

关键点：
- `crate-type = ["cdylib", "rlib"]`：`cdylib` 供宿主动态链接加载，`rlib` 供集成测试和 `run_local` 使用
- `algo-sdk` 已内置 `rknn` 运行时动态加载、DMA-BUF 零拷贝管理、三级配置优先级宏与本地测试 Runner，无需额外引入 `libloading` 或 `libc`。

## Step 2: manifest.json

```json
{
  "manifest_version": 1,
  "algorithm_id": "my_algorithm",
  "version": "1.0.0",
  "name": "My Algorithm (RK3568 RKNN)",
  "description": "算法功能描述",
  "algorithm_type": "object_detection",
  "alarm_type_id": "my_alarm",
  "platform_id": "linux-rknn",
  "min_adapter_version": "1.0.0",
  "runtime_constraints": {
    "target_soc": "rk3568"
  },
  "resource_profile": {
    "min_free_memory_mb": 128,
    "fps_tiers": [
      { "fps": 5, "units": 50 },
      { "fps": 15, "units": 150 },
      { "fps": 30, "units": 300 }
    ]
  },
  "self_test": {
    "timeout_ms": 10000,
    "input_mode": "test_image"
  }
}
```

必填字段：`manifest_version`、`algorithm_id`、`version`、`name`、`algorithm_type`、`alarm_type_id`、`platform_id`。

## Step 3: config.rs

使用 `algo-sdk` 提供的 `algo_config!` 声明式宏，只需声明字段及其默认值表达式，即可自动生成：
1. 结构体定义与 `explicit_fields: HashSet<String>` 显式字段跟踪；
2. `serde::Deserialize` 自动标记宿主显式传参；
3. `Default` 默认值注入；
4. `apply_env(&mut self, env: &PackageEnv)` 自动三级优先级阶梯覆盖（宿主配置 > `.env` > 默认值）。

```rust
use algo_sdk::algo_config;
pub use algo_sdk::env::PackageEnv;

pub const DEFAULT_CONFIDENCE: f32 = 0.25;
pub const DEFAULT_IOU: f32 = 0.45;

fn default_target_classes() -> Vec<String> {
    vec!["fire".to_string(), "smoke".to_string()]
}

algo_config! {
    /// 实例运行时配置
    #[derive(Debug, Clone, PartialEq)]
    pub struct InstanceConfig {
        pub confidence_threshold: f32 = DEFAULT_CONFIDENCE,
        pub iou_threshold: f32 = DEFAULT_IOU,
        /// 监控的目标类别
        pub target_classes: Vec<String> = default_target_classes(),
        /// 自定义告警标签（覆盖模型原始类别名）
        pub custom_alarm_label: Option<String> = None,
    }
}
```

## Step 4: plugin.rs（核心）

```rust
use algo_sdk::cv::platforms::rockchip::RgaCvEngine;
use algo_sdk::cv::postprocess::{
    parse_yolov8_int8, Yolov8ParseContext, Yolov8RknnConfig,
};
use algo_sdk::emitter::ResultEmitter;
use algo_sdk::error::AlgoError;
use algo_sdk::frame::SafeFrame;
use algo_sdk::plugin::{AlgoPlugin, InitContext};
use algo_sdk::rknn::{RknnSession, RknnSessionOptions, RKNN_NPU_CORE_0};
use crate::config::InstanceConfig;

pub struct MyDetector {
    pub session: RknnSession,
    pub cv_engine: RgaCvEngine,
    pub config: InstanceConfig,
}

impl AlgoPlugin for MyDetector {
    type Config = InstanceConfig;

    fn init(ctx: &InitContext<'_>, mut config: Self::Config) -> Result<Self, AlgoError> {
        // 1. 加载并应用包私有 .env 配置（支持三级优先级隔离）
        let env = ctx.load_env();
        config.apply_env(&env);

        // 2. 加载 RKNN 模型（由 algo_sdk 统一接管物理运行时、多核掩码调度与 CPU 模拟回退）
        let model_path = ctx.package_root.join("model/my_model.rknn");
        let session = RknnSession::open_or_fallback(
            ctx.package_root,
            &model_path,
            RknnSessionOptions::with_core_mask(RKNN_NPU_CORE_0),
        )?;

        // 3. 初始化 RGA 预处理引擎
        let cv_engine = RgaCvEngine::new();

        Ok(Self { session, cv_engine, config })
    }

    fn process(
        &mut self,
        frame: SafeFrame<'_>,
        emitter: &mut ResultEmitter<'_>,
    ) -> Result<(), AlgoError> {
        // 1. RGA 硬件 Letterbox 预处理
        let (buf, mode) = self.cv_engine.letterbox(&frame, 640, 384, [0, 0, 0])?;
        let orig_w = frame.width();
        let orig_h = frame.height();

        // 2. NPU 零拷贝推理 + INT8 后处理
        if let Some(fd) = buf.as_dma_buf_fd() {
            let size = 640 * 384 * 3;
            self.session.infer_with_dma_buf(fd, size, |net_out| {
                let boxes = parse_yolov8_int8(
                    &Yolov8ParseContext {
                        branches: net_out.branches,
                        config: &Yolov8RknnConfig {
                            model_input_w: 640.0,
                            model_input_h: 384.0,
                            dfl_bins: 16,
                            num_classes: 2,
                            use_score_sum: true,
                        },
                        conf_threshold: self.config.confidence_threshold,
                        iou_threshold: self.config.iou_threshold,
                        labels: &["fire", "smoke"],
                        label_fn: None,
                        mode: &mode,
                        orig_w,
                        orig_h,
                    },
                );
                emitter.emit_detection_boxes(&boxes)
            })?;
        }

        Ok(())
    }

    fn flush(&mut self, _emitter: &mut ResultEmitter<'_>) -> Result<(), AlgoError> {
        Ok(())
    }
}
```

### 关键 API 说明

#### `AlgoPlugin` trait

| 方法 | 必填 | 说明 |
|------|------|------|
| `init(ctx, config)` | 是 | 初始化：加载模型、创建推理会话、初始化预处理引擎 |
| `process(frame, emitter)` | 是 | 逐帧处理：预处理 → 推理 → 后处理 → 发射结果 |
| `flush(emitter)` | 否 | 冲刷内部缓冲区（如跟踪器），停止/销毁前调用 |
| `update_config(config)` | 否 | 动态更新运行时配置 |
| `set_rules(rules)` | 否 | 更新空间布防规则（ROI 多边形、绊线等） |

#### `SafeFrame`

从 C ABI `AvFrameDesc` 构造的安全只读视图，生命周期绑定到当前调用栈帧。

| 方法 | 返回 | 说明 |
|------|------|------|
| `width() / height()` | `u32` | 有效像素宽高 |
| `handle_view()` | `FrameHandleView` | 解包底层句柄：`DmaBuf{fd}` / `ApplePixelBuffer{ptr}` / `Host{data}` |
| `stride(plane)` | `i32` | 指定平面的行跨距 |
| `pts_ns()` | `i64` | 帧 PTS（纳秒） |

#### `ResultEmitter`

将检测结果序列化为 JSON 并回调宿主。

| 方法 | 说明 |
|------|------|
| `emit_detections(&[NormBox])` | 发射告警检测结果，自动附全景抓拍请求 |
| `emit_recognition_json(&[u8])` | 发射识别结果 JSON |
| `emit_self_test(count)` | 自检合格信号 |

#### `cv::postprocess` 后处理工具库

**通用原语：**
- `dequant_i8(val, zp, scale)` → `f32`：INT8 反量化
- `quant_f32(val, zp, scale)` → `i8`：INT8 量化
- `decode_dfl(slice, out)`：DFL softmax 加权求和解码

**YOLOv8 RKNN 解析器：**

```rust
use algo_sdk::cv::postprocess::{
    parse_yolov8_int8, Yolov8ParseContext, Yolov8RknnConfig, RknnTensorOutput,
};

let config = Yolov8RknnConfig {
    model_input_w: 640.0,   // 模型输入宽度
    model_input_h: 384.0,   // 模型输入高度
    dfl_bins: 16,           // DFL bins 数（YOLOv8 标准为 16）
    num_classes: 2,         // 类别数
    use_score_sum: true,    // true=9-tensor（含 score_sum 快筛），false=6-tensor
};

let ctx = Yolov8ParseContext {
    branches: &rknn_outputs,        // &[RknnTensorOutput] 从 RKNN 推理获取
    config: &config,
    conf_threshold: 0.25,           // 置信度阈值
    iou_threshold: 0.45,            // NMS IoU 阈值
    labels: &["fire", "smoke"],     // 类别标签表
    label_fn: None,                 // 自定义标签覆盖函数（可选）
    mode: &preprocess_mode,         // Letterbox / Resize 模式
    orig_w: 1920,                   // 原始帧宽度
    orig_h: 1080,                   // 原始帧高度
};

let boxes: Vec<NormBox> = parse_yolov8_int8(&ctx);
```

返回的 `NormBox` 坐标已归一化到 `[0.0, 1.0]`，并通过 `unmap_box` 完成 Letterbox 坐标反算。

#### `CvEngine` 预处理引擎

| 方法 | 说明 |
|------|------|
| `letterbox(frame, dst_w, dst_h, fill)` | 等比缩放 + 居中填充，返回 `(CvBuffer, PreprocessMode)` |
| `resize(frame, dst_w, dst_h)` | 直接缩放 |

`CvBuffer` 支持：
- `as_dma_buf_fd()` → `Option<i32>`：DMA-BUF 文件描述符（零拷贝路径）
- `as_host_bytes()` → `Option<&[u8]>`：Host 内存视图（回退路径）

## Step 5: lib.rs

```rust
pub mod config;
pub mod plugin;

use algo_sdk::export_algo;
use plugin::MyDetector;

export_algo!(
    MyDetector,
    algo_id: "my_algorithm",
    version: "1.0.0",
    algo_type: "detector",
    alarm_type_id: "my_alarm"
);
```

`export_algo!` 宏自动展开标准 C ABI 虚拟方法表、11 个外部 C 符号与 Panic 隔离墙。

## Step 6: 测试与本地评测

借助 `algo_sdk::testing::{LocalPluginRunner, MockFrameBuilder}`，本地单元测试与性能基准评测只需几行代码：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use algo_sdk::testing::{LocalPluginRunner, MockFrameBuilder};

    #[test]
    fn test_plugin_init_and_process() {
        let package_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let ctx = InitContext::new(
            package_root,
            "rk3568-rknn",
            "test",
            false,
        );

        let mut detector = MyDetector::init(&ctx, InstanceConfig::default()).unwrap();

        let frame = MockFrameBuilder::new()
            .dimensions(1920, 1080)
            .to_nv12(16)
            .build();

        let (elapsed_ms, _detections) =
            LocalPluginRunner::run_once(&mut detector, frame.as_safe_frame()).unwrap();
        assert!(elapsed_ms >= 0.0);
    }
}
```

## Step 7: 本地基准测试二进制 (`src/bin/run_local.rs`)

```rust
use std::path::Path;
use algo_sdk::error::AlgoError;
use algo_sdk::plugin::{AlgoPlugin, InitContext};
use algo_sdk::testing::{LocalPluginRunner, MockFrameBuilder};
use my_algorithm_rk3568_rknn::config::InstanceConfig;
use my_algorithm_rk3568_rknn::plugin::MyDetector;

fn main() -> Result<(), AlgoError> {
    let package_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ctx = InitContext::new(
        package_root,
        "rk3568-rknn",
        "local-debug",
        false,
    );
    let mut detector = MyDetector::init(&ctx, InstanceConfig::default())?;
    let frame = MockFrameBuilder::new().dimensions(1920, 1080).to_nv12(16).build();

    let (stats, _) = LocalPluginRunner::benchmark(&mut detector, frame.as_safe_frame(), 100)?;
    println!("{stats}");

    Ok(())
}
```

## 完整检查清单

| 检查项 | 命令 |
|--------|------|
| 格式化 | `cargo fmt --all` |
| Lint | `cargo clippy --all-targets -- -D warnings` |
| 测试 | `cargo test -p my-algorithm-rk3568-rknn` |
| 构建 | `cargo build --release` |
| 自检 | 宿主加载时自动执行六步沙箱物理自检 |

## 常见问题

### Q: 如何支持多平台？

每个平台一份算法包（如 `rk3568/` 和 `rk3576/`），共享 `src/` 代码但模型文件和 `librknnrt.so` 版本不同。RKNN runtime 代码可直接复用现有 `rknn.rs`，只需修改核心掩码常量。

### Q: 如何添加时序确认（多帧防抖）？

在 `plugin.rs` 中维护 `VecDeque<FrameResult>` 滑动窗口，`process()` 中逐帧推入并在窗口满后做投票判定。参考 `fire_smoke_detection` 的 `temporal_verifier.rs`。

### Q: 如何添加空间 ROI 过滤？

实现 `set_rules(&[AvRule])` 方法，将多边形坐标缓存到实例状态。在 `process()` 中对每个候选框做点在多边形内判定。`AvRule.points` 为归一化 `[0.0, 1.0]` 坐标。

### Q: debug_cpu_fallback_path 是什么？

当目标设备无 `librknnrt.so` 时，`RknnSession::new_fallback()` 创建模拟会话，返回构造的假输出数据。用于开发机测试后处理链路和 UI 集成，不代表真实推理精度。
