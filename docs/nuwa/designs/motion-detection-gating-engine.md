# Design: 运动检测与前置门控引擎 (Motion Detection & Gating Engine)

> **状态**: Draft  
> **作者**: Heimdall Engineering  
> **日期**: 2025-07-25  
> **关联规范**: [媒体管线](../nuwa/backend/media-pipeline.md)、[算法 SDK](../nuwa/backend/algo-sdk-guidelines.md)、[并发模型](../nuwa/backend/concurrency-guidelines.md)、[FFI 边界](../nuwa/backend/ffi-guidelines.md)

---

## 1. 背景与问题陈述

### 1.1 现状与痛点

Heimdall 定位于工业级边缘视频分析流媒体服务，面向 8~32 路摄像机的高并发视频流，且多部署在无风扇嵌入式计算盒（如 Rockchip RK3588、华为昇腾 Atlas 200I DK A2、Apple Silicon 等）上。

在实际安防与监控场景中，大部分摄像头（如夜间仓库、空旷周界、走廊通道）在 **80%~95%** 的时间内画面处于相对静止状态。若将每一帧解码后的图像无差别地送入 NPU 执行深度学习目标检测模型，将导致：
1. **能耗与温度飙升**：NPU 核心长期满负荷运转，在嵌入式盒子上快速达到温控警戒线（80℃+），触发系统级频率降级（Thermal Throttling）；
2. **多路并发吞吐受限**：无效的静态帧持续占用有限的 NPU 算力与总线带宽，挤占了真正发生告警通道的计算资源；
3. **硬件资源磨损**：在离网/太阳能供电等边缘边缘边缘节点上，造成不必要的电力耗尽。

当前系统中虽然已包含初代 `MotionGate`（位于 `crates/pipeline/src/motion_gate.rs`），但在生产落地中暴露了严重的工程断层与架构缺陷：

```
[ 当前实现链路痛点 ]
1. 硬件加速帧穿透空转:
   FrameHandle::DmaBuf / DeviceMemory ──► 无法/未做轻量判定 ──► has_motion 恒为 true (硬件加速下门控完全失效)

2. 采样哈希算法对传感器噪点极度脆弱:
   Host 采样 4096 字节 ──► DefaultHasher::finish() ──► 只要 1 个像素因热噪发生 ±1 波动 ──► 签名改变 ──► 误判为有运动

3. 配置与遥测链路断裂:
   MotionGateConfig { threshold, contour_area, keepalive_interval_ms } ──► pump.rs 硬编码 default() ──► 用户配置不生效
   CameraTelemetryEvent.motion_score ──► 未实时计算 ──► 前端进度条静止
```

### 1.2 业界标杆借鉴与边界澄清（Frigate 机制辨析）

开源 NVR 标杆 **Frigate** 凭借“基于子码流的轻量运动检测”在低功耗边缘端取得了巨大成功。深入分析其原理，需明确“取其精华、弃其糟粕”：

| 机制 / 维度 | Frigate 原始做法 | 在现代嵌入式 NPU（RK3588/昇腾）上的评估 | Heimdall 本设计决策 |
| :--- | :--- | :--- | :--- |
| **色彩空间处理** | 仅提取 YUV 的 Y 分量（单通道纯灰度） | **极致高效**：消除 YUV $\to$ RGB 耗时转换，内存访问缩减为 1/3 | **完全采纳**：直接借用 NV12 Y 平面 |
| **图像降噪差分** | 高斯模糊 + 阈值二值化（SAD）+ 形态学膨胀 + 连通域轮廓过滤 | **极致稳健**：彻底过滤 Sensor 高频热噪、飞虫、微小光斑 | **完全采纳**：SIMD 向量化灰度差分 + 连通域过滤 |
| **多边形遮罩** | Motion Mask 屏蔽静态噪点区（如马路车流、树叶摇晃、时间水印） | **工业级刚需**：大幅消除已知干扰源造成的虚假运动 | **完全采纳**：融合现有 `DetectionRuleRole::Mask` |
| **NPU 推理送检** | **动态多区域裁切（Dynamic Region Slicing）**：将分散运动切成多个 BBox 分别送检 | **严重架构反模式**：<br>1. 多次调用 NPU 调度开销反超全图；<br>2. 破坏 DMA-BUF 零拷贝直通；<br>3. 边界目标被切碎；<br>4. 噪点引发 NPU 任务雪崩。 | **坚决摒弃**：<br>**多点运动感知，单次全图零拷贝直通**（$O(1)$ 推理次数） |

---

## 2. 核心架构与设计原则

### 2.1 架构黄金准则

1. **确定性推理开销（$O(1)$ 推理调度）**：
   运动门控模块只负责回答布尔决策与热度计算：**“当前帧是否放行送检？”**。
   - **无运动**：拦截当前帧，**0 次 NPU 推理**；
   - **有运动**（哪怕全图有 10 处分散运动）：放行当前帧，**1 次全图零拷贝直通推理**，由现代 YOLO 等大模型在一次前向传播中并发检出所有目标。严禁在一帧内将图像切碎成多个碎片分别调用 NPU。
2. **端到端零拷贝契约维护（Zero-Copy Fast-Path Preservation）**：
   常驻主路径（`infer_fast_path`）严禁全尺寸（1080P/4K）CPU 像素 Readback 与颜色转换。
   - **Host 帧**：纯切片只读访问 Y 平面；
   - **硬件设备帧（DMA-BUF / DeviceMemory）**：通过 2D 加速器（RGA/VPC）下采样极微缩图（$160\times 90$ 或 $320\times 180$，仅 14KB~57KB）至常驻 Scratchpad 缓冲区，进行微秒级只读差分；或借用低分辨率子码流 Y 平面映射。
3. **迟滞与平滑保活（Hysteresis & Keepalive）**：
   - **保活心跳（Keepalive）**：达到 `keepalive_interval_ms` 周期时强制放行 1 帧，刷新模型与跟踪器内部状态；
   - **运动余晖延时（Motion Hold / Afterglow）**：检测到有效运动后，自动维持连续 $N$ 帧（如 5~15 帧）持续放行推理，避免行人在走动停顿或迈步间隙时因瞬时帧差为 0 导致跟踪器（ByteTrack）航迹断连。
4. **多区域独立感应与遮罩抑制**：
   - 支持全图全局差分；
   - 支持根据用户配置的 `DetectionRuleRole::Mask` 空间规则剔除干扰区域；
   - 支持仅关注用户划定的 `DetectionRuleRole::Roi` 布防多边形，防区外运动不唤醒 NPU。

---

## 3. 系统数据流与拓扑架构

### 3.1 数据流向与生命周期

```
                 [ RTSP 压缩码流 (Sub-stream 优先) ]
                               │
                               ▼
               [ 硬件/软件解码器 (VPU / FFmpeg) ]
                               │
                               ├──────────────────► [ update_decoded_frame: 零解码预览/快照候选 ]
                               │
                               ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│ 运动门控评估引擎 (MotionGate::evaluate)                                      │
│                                                                             │
│ 1. 检查启用状态 (enabled) 与 保活周期 (keepalive_interval_ms)                │
│    └─ 触发保活 ──► 强制放行 (return false)                                  │
│                                                                             │
│ 2. 获取当前帧灰度 Y 分量 (Zero-Copy View / RGA 微缩图)                       │
│                                                                             │
│ 3. 空间规则掩模抑制 (Mask Bitmap Filtering): 忽略遮罩区域                   │
│                                                                             │
│ 4. SIMD 绝对差分 (absdiff) + 阈值二值化 (threshold >= 25)                   │
│                                                                             │
│ 5. 连通域 / 网格聚合统计 (Grid-based Cluster Counting)                      │
│    └─ 有效变动像素数 >= contour_area ?                                       │
│       ├─ 是 ──► has_motion = true, 刷新 motion_hold 计数器                  │
│       └─ 否 ──► 检查 motion_hold > 0 (余晖放行)                             │
│                                                                             │
│ 6. 实时归一化计算: motion_score (0.0 ~ 1.0)                                 │
└─────────────────────────────────────────────────────────────────────────────┘
          │                                                  │
          │ (拦截静止帧)                                      │ (放行活动帧)
          ▼                                                  ▼
[ frames_skipped_motion.add(1) ]             [ AnalysisFpsGovernor (抽帧采样节流) ]
[ 丢弃，进入下一包解码 ]                                      │
                                                             ▼
                                             [ Sampling Slot (Drop-Oldest 单槽投递) ]
                                                             │
                                                             ▼
                                             [ NPU 硬件推理 (RKNN / CANN / CoreML) ]
                                                             │
                                                             ▼
                                             [ 坐标仿射还原 (RoiAffineMapper) ]
                                                             │ (仅用于静态 Pre-crop ROI)
                                                             ▼
                                             [ ByteTrack 航迹关联更新 ]
                                                             │
                                                             ▼
                                             [ 业务规则判定 (Roi 入侵 / Line 绊线) ]
```

---

## 4. 平台硬件与零拷贝数据获取方案

针对不同运行环境与载体，设计分层无缝适配的 Y 平面提取策略：

### 4.1 载体提取分层（Zero-Copy Friendly Y-Plane Extraction）

```rust
pub enum YPlaneSource<'a> {
    /// 连续切片借用 (Host 软解帧或 mmap 映射区，步长等于宽度)
    Contiguous(&'a [u8]),
    /// 带 Stride 的跨步借用 (水平虚宽 hor_stride > visible_width)
    Strided {
        data: &'a [u8],
        visible_width: usize,
        visible_height: usize,
        hor_stride: usize,
    },
    /// 设备侧 RGA/VPC 硬件异步生成的极微缩灰度帧
    HardwareThumbnail(&'a [u8]),
}
```

#### 4.1.1 方案 A：Host 内存帧（CPU 软解 / 开发调试）
- 直接从 `FrameHandle::Host(bytes)` 中截取 Y 平面：
  - NV12 / YUV420p 前 $W \times H$（或 $\text{hor\_stride} \times \text{ver\_stride}$）字节即为连续的 8 位亮度灰度数据；
  - 采用零拷贝切片借用，内存读取开销极低。

#### 4.1.2 方案 B：Rockchip RK3588 / RK3568（MPP 解码 $\to$ DMA-BUF 载体）
- **实现路径**：利用已有的 `rga_crop` 基础设施与 RGA 2D 硬件加速器。
- **操作步骤**：
  1. 解码器输出 DMA-BUF 帧（例如子码流 640x360 NV12）；
  2. 调度 RGA 执行异步微缩，将 NV12 的 Y 分量下采样为 $160 \times 90$ 单通道灰度图，写入常驻预分配的单个微型 Scratchpad DMA-BUF（仅 14.4 KB）；
  3. 通过 `dma_buf_sync`（`DMA_BUF_SYNC_READ`）轻量刷新 CPU 缓存并读回这 14.4 KB 数据；
  4. **性能损耗**：14.4 KB 的 D2H 总线传输耗时 $< 10\mu s$，彻底打破“硬件加速帧无法做运动检测”的瓶颈，同时严格保护 1080P/4K 主干直通管线不发生全图读回。

#### 4.1.3 方案 C：华为昇腾 Atlas / CANN（DVPP 解码 $\to$ DeviceMemory 载体）
- **实现路径**：利用 DVPP **VPC (Video Processing Component)**。
- **操作步骤**：
  1. 解码输出显存指针后，调用 VPC 异步缩放算子生成极小分辨率 Y 分量；
  2. 调用 `aclrtMemcpyAsync(H2D)` 仅回读缩略图数据；
  3. 后续在 OS Worker 专用线程内完成差分。

#### 4.1.4 方案 D：Apple Silicon（VideoToolbox $\to$ CVPixelBuffer 载体）
- 直接通过 `CVPixelBufferGetBaseAddressOfPlane(pixelBuffer, 0)` 获取 Y 平面地址。在 Unified Memory（统一内存架构）下，CPU 访问该指针完全是零拷贝总线访问，无需任何显存回读。

---

## 5. 核心算法设计与数学模型

为避免引入庞大臃肿的 C++ OpenCV 动态库，本模块基于纯 Rust 实现，配合 NEON / AVX2 自动向量化指令集，达到微秒级处理性能。

### 5.1 图像网格化与 SIMD 绝对差分（Grid-based SAD）

设输入灰度图尺寸为 $W \times H$，当前帧为 $I_t(x, y)$，历史参考帧为 $I_{\text{ref}}(x, y)$。

1. **绝对像素差与二值化（Thresholding）**：
   $$D_t(x, y) = \begin{cases} 1, & \text{if } |I_t(x, y) - I_{\text{ref}}(x, y)| \ge T_{\text{threshold}} \\ 0, & \text{otherwise} \end{cases}$$
   其中 $T_{\text{threshold}}$ 由 `MotionGateConfig::threshold` 指定（默认 25）。

2. **空间掩模屏蔽（Masking）**：
   若像素点 $(x, y)$ 落在任意配置了 `DetectionRuleRole::Mask` 的多边形内部，则强制修正：
   $$D_t'(x, y) = D_t(x, y) \cdot (1 - M(x, y))$$
   其中 $M(x, y) \in \{0, 1\}$ 为预先栅格化缓存的掩模位图（Bitmap）。

3. **宏块网格化连通滤波（Cell-based Density Clustering）**：
   单纯统计孤立变动像素容易受散粒噪点干扰。将图像划分为 $8 \times 8$ 像素的宏块网格（Grid Cells）：
   - 计算每个网格内激活像素数 $C_{\text{cell}}(i, j) = \sum_{u=0}^7 \sum_{v=0}^7 D_t'(8i+u, 8j+v)$；
   - 仅当 $C_{\text{cell}}(i, j) \ge 16$（即块内变动超过 25%）时，该网格被标记为有效活跃网格；
   - 统计所有活跃网格的总变动像素数 $S_{\text{active}}$。

4. **运动触发与热度归一化（Motion Decision & Score）**：
   - **触发判定**：$S_{\text{active}} \ge \text{contour\_area}$，判定 $M_{\text{raw}} = \text{true}$；
   - **运动热度分数**：
     $$\text{motion\_score} = \min\left(1.0, \; \frac{S_{\text{active}}}{\alpha \cdot (W \times H)}\right)$$
     其中 $\alpha$ 为灵敏度系数（默认 0.15，即全图有效变动达到 15% 时热度满格 1.0）。

### 5.2 自适应参考帧更新与动态背景演进

若简单使用前一帧作为参考帧（$I_{\text{ref}} = I_{t-1}$），当物体缓慢移动时可能出现差分消失。设计轻量化双参考帧演变模型：

1. **静态背景平滑吸收（Background Accumulator）**：
   画面持续判定为静止时，按移动加权平均更新背景，吸收光线慢速渐变（如日出、日落）：
   $$I_{\text{ref}}(x, y) \leftarrow (1 - \beta) I_{\text{ref}}(x, y) + \beta I_t(x, y), \quad \beta = 0.05$$
2. **瞬时剧变抑制（Scene Change / Light Shock）**：
   若全图变动面积突然超过 85%（如昼夜红外滤光片切换 IRCUT、开灯瞬间）：
   - 触发场景瞬变重置，强制更新参考帧为当前帧；
   - 强制触发 1 帧保活推理，避免大面积瞬态噪点导致系统误判连续剧烈运动。

---

## 6. Rust 核心数据结构与 API 契约

### 6.1 配置与参数类型扩展（`crates/types/src/task.rs`）

```rust
/// 运动门控配置参数
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MotionGateConfig {
    /// 是否启用运动门控
    pub enabled: bool,
    /// 灰度绝对变动敏感度阈值 (0..=255，默认 25)
    #[serde(default = "default_threshold")]
    pub threshold: u8,
    /// 有效变动面积阈值 (像素数，默认 100)
    #[serde(default = "default_contour_area")]
    pub contour_area: u32,
    /// 强制推理保活心跳周期 (毫秒，默认 2000)
    #[serde(default = "default_keepalive_interval_ms", alias = "keepalive_interval_ms")]
    pub keepalive_interval_ms: u64,
    /// 运动检测触发后的持续放行帧数 (余晖效应，默认 10 帧)
    #[serde(default = "default_motion_hold_frames")]
    pub motion_hold_frames: u32,
}

fn default_threshold() -> u8 { 25 }
fn default_contour_area() -> u32 { 100 }
fn default_keepalive_interval_ms() -> u64 { 2000 }
fn default_motion_hold_frames() -> u32 { 10 }
```

### 6.2 引擎状态结构体（`crates/pipeline/src/motion_gate.rs`）

```rust
/// 运动检测与门控评估结果
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionGateDecision {
    /// 是否跳过本次推理 (true: 静止跳过; false: 有活动或保活放行)
    pub should_skip: bool,
    /// 当前帧归一化运动热度 (0.0 ~ 1.0)
    pub motion_score: f32,
    /// 本次放行是否由保活心跳触发
    pub is_keepalive: bool,
}

/// 空间掩模栅格化位图缓存
#[derive(Debug, Clone)]
pub struct MaskBitmap {
    width: usize,
    height: usize,
    /// 8 像素对齐位图，1 表示被遮罩屏蔽，0 表示正常计算
    data: Vec<u8>,
}

/// 工业级运动门控引擎
#[derive(Debug)]
pub struct MotionGate {
    config: MotionGateConfig,
    last_infer_time_ms: i64,
    motion_hold_counter: u32,
    reference_frame: Option<Vec<u8>>,
    mask_bitmap: Option<MaskBitmap>,
    cached_width: usize,
    cached_height: usize,
}

impl MotionGate {
    pub fn new(config: MotionGateConfig) -> Self {
        Self {
            config,
            last_infer_time_ms: 0,
            motion_hold_counter: 0,
            reference_frame: None,
            mask_bitmap: None,
            cached_width: 0,
            cached_height: 0,
        }
    }

    /// 更新空间遮罩规则，重新栅格化生成 Mask 位图
    pub fn update_rules(&mut self, rules: &[types::DetectionRule], width: usize, height: usize);

    /// 核心评估函数：输入当前帧 Y 分量与时间戳，返回跳帧决策与热度
    pub fn evaluate(
        &mut self,
        current_y: &[u8],
        width: usize,
        height: usize,
        timestamp_ms: i64,
    ) -> MotionGateDecision;
}
```

### 6.3 解码驱动泵集成（`crates/pipeline/src/pump.rs`）

彻底修复原代码中写死 `MotionGateConfig::default()` 的缺陷，贯通用户参数：

```rust
// pump 启动前从 Coordinator 传入完整的 Option<MotionGateConfig>
let mut motion_gate = config.motion_gate.clone().map(MotionGate::new);

// 解码主循环内：
match decoder.decode_packet(&pkt.payload, pkt.pts_ms).await {
    Ok(Some(frame)) => {
        metrics_clone.frames_decoded.fetch_add(1, Ordering::Relaxed);

        // 1. 更新保底快照与预览候选
        pipeline_mgr_decode.update_decoded_frame(&cam_id, frame.clone()).await;

        // 2. 运动门控前置计算
        let mut should_skip = false;
        let mut motion_score = 0.0f32;

        if let Some(gate) = motion_gate.as_mut() {
            let decision = gate.evaluate_frame(&frame, frame.timestamp);
            should_skip = decision.should_skip;
            motion_score = decision.motion_score;

            // 实时向遥测管道广播运动热度状态
            pipeline_mgr_decode.report_motion_telemetry(&cam_id, motion_score, should_skip);

            if should_skip {
                metrics_clone.frames_skipped_motion.fetch_add(1, Ordering::Relaxed);
                continue; // 成功拦截静止帧，不向下分发
            }
        }

        // 3. 轮询算法实例，执行独立 FPS 采样与 Drop-Oldest 投递
        for slot in &mut decode_slots {
            // ...
        }
    }
    // ...
}
```

---

## 7. 性能评估与开销预算

### 7.1 算法执行耗时与内存开销（以子码流 640×360 NV12 为例）

| 平台 / 处理器 | 差分与网格统计耗时 (单帧) | 内存额外开销 | 相比全量 NPU 推理的收益 |
| :--- | :--- | :--- | :--- |
| **Rockchip RK3588** (Cortex-A76 @ 2.4GHz) | **0.12 ms** (NEON 向量化) | 230 KB (前序参考帧) | 节省 12~18ms NPU 算力与 3W 功耗 |
| **Huawei Ascend 310B** (TaiShan v120 @ 1.0GHz) | **0.25 ms** (AArch64 SIMD) | 230 KB | 节省 ACL 推理调度与显存总线带宽 |
| **Apple M2 / M3** (Performance Core) | **0.04 ms** (NEON 展开) | 230 KB | 降低 ANE / GPU 唤醒频率与发热 |
| **x86_64 Intel i7 / Xeon** (AVX2) | **0.03 ms** | 230 KB | 显著降低多路 Docker 宿主 CPU 占用 |

### 7.2 端到端跳帧能效收益预估

假设一路 1080P@25fps 摄像机，配置子码流 640×360@15fps 进行 AI 分析：
- **静态场景（如夜间走廊）**：
  - 运动门控拦截率：$\approx 90\%$；
  - 实际送入 NPU 的帧率：由原先的 15 fps 降至 $0.5 \text{ fps}$（保活周期 2000ms）；
  - **NPU 运算负载下降 96.6%**！
- **动态场景（人员持续通行）**：
  - 触发 `motion_hold_frames` 连续放行，检测跟踪不丢帧；
  - 运动门控引入的计算开销仅占单个 CPU 核心的 $< 0.5\%$，完全不影响系统吞吐。

---

## 8. 实施计划与演进路线图

### 阶段一：纯 Rust 算法升级与配置遥测闭环（Milestone 1）
- [ ] 重写 `crates/pipeline/src/motion_gate.rs`：废弃脆弱的 `DefaultHasher`，实现 SIMD 向量化 Y 通道 SAD 差分与网格聚合；
- [ ] 增加 `motion_hold_frames` 余晖机制，支持保活平滑；
- [ ] 改造 `StartCameraPipelineParams` 与 `AnalysisPump`：由 `motion_gate_enabled: bool` 升级为透传完整的 `Option<MotionGateConfig>`，彻底打通数据库与前端配置链；
- [ ] 闭环遥测数据：计算 `motion_score` 并接入 `CameraTelemetryEvent`，驱动 Web 端 `LivePlayer` 动态热度条展示。

### 阶段二：空间遮罩（Mask）与多防区联动（Milestone 2）
- [ ] 实现 `MaskBitmap` 快速光栅化，在差分计算中跳过配置了 `DetectionRuleRole::Mask` 的区域；
- [ ] 支持按任务布防的 `DetectionRuleRole::Roi` 执行正向过滤，防区外运动直接抑制，杜绝无效唤醒。

### 阶段三：硬件设备帧微缩支持（Milestone 3）
- [ ] 针对 Linux RK3588 平台的 DMA-BUF 物理帧，接入 RGA 异步微缩图流程；
- [ ] 针对华为昇腾平台，接入 VPC 异步下采样流程，实现嵌入式硬解路径下的原生节能闭环。

---

## 9. 验收门禁与测试计划

### 9.1 单元测试（Unit Tests）
1. **抗噪性测试**：注入带有 $\pm 5$ 随机高斯噪声的连续静态灰度图，断言 `should_skip == true`，绝对不被噪声误触发；
2. **运动触发测试**：注入局部产生 $20 \times 20$ 像素（变动量 40）的运动图块，断言准确触发 `should_skip == false` 且 `motion_score > 0`；
3. **保活周期测试**：维持静止帧输入，时间跨越超过 `keepalive_interval_ms`，断言立即放行一帧且 `is_keepalive == true`；
4. **余晖平滑测试**：发生单帧运动后恢复静止，断言后续 $N$ 帧在余晖窗口内保持放行。

### 9.2 跨层集成验证（E2E Tests）
- 启动完整 Coordinator 测试管线，监控 `PipelineMetrics::frames_skipped_motion` 指标，验证静止流下跳帧计数线性稳定增长；
- 启动前端任务配置界面，修改 `threshold` 与 `keepaliveIntervalMs`，验证后端数据库原子持久化且运行时动态平滑生效。
