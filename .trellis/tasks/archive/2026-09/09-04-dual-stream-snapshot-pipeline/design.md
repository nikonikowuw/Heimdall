# 边缘按需硬件解码与双流高清抓拍管线 技术设计方案 (Subtask 2)

## 1. 架构目标与职责边界

本子任务聚焦于在单一 Rust 进程内实现**极致能效的边缘双流媒体流水线**：
1. **推理走轻流（子码流 640×360 按需硬解）**：仅在 AI 任务启用时拉起专属硬件解码工作线程（macOS VideoToolbox / Linux MPP），零拷贝流转至 C ABI 算法包，关闭任务时即刻释放硬件会话与显存；
2. **证据取高清（主码流 1080P/4K 零解码 Ring Buffer）**：主码流网络线程仅存 NALU 数据包，维持最近 2~3 秒 GOP 环形队列（内存仅 10~20MB，VPU 占用 0）；
3. **靶向高清单帧瞬时抽帧**：告警瞬间按时间戳 $T$ 精准索引 Ring Buffer 中的前置关键帧与 P 帧，硬件瞬时快进解码出 1 帧 1080P/4K 原图与目标特写抠图，异步线程池压缩落盘，主流断线时自动降级子流保底。

```
┌────────────────────────────────────────────────────────────────────────────────────────┐
│                                   crates/media & pipeline                              │
│                                                                                        │
│  ┌──────────────────────┐   RTSP RTP NALU   ┌───────────────────────────────────────┐  │
│  │ Camera Sub-stream    ├──────────────────>│ StreamHub (KeyframeCache)             │  │
│  │ (640×360 @ 15~25fps) │                   │ - Web MSE HTTP/WS-FLV 实时流分发      │  │
│  └──────────────────────┘                   └──────────────────┬────────────────────┘  │
│                                                                │ (broadcast channel)   │
│                                                                ▼                       │
│                                             ┌───────────────────────────────────────┐  │
│                                             │ On-Demand Decoder Worker (按需解码)   │  │
│                                             │ - 仅当 desired_enabled == true 时拉起 │  │
│                                             │ - VideoToolbox / MPP 硬件解码         │  │
│                                             │ - 输出 FrameRef (CVPixelBuffer 零拷贝)│  │
│                                             └──────────────────┬────────────────────┘  │
│                                                                │                       │
│                                                                ▼                       │
│                                                     [C ABI 算法包推理引擎]             │
│                                                                │                       │
│                                                        (触发告警时标 T)                │
│                                                                │                       │
│  ┌──────────────────────┐   RTSP RTP NALU   ┌──────────────────▼────────────────────┐  │
│  │ Camera Main-stream   ├──────────────────>│ MainStreamRingBuffer (内存环形队列)   │  │
│  │ (1080P/4K @ 25fps)   │                   │ - 仅存 NALU (Arc<EncodedPacket>)      │  │
│  └──────────────────────┘                   │ - 2~3秒 GOP (零 VPU/CPU 解码开销)     │  │
│                                             └──────────────────┬────────────────────┘  │
│                                                                │ (定位 I 帧 -> T 快进) │
│                                                                ▼                       │
│                                             ┌───────────────────────────────────────┐  │
│                                             │ Snapshot Engine (靶向精准快进抽帧)    │  │
│                                             │ - 瞬时解码单帧 1080P/4K 原图          │  │
│                                             │ - 降级策略: 主流不可用时抓取子流帧    │  │
│                                             │ - 异步图像池: SIMD/ImageIO 压缩 JPEG │  │
│                                             │ - 输出: 全景原图 + 目标特写抠图       │  │
│                                             └───────────────────────────────────────┘  │
└────────────────────────────────────────────────────────────────────────────────────────┘
```

---

## 2. 核心模块详细设计

### 2.1 主码流内存环形队列 (`MainStreamRingBuffer`)
- **存储载体**：`Arc<EncodedPacket>`（已在 `types::EncodedPacket` 中定义，包含 `pts: i64`, `is_keyframe: bool`, `codec: VideoCodec`, `payload: Bytes`）。
- **容量与驱逐策略**：
  - 双重边界约束：最大帧数上限（例如 120 帧）与最大时间跨度（默认 3500ms）；
  - 每次推入新数据包时，若队列时间跨度超过上限，且最老帧不是唯一的 I 帧，淘汰超出时限的老包；
  - 始终保留最近至少 1 个完整 GOP（即最近的 I 帧及其之后的所有 P 帧），确保随时能从关键帧起步解码。
- **检索接口**：
  ```rust
  pub fn get_gop_for_timestamp(&self, target_pts: i64) -> Option<Vec<Arc<EncodedPacket>>>
  ```
  根据时标 $T$ 快速定位：找到满足 $PTS_{I} \le T$ 的最近关键帧，并切出直至 $T$ 的完整连续 NALU 包切片。

### 2.2 硬件按需解码器 (`VideoToolboxDecoder` & `VideoDecoder`)
- **生命周期隔离**：
  - 任务开启（`desired_enabled == true`）：在独立专用工作线程拉起解码器会话；
  - 任务关闭（`desired_enabled == false`）：发送停止通知，退出循环，显存和解码会话在 `Drop` 中即刻全部销毁；
  - 不反压 RTSP 流：`broadcast::channel(16)`，消费慢时丢弃旧帧，绝不阻塞网络拉流与 Web 客户端播放。
- **macOS VideoToolbox 实现 (`crates/media/src/decoders/videotoolbox.rs`)**：
  - 基于系统 Framework `VideoToolbox` 和 `CoreMedia`：
    1. 从 SPS/PPS NALU 创建 `CMFormatDescription`；
    2. 创建 `VTDecompressionSession`，绑定输出属性 `kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange` (NV12) 并绑定 `kCVPixelBufferIOSurfacePropertiesKey`；
    3. 同步/异步解码 `CMSampleBuffer`，获得 `CVPixelBufferRef`；
    4. 封装进 `FrameRef::new(..., FrameHandle::ApplePixelBuffer { ptr })`，真正实现显存到 Core ML / 算法包零拷贝直通。

### 2.3 靶向精准抽帧快拍引擎 (`SnapshotEngine`)
- **精准快进解码**：
  - 收到抽帧指令 `(camera_id, target_pts, bbox)`；
  - 从 `MainStreamRingBuffer` 取出对应 GOP 的 NALU 包列表；
  - 顺序将 I 帧和后续 P 帧喂给专有快照解码器，仅保留时标最接近 $T$ 的那 1 帧 1080P/4K 解码原图；
  - **降级容错**：若主码流断线或 RingBuffer 为空，自动回退抓取当前子码流帧，确保 100% 不丢图。
- **异步特写抠图与 JPEG 压缩 (`SnapshotEncoder`)**：
  - 投递至独立 I/O 线程池，避免阻塞音视频流水线；
  - 全景图：将 NV12 / RGBA 压缩为高画质 JPEG；
  - 特写图：按目标 `BoundingBox` 进行坐标裁剪（增加 10% 边界防切边），保存特写 JPEG；
  - 保存至 `var/data/evidence/{camera_id}/{date}/{image_id}.jpg`。

---

## 3. 跨层数据流转与并发模型

1. **零锁争用**：
   - 拉流线程通过有界 channel 向 RingBuffer 快速写入；
   - 快照抽取复制 `Arc<EncodedPacket>` 句柄（仅增加引用计数，无内存拷贝），解码在快照专有线程进行。
2. **显存生命周期（RAII）**：
   - 解码产出的 `CVPixelBuffer` 包装进 `FrameRef`，并在 `Drop` 时触发 `CVPixelBufferRelease`；
   - 算法推理完成后自动析构，显存实时释放回系统驱动内存池。
