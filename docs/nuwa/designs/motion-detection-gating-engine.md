# Design: 运动检测与前置门控引擎 (Motion Detection & Gating Engine)

> **状态**: Milestone 1/2 已实现（Host 帧与 Apple Unified Memory）；Milestone 3 的 Rockchip DMA-BUF 路径已接入（RGA 缩略图，320×180，评估在专用 OS Worker 内执行）；昇腾 DeviceMemory（VPC + `aclrtMemcpyAsync`）待接入，未接入前保守放行且计入绕过计数。  
> **代码落地**: `crates/pipeline/src/motion_gate/`（`mod.rs` 差分判定与保活状态机、`mask.rs` 规则光栅化、`y_plane.rs` 载体适配与平台 FFI、`telemetry.rs` 绕过计数与限速日志）、`crates/pipeline/src/motion_gate_worker.rs`（每路一个的专用 OS 线程）、`crates/media/src/motion_thumb.rs`、`crates/types/src/task.rs`

---

## 1. 设计原则

1. **确定性 $O(1)$ 推理调度**：
   门控引擎仅输出放行/拦截决策。无运动拦截（0 次 NPU 推理）；有运动放行（1 次全图零拷贝直通推理，由目标检测模型并发检出所有目标）。严禁动态多区域切图多次送检。
2. **端到端零拷贝直通**：
   常驻推理主路径严禁 CPU 全图回读与色彩转换。纯切片借用 NV12 Y 分量；硬件加速帧通过 2D 加速器（RGA/VPC）在设备侧降采样出**定长缩略图**，只回读缩略图 Y 平面（当前 $320 \times 180$，57.6KB/帧），回读量与原分辨率无关。这是常驻推理路径上唯一被允许的 CPU 回读（快照证据路径的 D2H 属另一条路径）。
   - **尺寸取值**：16:9 源固定 $320 \times 180$，与主流 NVR 的检测分辨率惯例一致（如 Frigate 的 `detect` 默认 320×180）；其余画幅按同高换算并夹紧到 $[64, 640]$。该尺度下既有足够的面积平均抑制传感器热噪，每帧回读量又恒定。
   - **执行位置**：RGA 调用属平台 FFI，必须在**每路一个的专用 OS Worker**内执行，不得直接跑在 Tokio Worker 上（见 [并发模型](../backend/concurrency-guidelines.md#执行归属)）。
3. **阈值口径与源分辨率解耦**：
   `threshold`（灰度差）与 `contour_area`（有效变动像素数）都作用在**评估栅格**上。直接在原生分辨率上差分会让 `contour_area` 的语义随画幅漂移（1080p 全图静止噪声即可轻易越过默认 100 像素），因此有硬件缩略图链路的载体固定尺寸、各画幅按同高换算。

   评估栅格按载体划分，同一份配置在不同载体上的物理含义不同，**现场标定必须按载体进行**：

   | 载体 | 评估栅格 | 口径 |
   | :--- | :--- | :--- |
   | `DmaBuf`（Rockchip） | 硬件缩略图 $320 \times 180$（上限 640，下限 64） | 与源分辨率完全解耦 |
   | `Host` / `CVPixelBuffer` | 源帧可见尺寸（无缩略图链路） | 随源分辨率放大，`contour_area` 需按实际画幅调大 |
   | `DeviceMemory`（Ascend） | 待接入 | 保守放行 |

4. **空间规则遮罩抑制**：支持结合 `DetectionRuleRole::Mask` 剔除干扰区域（如树叶、路面）；支持 `DetectionRuleRole::Roi` 防区过滤。
   - Mask / ROI 位图在**评估栅格**上惰性光栅化（规则不绑定源分辨率），因此防区几何在小图上会被量化：亚像素规则（面积不足一个栅格像素）保守标记为 1 个像素，绝不凭空消失。
   - 防区覆盖像素数低于 `contour_area` 时，有效变动像素数永远达不到阈值，该路除保活外永不推理：这是可推导的死区，必须在光栅化时 `WARN` 一次（字段 `roi_pixels` / `contour_area`）。
   - `Precrop`（特写取景框）是**画幅**规则，只决定送模画面，**不得**并入掩码构建：否则「只画取景框」的任务会静默变成「只算框内运动」。见 [媒体管线](../backend/media-pipeline.md#特写取景预裁剪pre-crop-roi)。
5. **迟滞保活与余晖延时**：
   - **保活心跳（Keepalive）**：达到 `keepaliveIntervalMs`（默认 2000ms）强制放行 1 帧，刷新模型与跟踪器内部状态；
   - **运动余晖（Afterglow）**：检测到有效运动后维持连续 $N$ 帧（默认 10 帧）放行推理，防止目标微小停顿导致 ByteTrack 航迹断连。

---

## 2. 判定流水线拓扑

```text
              Tokio Worker (解码/编排)           专用门控 Worker (每路 1 个 OS 线程)
  [ 解码输出帧 FrameRef ] ──(有界通道, 容量 1, FrameRef 为 Arc 借用)──► [ 获取 Y 分量 (切片借用 / 硬件极微缩图) ]
                                                                              │
                                                                              ▼
                                                                [ 空间掩模屏蔽 (Mask Bitmap) ]
                                                                              │
                                                                              ▼
                                                                [ SIMD 绝对差分 (absdiff >= threshold) ]
                                                                              │
                                                                              ▼
                                                                [ 8x8 宏块网格连通过滤 (Cell >= 16) ]
                                                                              │
                                                                              ▼
                                                      [ 有效变动面积 >= contourArea ? ]
                                                        ├─ 是 ──► has_motion = true, 刷新 motion_hold
                                                        └─ 否 ──► 检查 motion_hold > 0 (余晖延时)
                                                                              │
                                                                              ▼
                                                          [ 决策: 放行送检 / 拦截跳帧 ] ──► 回信至 Tokio Worker
```

**线程归属**：RGA / VPC 降采样与 DMA-BUF 回读都是平台 FFI（库函数内部可 `poll` 等待 DMA-BUF 就绪，单次阻塞可达百毫秒级），严禁跑在 Tokio Worker 上。每路摄像机在管线启动时创建一个专用 OS 线程，硬件上下文在该线程内常驻；门控对象、规则与参考帧均为线程独占状态，无需加锁。请求通道容量固定为 1（调用方逐帧串行提交，容量只是保险），提交方带超时：Worker 异常退出、卡死或崩断时，本路门控保守放行全部帧、计入绕过计数并 `ERROR` 告警一次，绝不阻塞视频接入与解码。停机也遵守同一边界：关闭请求通道后由瞬态看护线程做有界 `join`（无期限 `join` 被禁止），调用方（可能正在 Tokio Worker 上析构）从不等待。

---

## 3. 算法数学模型

### 3.1 差分与网格连通滤波
1. **绝对差分与二值化**：
   $$D_t(x, y) = \begin{cases} 1, & \text{if } |I_t(x, y) - I_{\text{ref}}(x, y)| \ge T_{\text{threshold}} \\ 0, & \text{otherwise} \end{cases}$$
   其中 $T_{\text{threshold}}$ 默认 25。
2. **空间掩模屏蔽**：若 $(x, y)$ 落在 Mask 规则内，强制 $D_t'(x, y) = 0$。
3. **8×8 宏块密度滤波**：计算块内变动像素 $C_{\text{cell}}(i, j) = \sum_{u=0}^7 \sum_{v=0}^7 D_t'(8i+u, 8j+v)$；仅当 $C_{\text{cell}} \ge 16$ 时记为活跃网格。
4. **热度归一化**：
   $$\text{motion\_score} = \min\left(1.0, \; \frac{S_{\text{active}}}{\alpha \cdot (W \times H)}\right)$$
   $\alpha$ 默认 0.15，全图变动 15% 达到满分 1.0。

### 3.2 动态背景演进与瞬变抑制
- **背景平滑更新**：持续静止时，按指数移动加权更新参考帧：$I_{\text{ref}} \leftarrow 0.95 I_{\text{ref}} + 0.05 I_t$。
- **场景剧变重置**：全图变动面积超过 85% 时（如开关灯、IRCUT 切换），强制重置参考帧为当前帧并放行 1 帧保活，避免瞬态噪点引发雪崩。
- **优先级声明**：瞬变抑制先于掩模判定生效——被 Mask 覆盖的帧仍然会贡献全图变动比例；全图重绘（如开关灯）下即使整幅都在 Mask 内也会重置基准并放行 1 帧。

### 3.3 规则光栅化与死区
Mask / ROI 规则以归一化坐标（$[0,1]$）声明，在**当前评估栅格**上光栅化为位图，不做几何插值：
- 亚像素规则：若一个规则包住的像素中心全部落在外（多边形不足一个栅格像素），保守标记其 AABB 中心像素，保证“画了防区就一定生效”。
- 死区告警：`declared_roi` 为真且光栅化后覆盖像素数 $< \text{contour\_area}$ 时，该路除保活外永远不会推理，启动/重载规则时 `WARN` 一次。

---

## 4. 跨平台 Y 平面提取策略

实现集中在 `crates/pipeline/src/motion_gate/y_plane.rs`：平台 FFI（CoreVideo / RGA / DMA-BUF 栅障）只允许出现在该模块，上层差分判定只看一段连续 Y 平面切片。

| 平台 | 载体 | Y 平面提取实现 | 性能开销 |
| :--- | :--- | :--- | :--- |
| **Host 内存帧** | `FrameHandle::Host` | 零拷贝切片借用 NV12 前 $W \times H$ 字节 | $< 1\mu s$ |
| **Rockchip** | `FrameHandle::DmaBuf` | RGA `scale_sync` 硬件降采样 → 常驻缩略图 DMA-BUF（$320 \times 180$ NV12），仅 `dma_buf_sync` 回读 Y 平面 57.6KB（与源分辨率无关）；评估在专用 OS Worker 内 | RGA 单次降采样 $< 1ms$，Y 回读 $\approx 57.6\text{KB}$/帧 |
| **Ascend** | `FrameHandle::DeviceMemory` | DVPP VPC 异步缩放生成极微缩图，`aclrtMemcpyAsync(D2H)` 回读 | VPC $< 1ms$，D2H $< 15\mu s$ |
| **Apple Silicon** | `FrameHandle::CVPixelBuffer` | `CVPixelBufferGetBaseAddressOfPlane(buf, 0)`（统一内存直接读） | 0 拷贝，$< 1\mu s$ |

---

## 5. 配置契约与遥测数据

### 5.1 配置结构 (`crates/types/src/task.rs`)
```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MotionGateConfig {
    pub enabled: bool,
    #[serde(default = "default_threshold")]
    pub threshold: u8,               // 默认 25
    #[serde(default = "default_contour_area")]
    pub contour_area: u32,           // 默认 100 像素（作用在“评估栅格”像素上，不是源分辨率像素；见 §1.3 口径表）
    #[serde(default = "default_keepalive_interval_ms")]
    pub keepalive_interval_ms: u64,  // 默认 2000 ms
    #[serde(default = "default_motion_hold_frames")]
    pub motion_hold_frames: u32,     // 默认 10 帧余晖
}
```

### 5.2 评估输出与遥测
```rust
pub struct MotionGateDecision {
    pub should_skip: bool,           // 是否拦截当前帧（true = 不送 NPU）
    pub motion_score: f32,           // 归一化热度分数 0.0 ~ 1.0
    pub is_keepalive: bool,          // 是否由保活心跳触发放行
}
```
遥测事件通过 `CameraTelemetryEvent.motion_score` 广播至前端实时进度条展示。

### 5.3 观测与阈值标定

- **决策日志**：`DEBUG` 级每路每 2s 最多一条，字段含 `camera` / `width` / `height` / `active_pixels` / `motion_score` / `should_skip` / `is_keepalive` / `bypassed_frames`；`active_pixels` 为真实有效变动像素数，现场无需额外探针即可标定 `threshold` 与 `contour_area`。
- **绕过计数与告警**：载体未接入缩略图链路、构建未启用 `rga`、缩略图初始化失败或单帧降采样失败时**保守放行**，但必须满足“绝不静默”：
  - 每类原因**首次**出现时 `WARN`（位图去重，不刷屏、也不丢新原因）；后续同类抑制为限速 `DEBUG`；
  - 初始化失败或连续失败达 `THUMB_FAILURE_STREAK_LIMIT`（3 帧）时熔断链路、`ERROR` 一次并降级为保守放行 —— 连续失败意味着源帧布局或链路本身不可用，逐帧重试只会白耗 RGA 与 CPU（并逐帧构造错误字符串）；
  - 逐帧计入 `MotionGate::bypassed_frames()`，按增量汇总到 `PumpMetrics::frames_gate_bypassed`；
  - 承接异常的单帧失败不抛错，只降级该帧；降级后不再消费随帧下发的规则版本，避免无意义的重传与克隆。
- **门控 Worker 异常**：专用线程创建失败、退出或超时未回信时，本路降级为保守放行，同样计入 `frames_gate_bypassed` 并 `ERROR` 一次（见 §2 线程归属）。
- **板端标定方法**：静止场景采集 `active_pixels` 的分布，取分位数留足余量后确定 `contour_area`；不得用开发机假设值直接下发。Host 载体的评估栅格等于源分辨率，`contour_area` 必须按实际画幅重新标定。
