# Technical Design: Camera Ingestion & Real-Time Live Preview

## 1. 架构总览与模块划分

```
┌────────────────────────────────────────────────────────────────────────────────────────┐
│                        Heimdall Media Pipeline & Streaming Architecture                │
├────────────────────────────────────────────────────────────────────────────────────────┤
│                                                                                        │
│  [RTSP Stream]                                                                         │
│       │                                                                                │
│       ▼                                                                                │
│  ┌─────────────────────────────────────────────────────────────┐                       │
│  │ media::rtsp::RtspIngestor (Pure Rust, async tokio)          │                       │
│  │  - TCP Interleaved / UDP demux                              │                       │
│  │  - Extract SPS / PPS / IDR / Non-IDR NALUs                  │                       │
│  │  - Exponential backoff auto-reconnect                       │                       │
│  └──────────────────────────────┬──────────────────────────────┘                       │
│                                 │ Arc<EncodedPacket>                                   │
│                                 ▼                                                      │
│  ┌─────────────────────────────────────────────────────────────┐                       │
│  │ media::stream_hub::StreamHub (Global Stream & Lifecycle Hub)│                       │
│  │  - Active Subscriber Reference Counting (Viewers + AI Task) │                       │
│  │  - 5-Second Graceful Cooldown for Inactive Streams          │                       │
│  │  - Keyframe Cache: Latest SPS/PPS + Latest IDR NALUs        │                       │
│  │  - Bounded Broadcast Channel (32 packets, drop oldest)      │                       │
│  └──────────────┬───────────────────────────────┬──────────────┘                       │
│                 │                               │                                      │
│                 ▼                               ▼                                      │
│  ┌───────────────────────────────┐ ┌────────────────────────────────────────┐          │
│  │ webrtc-rs / WHEP Track        │ │ media::VideoDecoder (VideoToolbox/MPP) │          │
│  │  - H264Payloader (Zero Copy)  │ │  - send_packet(nalu, pts)              │          │
│  │  - Instant Keyframe Injection │ │  - Output: FrameRef (Zero-Copy Texture)│          │
│  │  - WebRTC PeerConnection      │ │  - Drop: Return buffer to Pool         │          │
│  └──────────────┬────────────────┘ └────────────────────┬───────────────────┘          │
│                 │                                       │                              │
│                 ▼ (WHEP / WebRTC)                       ▼                              │
│       [Browser / React 19]                      [AI Inference Pipeline]                │
│    (Native <WhepPlayer> & Canvas)            (Motion Gate -> CoreML -> ByteTrack)      │
└────────────────────────────────────────────────────────────────────────────────────────┘
```

---

## 2. 核心模块与设计细节

### 2.1 纯 Rust RTSP 接入与 SPS 解析 (`media::rtsp` & `media::sps`)
- **纯 Rust RTSP 客户端**：基于 Tokio 异步网络栈，处理 RTSP 1.0 协议标准流程（`OPTIONS` -> `DESCRIBE` -> `SETUP` -> `PLAY`），支持 TCP interleaved 传输通道与 UDP RTP/RTCP。
- **纯 Rust SPS 序列参数集解析器** (`media::sps`)：
  - 基于 Exponential Golomb 编码（ue/se 算法）无额外依赖解析 H.264 SPS 字节流；
  - 精确计算有效分辨率：$Width = (pic\_width\_in\_mbs\_minus1 + 1) \times 16 - (crop\_left + crop\_right) \times 2$，$Height$ 类似；
  - 提取 Profile IDC、Level IDC 以及 timing_info (FPS)。

### 2.2 码流分发中心、定时巡检与按需调度器 (`media::StreamHub`)
- **数据结构**：
  ```rust
  pub struct EncodedPacket {
      pub pts_ms: i64,
      pub is_keyframe: bool,
      pub codec: CodecType,
      pub payload: bytes::Bytes, // NALU 单元
  }
  
  pub struct CameraStreamSession {
      pub camera_id: String,
      pub rtsp_url: String,
      pub transport_policy: TransportPolicy,
      pub active_viewers: Arc<AtomicUsize>,
      pub ai_task_enabled: Arc<AtomicBool>,
      pub keyframe_cache: Arc<RwLock<KeyframeCache>>,
      pub broadcast_tx: broadcast::Sender<Arc<EncodedPacket>>,
      pub cancel_token: CancellationToken,
  }
  ```
- **生命周期状态转移**：
  - `Idle` -> `Active`: 收到 WHEP 连接或 AI 任务启动，触发拉流后台任务。
  - `Active` -> `Cooling`: `active_viewers == 0 && !ai_task_enabled`，启动 5s 延迟挂起定时器。
  - `Cooling` -> `Active`: 5s 内又有新观众或任务启动，取消定时器继续推流。
  - `Cooling` -> `Idle`: 5s 超时到达，释放拉流 TCP/UDP 资源与广播发送端。
- **定时巡检与防抖三态调度器 (Periodic Probe & Anti-Flapping Scheduler)**：
  - 后台每隔 30 秒执行一次轮询扫描；
  - 针对处于 `Idle` 状态的摄像头，通过 `StreamProber` 执行轻量握手验证；
  - 引入防抖迟滞逻辑（Failures 计数器）：
    - 首次失败：进入 `degraded` 状态（黄灯提示），重试计数 +1；
    - 连续 3 次失败：正式标记为 `failed`（红灯报警）；
    - 任意一次成功：重置计数器并恢复为 `healthy`（绿灯）。
  - 若状态发生变动，异步更新 SQLite `cameras` 表并通过 WebSocket 广播 `camera.probe_updated` 事件。

### 2.3 WebRTC (WHEP) 端点实现 (`api::routes::whep`)
- **依赖**：引入 `webrtc-rs`（包括 `webrtc::peer_connection`, `webrtc::track::track_local_static_sample`）。
- **流程**：
  1. 解析 HTTP POST Query `cameraId`，向 `StreamHub` 注册一个新的 WebRTC Subscriber；
  2. 创建 `RTCPeerConnection`，添加 H.264 Video Track；
  3. 解析客户端 SDP Offer，设置 Remote Description 并生成 Local SDP Answer；
  4. 启动异步转发协程：
     - 首先从 `keyframe_cache` 中读取最新 SPS/PPS 与 IDR 帧注入 Track；
     - 持续从广播通道接收 `Arc<EncodedPacket>`，通过 `H264Payloader` 打包为 RTP Sample 写入 Track；
  5. 监听 PeerConnection 状态（`OnConnectionStateChange`）：当状态进入 `Disconnected` / `Failed` / `Closed` 时，注销 Subscriber 并递减观众计数。

### 2.4 前端 `<WhepPlayer />` 与 Smart Hero 控制台
- **组件结构**：
  - `WhepPlayer.tsx`：受控组件，输入 `cameraId`、`className`、`onStats`，挂载 `<video>` 和 `<canvas>`；
  - `useWhepStream.ts`：Hook 封装 `RTCPeerConnection` 创建、WHEP POST SDP 交换、自动重连与卸载注销；
  - `useCanvasOverlay.ts`：通过 `requestAnimationFrame` 从全局 WebSocket 消息的 `Ref` 中读取 BBox 与 Track 向量进行 60fps Canvas 2D 绘制；
  - `LivePage.tsx`：
    - 主区域：**Hero Stage (72%)** 渲染大画幅 `WhepPlayer` + 科技 HUD；
    - 侧边栏：**Bento Live Rail (28%)** 渲染辅助卡片列表（包含微型 `WhepPlayer`、AI 徽标、Motion Gate 热度条与报警脉冲）；
    - 智能追焦：支持配置 Auto Spotlight，后台摄像头告警时主视口自动切流或弹出画中画。
