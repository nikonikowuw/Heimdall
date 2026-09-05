# 技术设计：接入 Retina 打造生产级 RTSP 接入内核

## 1. 架构总览

```text
┌────────────────────────────────────────────────────────┐
│                   IPC 摄像头 (RTSP Server)             │
└──────────────────────────┬─────────────────────────────┘
                           │ RTSP 1.0 (TCP 或 UDP RTP)
                           ▼
┌────────────────────────────────────────────────────────┐
│             crates/media/src/retina_ingest.rs          │
│                                                        │
│  1. URL 清洗与凭证抽取 (strip username/password)       │
│  2. retina::client::Session::describe()                │
│  3. 识别 video 轨 (encoding: "h264" | "h265")          │
│  4. Session::setup(Transport::Tcp / Transport::Udp)    │
│     - 绑定 FrameFormat::SIMPLE (自动注入 Annex B & SPS)│
│  5. Session::play().demuxed() 异步流消费                │
│  6. 转换为 types::EncodedPacket                        │
└──────────────────────────┬─────────────────────────────┘
                           │ Arc<EncodedPacket> 广播分发
                           ▼
┌────────────────────────────────────────────────────────┐
│                 crates/media/src/stream_hub.rs         │
│  • KeyframeCache (VPS/SPS/PPS 缓存与 GOP 闭环)         │
│  • MainStreamRingBuffer (主码流 0% 解码压缩包缓存)     │
│  • FlvStreamPipeline (HTTP-FLV / WS-FLV 实时推流)      │
└────────────────────────────────────────────────────────┘
```

---

## 2. 关键设计点与 Gotchas 处理

### 2.1 URL 格式与凭证隔离
`retina` 严格要求传递给 `Session::describe(url, ...)` 的 `url::Url` **不能包含嵌入式用户名或密码**（否则会直接报错 `URL must not contain credentials`）。
- **处理方案**：
  编写 `sanitize_rtsp_url_and_credentials(raw_url: &str) -> Result<(url::Url, Option<retina::client::Credentials>), MediaError>`：
  - 解析原始 URL；
  - 提取 username 和 password（若存在）；
  - 剥离 URL 中的 credentials，并重置为空；
  - 构建 `retina::client::Credentials { username, password }` 注入 `SessionOptions`。

### 2.2 传输策略映射 (TransportPolicy)
- `TransportPolicy::Tcp` ➔ `retina::client::Transport::Tcp(Default::default())`（RTSP interleaved over TCP）；
- `TransportPolicy::Udp` ➔ `retina::client::Transport::Udp(Default::default())`；
- `TransportPolicy::Auto` ➔ 优先尝试 TCP，若失败可退避或自适应。

### 2.3 帧格式与 Annex B 直通 (`FrameFormat::SIMPLE`)
`Heimdall` 下游的 `VideoToolboxDecoder`、`FlvMuxer` 和 `MainStreamRingBuffer` 都强依赖 Annex B 格式（`00 00 00 01` 起始码）以及关键帧前置参数集。
`retina` 的 `FrameFormat::SIMPLE`：
- `h26x_framing = h26x::Framing::AnnexB`
- `parameter_set_insertion = ParameterSetInsertion::EachKeyFrame`
因此解包出的 `VideoFrame::into_data()` 已经是纯净标准的 Annex B 序列，直接转成 `bytes::Bytes`，**无需二次格式重组，实现零拷贝直传**！

### 2.4 时间戳与单调展开
`retina` 内部已经接管了 32 位 RTP 时间戳回绕问题，并提供了 `frame.timestamp().elapsed_secs()`。
- 基准毫秒时标：在 Session 连接就绪时刻记录 `base_wall_ms = chrono::Utc::now().timestamp_millis()`；
- 每帧时间戳：`pts_ms = base_wall_ms + (frame.timestamp().elapsed_secs() * 1000.0) as i64;`；
- 结合单调滤波：`pts_ms = pts_ms.max(last_pts_ms);`，杜绝时间戳倒退。

### 2.5 异步取消与自愈重连
- 外层使用 Tokio 任务循环，结合 `tokio::sync::watch::Receiver<bool>` 和 `tokio::select!` 监控外部取消信号；
- 发生异常或网络中断时，执行指数退避（`Duration::from_secs(1)` 上限 30 秒）。

---

## 3. 模块组织与平滑演进

- 新增 `crates/media/src/retina_ingest.rs`，实现 `RetinaIngestor`；
- 导出 `RetinaIngestor` 至 `crates/media/src/lib.rs`；
- `StreamHub` 中无缝切换为 `RetinaIngestor` 驱动拉流循环；
- 原有自研 `RtspIngestor` 保留并作为测试或回退内核，确保已有单元测试不受破坏。
