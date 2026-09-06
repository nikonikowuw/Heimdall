# YOLO26n 通用目标检测算法插件 (macOS Apple Silicon CoreML)

基于 `crates/algo-sdk` 使用纯 Rust 重构构建的通用目标检测插件，专为 Apple Silicon (macOS arm64) 设计。
通过调用 Apple 原生 CoreVideo / vImage 硬件 Letterbox 预处理与 CoreML 原生框架，实现显存零拷贝前向推理。

---

## 规格信息

- **算法标识 (`algorithm_id`)**：`general_detection`
- **版本号 (`version`)**：`1.0.0`
- **平台标识 (`platform_id`)**：`macos-arm64-coreml`
- **告警类型 (`alarm_type_id`)**：`object_detect`
- **模型文件**：`model/yolo26n.mlpackage` (输入 `image` 640x384 BGRA，输出 `var_911` Float16 [1, 300, 6])
- **支持类别**：COCO 80 类通用目标（行人、车辆、常见动植物与生活用品等）

---

## 目录结构

```text
.
├── Cargo.toml               # 插件 crate 清单与依赖 (依赖 algo-sdk)
├── Makefile                 # 标准生命周期便捷命令
├── README.md                # 算法包说明与使用文档
├── manifest.json            # 算法元数据、输入能力与默认参数
├── config.schema.json       # 参数 JSON Schema 校验契约
├── .env.example             # 本地单机调试与参数调优模板
├── testimage.jpg            # 自检与单机验证样例图像 (810x1080)
├── model/
│   └── yolo26n.mlpackage    # Apple CoreML 原始模型包
├── lib/
│   └── libgeneral_detection.dylib # 编译构建生成的 C ABI 标准动态库
└── src/                     # 纯 Rust 源码
    ├── bin/run_local.rs     # 本地单机评测与可视化运行工具
    ├── config.rs            # 参数反序列化与类别掩码 (ClassMask)
    ├── coreml.rs            # CoreML 原生 Objective-C Runtime 绑定 (ANE加速)
    ├── postprocess.rs       # 张量解析与坐标反算 (unmap_box)
    ├── plugin.rs            # AlgoPlugin / AlgoInstance 生命周期实现
    └── lib.rs               # C ABI 虚表与 panic 隔离导出宏 (export_algo!)
```

---

## 快速上手与本地调试

### 1. 编译动态库
```bash
make
# 或编译高度优化的 Release 库并自动更新至 lib/
make build
```

### 2. 执行纯净单元测试
```bash
make test
```

### 3. 单机运行检测并生成可视化结果
```bash
make run
```
程序将从 `testimage.jpg`（或 `.env` 中指定的图片）读取输入，执行硬件 Letterbox 转换与 ANE 前向推理，终端输出美化后的目标检测结果 JSON，并在本地生成带绿色检测框的 `result.jpg`。

### 4. 运行阶段级性能基准剖析 (Benchmark)
```bash
make benchmark
```
将执行 5 轮 Warmup 预热和 100 轮循环推理，分阶段统计 **Preprocess（硬件预处理）、Inference（CoreML ANE）、Postprocess（反算后处理）、End-to-end 与 ABI process** 的 P50/P99 延迟与 FPS 吞吐。

### 5. 满载持续压力与稳定性测试 (Stress Test)
```bash
# 默认满载运行 30 秒连续推理
make stress

# 支持通过 DURATION 参数指定自定义时长（例如满载压测 60 秒）
make stress DURATION=60
```
程序将在指定时间内不间断向算法实例推送真实帧数据，每 5 秒打印一次实时吞吐心跳，验证算法在高负载连续推理下是否存在内存泄漏、句柄泄漏或崩溃。

### 6. 一键生成标准化分发归档
```bash
make package
```
打包生成符合平台沙箱规范的 `general_detection.tar.gz`（自动包含 `README.md`、元数据清单、模型与动态库，并自动剔除源码与临时调试文件）。

---

## 本地调试配置 (`.env`)

可复制 `.env.example` 为 `.env` 进行自定义参数覆盖测试：

```env
CONF_THRESH=0.5
IOU_THRESH=0.45
INPUT_IMAGE=testimage.jpg
OUTPUT_IMAGE=result.jpg
MODEL_PATH=model/yolo26n.mlpackage
TARGET_CLASSES=person,bus,car
LOOPS=100
WARMUP=5
```
