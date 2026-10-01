# YOLOv8n 安全帽检测算法插件 (Rockchip RK3568 RKNN)

基于 `crates/algo-sdk` 构建的纯 Rust 安全帽检测算法包，专为 Rockchip RK3568 (Linux aarch64) 设计。
结合 Rockchip RGA 2D 硬件加速（Letterbox 等比缩放）与 RKNN NPU 运行时，实现常驻视频流的纯设备侧零拷贝前向推理。

---

## 规格信息

- **算法标识 (`algorithm_id`)**：`safetyhelmet_detection`
- **版本号 (`version`)**：`1.0.0`
- **平台标识 (`platform_id`)**：`linux-rknn`
- **目标芯片 (`target_soc`)**：`rk3568`
- **告警类型 (`alarm_type_id`)**：`safety_violation`
- **模型文件**：`model/yolov8_hard_hat.rknn`（INT8, 4.0 MB, `target_platform=rk3568`）
  - 输入：`images` `[1, 3, 384, 640]` RGB, `mean=0` / `std=255`（已固化进图）
  - 输出：**6 个张量**（P3/P4/P5 × {`box`, `score`}），官方标准解耦结构，无 score_sum 分支
  - ⚠️ `score_*` 分支为**未激活 logits**（sigmoid 已移出计算图），解码时由
    `CLS_IS_LOGITS = true` 按 `sigmoid` 还原并换算阈值；详见 `model/CONVERSION.md`
- **支持类别**：
  - `Hardhat` (class 0) — 已佩戴安全帽
  - `NO-Hardhat` (class 1) — 未佩戴安全帽
- **RK3568 NPU 参考性能**（板端实测）：

| 模型 | 大小 | 延迟 | FPS |
|------|------|------|-----|
| `yolov8_hard_hat.rknn`（当前, INT8） | 4.0 MB | 36.2 ms (E2E) | 27.4 |

> RK3568 EVB1 DDR4 V10 实测（`librknnrt` 2.3.2，含 RGA 预处理与后处理）。
> 完整分阶段数据见 `model/CONVERSION.md`。

---

## 目录结构

```
.
├── Cargo.toml               # 插件 crate 清单与依赖
├── Makefile                 # 标准生命周期便捷构建命令
├── README.md                # 算法包说明文档
├── manifest.json            # 算法元数据、平台与资源档案
├── config.schema.json       # 参数 JSON Schema 校验契约
├── .env.example             # 本地调试参数模板
├── model/                   # 模型文件与文档
│   ├── yolov8_hard_hat.rknn # 当前模型 (6-tensor, logits 分类头)
│   ├── CONVERSION.md        # 模型转换记录与输入输出契约
│   └── README.md            # 模型说明与导出文档
├── python/                  # 转换脚本和校准数据
│   ├── convert.py           # RKNN转换脚本
│   ├── dataset.txt          # 校准数据集列表
│   └── safetyhelmet_dataset/# 校准图片目录
├── lib/
│   └── libsafetyhelmet_detection.so  # 编译产物
└── src/
    ├── bin/run_local.rs     # 本地评测与可视化工具
    ├── config.rs            # 参数反序列化
    ├── postprocess.rs       # 6 张量解码、DFL、logits 还原、NMS
    ├── plugin.rs            # AlgoPlugin 生命周期实现
    └── lib.rs               # C ABI 虚表导出宏
```

> 历史模型 `best_hybrid.rknn` / `best_pure.rknn` / `best_fp16.rknn` 已不再入库；
> librknnrt 动态加载与 RAII 会话已上移至 `algo-sdk` 的 `runtime` 模块。

---

## 快速上手

### 1. 编译动态库
```bash
make           # 交叉编译 (Mac -> RK3568)
make host      # 本机编译 (在 RK3568 设备上)
```

### 2. 运行单元测试
```bash
make test
```

### 3. 本地运行检测
```bash
make run
```

### 4. 性能基准测试
```bash
make benchmark
```

### 5. 持续压力测试
```bash
make stress            # 默认 30 秒
make stress DURATION=60
```

### 6. 打包与部署
```bash
make package           # 生成 safetyhelmet_detection.tar.gz
make deploy            # SCP 部署到目标设备
```

---

## 本地调试配置

复制 `.env.example` 为 `.env` 进行自定义参数测试：

```env
CONF_THRESH=0.45
IOU_THRESH=0.45
INPUT_IMAGE=testimage.jpg
OUTPUT_IMAGE=result.jpg
MODEL_PATH=model/yolov8_hard_hat.rknn
LOOPS=100
WARMUP=5
DURATION=30
```

---

## 模型信息

本模型基于 [keremberke/yolov8n-hard-hat-detection](https://huggingface.co/keremberke/yolov8n-hard-hat-detection) 微调，
经 [airockchip/ultralytics_yolov8](https://github.com/airockchip/ultralytics_yolov8) 导出为 RKNN 优化格式。

**RKNN 优化特性：**
- 去除解耦头（Decoupled Head），将后处理移至 CPU
- 增加 score_sum 输出分支，加速候选框过滤
- 输入尺寸 384x640（16:9），适配常见摄像头比例
- 支持 INT8 量化，RK3568 单核 NPU 推理约 31ms
