# Design: 运动检测与前置门控引擎 (Motion Detection & Gating Engine)

> **状态**: Milestone 1/2 已实现（Host 帧与 Apple Unified Memory）；Milestone 3 嵌入式硬件缩略图（RK3588 RGA / 昇腾 VPC）待接入  
> **代码落地**: `crates/pipeline/src/motion_gate.rs`、`crates/types/src/task.rs`

---

## 1. 设计原则

1. **确定性 $O(1)$ 推理调度**：
   门控引擎仅输出放行/拦截决策。无运动拦截（0 次 NPU 推理）；有运动放行（1 次全图零拷贝直通推理，由目标检测模型并发检出所有目标）。严禁动态多区域切图多次送检。
2. **端到端零拷贝直通**：
   常驻推理主路径严禁 CPU 全图回读与色彩转换。纯切片借用 NV12 Y 分量；硬件加速帧通过 2D 加速器（RGA/VPC）异步生成极微缩图（160×90，14KB）回读差分。
3. **迟滞保活与余晖延时**：
   - **保活心跳（Keepalive）**：达到 `keepaliveIntervalMs`（默认 2000ms）强制放行 1 帧，刷新模型与跟踪器内部状态；
   - **运动余晖（Afterglow）**：检测到有效运动后维持连续 $N$ 帧（默认 10 帧）放行推理，防止目标微小停顿导致 ByteTrack 航迹断连。
4. **空间规则遮罩抑制**：支持结合 `DetectionRuleRole::Mask` 剔除干扰区域（如树叶、路面）；支持 `DetectionRuleRole::Roi` 防区过滤。
   `Precrop`（特写取景框）是**画幅**规则，只决定送模画面，**不得**并入掩码构建：否则「只画取景框」的任务会静默变成「只算框内运动」。见 [媒体管线](../backend/media-pipeline.md#特写取景预裁剪pre-crop-roi)。

---

## 2. 判定流水线拓扑

```text
[ 解码输出帧 ] ──► [ 获取 Y 分量 (切片借用 / 硬件极微缩图) ]
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
      [ 决策: 放行送检 (1次推理) / 拦截跳帧 (0次推理) ]
```

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

---

## 4. 跨平台 Y 平面提取策略

| 平台 | 载体 | Y 平面提取实现 | 性能开销 |
| :--- | :--- | :--- | :--- |
| **Host 内存帧** | `FrameHandle::Host` | 零拷贝切片借用 NV12 前 $W \times H$ 字节 | $< 1\mu s$ |
| **Rockchip** | `FrameHandle::DmaBuf` | RGA 下采样生成 $160 \times 90$ NV12 到 Scratchpad DMA-BUF，`dma_buf_sync` 读回 14.4KB | RGA $< 0.5ms$，D2H $< 10\mu s$ |
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
    pub contour_area: u32,           // 默认 100 像素
    #[serde(default = "default_keepalive_interval_ms")]
    pub keepalive_interval_ms: u64,  // 默认 2000 ms
    #[serde(default = "default_motion_hold_frames")]
    pub motion_hold_frames: u32,     // 默认 10 帧余晖
}
```

### 5.2 评估输出与遥测
```rust
pub struct MotionGateDecision {
    pub should_infer: bool,          // 是否放行当前帧进行 NPU 推理
    pub motion_score: f32,           // 归一化热度分数 0.0 ~ 1.0
    pub is_keepalive: bool,          // 是否由保活心跳触发放行
}
```
遥测事件通过 `CameraTelemetryEvent.motion_score` 广播至前端实时进度条展示。
