# algo-sdk 算法包开发指南

本指南帮助开发者从零创建一个符合 Heimdall 规范的算法包。以 RK3568 RKNN 平台的烟火检测为例，覆盖完整开发流程。

## 前置条件

- Rust toolchain（stable）
- 目标平台交叉编译工具链（如 RK3568 的 `aarch64-linux-gnu-gcc`）
- 算法模型文件（`.rknn` / `.onnx` / `.pt`）

## 目录结构

```
algo-packages/rknn/rk3568/my-algorithm/
├── Cargo.toml              # 包定义与依赖
├── manifest.json           # 算法包元数据（宿主加载时校验）
├── config.schema.json      # 配置 JSON Schema（可选，前端动态表单）
├── model/
│   ├── my_model.rknn       # 模型文件
│   └── labels.txt          # 类别标签（可选）
├── lib/
│   └── librknnrt.so        # RKNN 运行时库（目标平台）
├── testimage.jpg           # 自检测试图片
├── src/
│   ├── lib.rs              # C ABI 导出入口
│   ├── plugin.rs           # AlgoPlugin 实现（核心业务逻辑）
│   ├── config.rs           # 配置定义与反序列化
│   ├── rknn.rs             # RKNN 运行时绑定（可复用现有实现）
│   └── bin/
│       └── run_local.rs    # 本地评测工具
└── tests/
    └── ...
```

## Step 1: Cargo.toml

```toml
[package]
name = "my-algorithm-rk3568-rknn"
version = "1.0.0"
edition.workspace = true
license.workspace = true

[lints]
workspace = true

[lib]
name = "my_algorithm"
crate-type = ["cdylib", "rlib"]

[[bin]]
name = "my_algorithm_run_local"
path = "src/bin/run_local.rs"

[dependencies]
algo-sdk = { workspace = true, features = ["rga", "testing-hardware"] }
serde = { workspace = true }
serde_json = { workspace = true }
tracing = { workspace = true }
libloading = { workspace = true }
libc = { workspace = true }
```

关键点：
- `crate-type = ["cdylib", "rlib"]`：`cdylib` 供宿主动态加载，`rlib` 供测试和 `run_local` 使用
- `algo-sdk` 的 `rga` feature 启用 RGA 硬件加速，`testing-hardware` 启用测试工具

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

```rust
use serde::Deserialize;

/// 实例运行时配置
#[derive(Debug, Clone, Deserialize)]
pub struct InstanceConfig {
    #[serde(default = "default_confidence")]
    pub confidence_threshold: f32,

    #[serde(default = "default_iou")]
    pub iou_threshold: f32,

    /// 监控的目标类别
    #[serde(default = "default_target_classes")]
    pub target_classes: Vec<String>,

    /// 自定义告警标签（覆盖模型原始类别名）
    #[serde(default)]
    pub custom_alarm_label: Option<String>,
}

fn default_confidence() -> f32 { 0.25 }
fn default_iou() -> f32 { 0.45 }
fn default_target_classes() -> Vec<String> {
    vec!["fire".to_string(), "smoke".to_string()]
}

impl Default for InstanceConfig {
    fn default() -> Self {
        Self {
            confidence_threshold: default_confidence(),
            iou_threshold: default_iou(),
            target_classes: default_target_classes(),
            custom_alarm_label: None,
        }
    }
}
```

要点：
- 所有字段必须有 `serde(default)` 或 `Default` 实现
- 配置通过 JSON 反序列化，前端动态表单可根据 `config.schema.json` 自动生成

## Step 4: plugin.rs（核心）

```rust
use algo_sdk::emitter::ResultEmitter;
use algo_sdk::error::AlgoError;
use algo_sdk::frame::SafeFrame;
use algo_sdk::plugin::{AlgoPlugin, InitContext};
use crate::config::InstanceConfig;

#[cfg(target_os = "linux")]
pub struct MyDetector {
    pub session: crate::rknn::RknnSession,
    pub cv_engine: algo_sdk::cv::platforms::rockchip::RgaCvEngine,
    pub config: InstanceConfig,
}

#[cfg(target_os = "linux")]
impl AlgoPlugin for MyDetector {
    type Config = InstanceConfig;

    fn init(ctx: &InitContext<'_>, config: Self::Config) -> Result<Self, AlgoError> {
        // 1. 加载 RKNN 模型
        let model_path = ctx.package_root.join("model/my_model.rknn");
        let runtime = crate::rknn::RknnRuntime::load(ctx.package_root)
            .map_err(|e| tracing::warn!(reason = ?e, "未检测到 librknnrt.so"));
        let session = match runtime {
            Ok(rt) => crate::rknn::RknnSession::new(rt, &model_path)?,
            Err(_) => crate::rknn::RknnSession::new_fallback(&model_path)?,
        };

        // 2. 初始化 RGA 预处理引擎
        let cv_engine = algo_sdk::cv::platforms::rockchip::RgaCvEngine::new();

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

        // 2. NPU 推理 + 后处理
        if let Some(fd) = buf.as_dma_buf_fd() {
            let size = 640 * 384 * 3;
            self.session.infer_with_dma_buf(fd, size, |net_out| {
                let boxes = algo_sdk::cv::postprocess::parse_yolov8_int8(
                    &algo_sdk::cv::postprocess::Yolov8ParseContext {
                        branches: net_out branches,
                        config: &algo_sdk::cv::postprocess::Yolov8RknnConfig {
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
                emitter.emit_detections(&boxes)
            })?;
        } else if let Some(host_bytes) = buf.as_host_bytes() {
            // Host 内存回退路径
            self.session.infer_with_host_bytes(host_bytes, |net_out| {
                // ... 同上
                Ok(())
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
pub mod rknn;

use algo_sdk::export_algo;
use algo_sdk::plugin::AlgoPlugin;
use plugin::MyDetector;

export_algo!(
    MyDetector,
    algo_id: "my_algorithm",
    version: "1.0.0",
    algo_type: "object_detection",
    alarm_type_id: "my_alarm"
);
```

`export_algo!` 宏生成标准 C ABI 虚表和 `av_algo_get_abi` 导出符号，自动隔离 panic。

## Step 6: 测试

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use algo_sdk::testing::{MockEmitter, MockFrameBuilder};

    #[test]
    fn test_plugin_init_and_process() {
        let config = InstanceConfig::default();
        let ctx = InitContext {
            package_root: Path::new("."),
            platform_id: "linux-rknn",
            instance_id: "test",
            is_self_test: false,
        };

        let mut detector = MyDetector::init(&ctx, config).unwrap();

        let frame = MockFrameBuilder::new()
            .dimensions(1920, 1080)
            .host_data(vec![128u8; 1920 * 1080 * 3 / 2])
            .to_nv12(16)
            .build();

        let mut emitter = MockEmitter::new();
        let safe_frame = frame.as_safe_frame();
        detector.process(safe_frame, &mut emitter).unwrap();
    }
}
```

运行测试：
```bash
cargo test -p my-algorithm-rk3568-rknn
```

## Step 7: 本地评测

```bash
# 编译
cargo build --release --target aarch64-unknown-linux-gnu

# 推送到 RK3568 板端
adb push target/aarch64-unknown-linux-gnu/release/my_algorithm_run_local /data/

# 运行
adb shell
cd /data
./my_algorithm_run_local --input test.jpg --output result.jpg --threshold 0.3
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
