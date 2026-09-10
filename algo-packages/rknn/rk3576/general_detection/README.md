# YOLOv8n 通用目标检测算法插件 (Rockchip RK3576 RKNN)

基于 `crates/algo-sdk` 构建的纯 Rust 通用目标检测算法包，专为 Rockchip RK3576 (Linux aarch64) 设计。
通过结合 Rockchip 官方 RGA 2D 图像硬件加速（Letterbox 等比缩放与色度转换）与 RKNN NPU 运行时，实现常驻视频流的纯设备侧零拷贝前向推理。

---

## 规格信息

- **算法标识 (`algorithm_id`)**：`general_detection`
- **版本号 (`version`)**：`1.0.0`
- **平台标识 (`platform_id`)**：`linux-rknn`
- **目标芯片 (`target_soc`)**：`rk3576`
- **告警类型 (`alarm_type_id`)**：`object_detect`
- **模型文件**：`model/yolov8n-640x384-rk3576.rknn`
  - 输入：`images` 640x384 RGB888（NCHW/NHWC，Toolkit 自动执行归一化）
  - 输出：`output0` [1, 84, 5040]（包含 4 个预测框坐标与 80 个 COCO 类别得分）
- **支持类别**：COCO 80 类通用目标（行人、车辆、常见动植物与生活用品等）

---

## 目录结构

```text
.
├── Cargo.toml               # 插件 crate 清单与依赖 (依赖 algo-sdk)
├── Makefile                 # 标准生命周期便捷构建与测试命令
├── README.md                # 算法包说明与使用文档
├── manifest.json            # 算法元数据、输入能力与资源档案
├── config.schema.json       # 参数 JSON Schema 校验契约
├── .env.example             # 本地单机调试与参数调优模板
├── testimage.jpg            # 六步安全沙箱自检样例图像 (810x1080)
├── model/
│   └── yolov8n-640x384-rk3576.rknn # Rockchip 官方优化版 RKNN 模型
├── lib/
│   └── libgeneral_detection.so     # 编译生成的 C ABI 标准动态库
└── src/                     # 源码
    ├── bin/run_local.rs     # 本地单机评测与可视化运行工具
    ├── config.rs            # 参数反序列化与类别掩码 (ClassMask)
    ├── rknn.rs              # librknnrt.so 动态加载与安全 RAII 会话 (零拷贝直通/回退)
    ├── postprocess.rs       # [1, 84, 5040] 解码、NMS 与坐标反算 (unmap_box)
    ├── plugin.rs            # AlgoPlugin / AlgoInstance 生命周期实现 (RGA加速)
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

### 2. 执行单元与集成测试（带完整输出）
```bash
make test
```
将运行后处理单元测试以及样例图像全流程推理测试，终端直观展示检出的目标类别、置信度与坐标框。

### 3. 单机运行检测并生成可视化结果
```bash
make run
```
程序将从 `testimage.jpg`（或 `.env` 中指定的图片）读取输入，执行 RGA Letterbox 转换与前向推理，终端输出美化后的目标检测结果 JSON，并在本地生成带绿色检测框的 `result.jpg`。

### 4. 运行阶段级性能基准剖析 (Benchmark)
```bash
make benchmark
```
将执行 5 轮 Warmup 预热和 100 轮循环推理，分阶段统计 **Preprocess（图像预处理）、Inference（NPU推理）、Postprocess（反算后处理）、End-to-end 与 ABI process** 的 P50/P99 延迟与 FPS 吞吐。

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
CONF_THRESH=0.45
IOU_THRESH=0.45
INPUT_IMAGE=testimage.jpg
OUTPUT_IMAGE=result.jpg
MODEL_PATH=model/yolov8n-640x384-rk3576.rknn
TARGET_CLASSES=person,car,bus,truck
LOOPS=100
WARMUP=5
DURATION=30
```
