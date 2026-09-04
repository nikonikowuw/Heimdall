# Camera Ingestion & Real-Time Live Preview (摄像头接入与实时预览模块)

## 1. Goal (目标)

依据 `@prd/prd-v1.0.md` 第 5.2 节「摄像头接入与流媒体中心」、第 5.5 节「现代化 Web 控制台」以及 Grilling 对齐共识，构建纯 Rust 单二进制、端到端低延迟（100~300ms）的摄像头接入、探活、流分发、WebRTC/WHEP 实时推流与现代智能控制台大屏（Smart Hero + Bento Live Rail）：

1. **纯 Rust 异步 RTSP 接入与解析**：采用纯 Rust 协议栈（无 FFmpeg/C 动态库依赖），支持 TCP Interleaved / UDP 方式拉取 H.264/H.265 码流，提取原始 NALU 包与关键帧参数。
2. **轻量异步探活与 SPS 解析**：实现异步非阻塞 RTSP 探活器，纯 Rust 解析 SPS 提取分辨率（Width/Height）、Profile、编码类型及帧率，自动回写 SQLite 数据库并推送实时状态。
3. **双通路零拷贝分发与按需调度**：构建 `StreamHub` 码流分发引擎，基于 `Arc<EncodedPacket>` 广播通道解耦“实时预览”与“AI 推理硬解”，实施零延迟丢帧隔离背压；建立按需拉流状态机（活跃条件：WebRTC 观众数 > 0 或 AI 任务启用，带 5 秒优雅冷却），支持指数退避自动重连。
4. **WebRTC / WHEP 服务端与秒开机制**：基于 `webrtc-rs` 落地 `/api/v1/webrtc/whep` 端点，H.264 NALU 直通 RTP 打包（零二次编码），内存缓存 SPS/PPS/IDR 关键帧实现浏览器 <50ms 瞬间点亮首帧。
5. **现代边缘智能控制台（Smart Hero + Bento Rail）**：颠覆传统 1/4/9 九宫格，落地沉浸式主视口（72%）+ 智能状态轨（28%），结合 `<WhepPlayer>` 原生生命周期管理、Canvas 2D 零 Re-render 离屏轨迹/检测框叠加，支持 AI 智能追焦（Auto Spotlight）与自适应 Bento 全景矩阵。

---

## 2. Requirements (功能需求)

### 2.1 摄像头视频源管理与双轨健康探活 (Camera Assets & Dual-Track Health Probing)
1. **摄像头 CRUD**：
   - 基础信息：`camera_id` (UUID), `name`, `protocol` (固定 rtsp), `rtsp_url`, `sub_rtsp_url`, `remark`, `transport_policy` (`auto` / `tcp` / `udp`)。
   - 接口支持：`GET /api/v1/cameras` (列表), `POST /api/v1/cameras` (新增并自动触发探活), `PUT /api/v1/cameras/:id` (修改并触发探活), `DELETE /api/v1/cameras/:id` (删除), `POST /api/v1/cameras/:id/probe` (手动探活)。
   - 写操作接入 `oplog` 审计记录。
2. **纯 Rust 双轨健康度感知与防抖三态模型 (Dual-Track Sensing & 3-State Anti-Flapping)**：
   - **防状态抖动（Anti-Flapping）机制**：
     - `healthy` (绿灯)：正常在线，收包稳定；
     - `degraded` / `reconnecting` (黄灯/呼吸提示)：发生 1~2 次瞬时网络波动或正在重连，容错缓冲窗口（连续重试 3 次或持续 10 秒内不报死亡）；
     - `failed` (红灯)：连续 3 次探活失败或重连超过 10 秒无响应，才正式判定为彻底离线。
   - **活跃流看门狗 (Streaming Watchdog)**：拉流中若遭遇网络断开/EOF，拉流 Actor 进入 `degraded` 重连状态，触发指数退避自愈，超限后转为 `failed`。
   - **静默流定时巡检 (Periodic Standby Probe)**：后台常驻定时巡检任务（默认 30s 周期），对处于未拉流待机状态的摄像头执行轻量 RTSP 探活，结合防抖计数器更新状态。
   - **SPS 与元数据解析**：
     - `StreamProber::probe(rtsp_url, transport_policy, timeout)`：建立带超时的 RTSP 握手，抓取 SDP 及首个 SPS NALU。
     - 解析出：`codec` ("h264" / "h265"), `width`, `height`, `fps`。
     - 探活结果异步落库 `cameras` 表（`last_probe_status`, `last_probe_at`, `last_probe_error_code`, `last_codec`, `last_width`, `last_height`, `last_fps`），并通过 WebSocket 向前端广播更新事件。

### 2.2 码流分发引擎与按需生命周期 (StreamHub & Lifecycle)
1. **`Arc<EncodedPacket>` 数据结构**：
   - `EncodedPacket`: `{ pts_ms: i64, is_keyframe: bool, codec: CodecType, nalu: Bytes }`。
   - 通过 `tokio::sync::broadcast` 通道分发给 WebRTC 预览轨道和 AI Pipeline。
2. **有界队列与背压隔离**：
   - 通道容量固定（如 30~60 帧），消费者慢时自动丢弃旧帧（Lagged 静默处理），严禁反压阻塞 RTSP 网络接收套接字。
3. **按需拉流状态机 (On-Demand State Machine)**：
   - 每路摄像头维护 `active_subscribers = webrtc_viewers + (if ai_task_enabled { 1 } else { 0 })`。
   - 当 `active_subscribers > 0` 且处于挂起状态时：立即唤醒并启动 RTSP 异步拉流任务；
   - 当 `active_subscribers == 0` 时：启动 5 秒静默冷却计时器，若 5 秒内无新订阅，则自动断开 RTSP 连接并释放网络与内存句柄。
4. **断线指数退避自愈**：
   - 激活状态下遭遇网络断开或 RTSP 错误，以 1s → 2s → 4s ... 30s 指数退避自动重连。

### 2.3 WebRTC / WHEP 服务端与秒开加速 (WebRTC WHEP Server)
1. **WHEP 标准协议端点**：
   - `POST /api/v1/webrtc/whep?cameraId=:cameraId`：接收客户端 SDP Offer，返回 SDP Answer（`Content-Type: application/sdp`），并在响应头返回 `Location: /api/v1/webrtc/whep/:sessionId` 支持断开协商。
   - `DELETE /api/v1/webrtc/whep/:sessionId` (或 PATCH)：支持客户端显式断开 WHEP 会话。
2. **零转码 RTP 直通**：
   - 使用 `webrtc::rtp::codecs::h264::H264Payloader` 直接将 H.264 NALU 分包为 RTP 发送给 `TrackLocalStaticSample` / `TrackLocalStaticRTP`，CPU 占用极低。
3. **SPS/PPS + IDR 关键帧缓存秒开机制**：
   - 内存维护当前流最新的 SPS/PPS 及最近一个完整 IDR 帧；
   - 新客户端完成 SDP 握手后，首先下发 SPS/PPS 及最新 IDR 关键帧，使前端 `<video>` 50ms 内瞬间亮屏，彻底消除 2~4 秒的黑屏等待。

### 2.4 现代边缘智能控制台前端 (Modern Edge-AI Live Console)
1. **原子化 `<WhepPlayer />` 组件**：
   - 封装原生 `RTCPeerConnection`，通过 fetch 请求 `/api/v1/webrtc/whep?cameraId=xxx` 完成 WHEP 握手；
   - 媒体流绑定到 `<video autoPlay playsInline muted />`；
   - 严格在 `useEffect` 清理函数中调用 `pc.close()` 并释放媒体流，向后端发送关闭信令（减少观众计数）；
   - 连接断开时指数退避自动重试。
2. **Smart Hero + Bento Live Rail 交互布局**：
   - **Hero Stage (72% 区域)**：大画幅主视口，集成 Canvas 2D 60fps 轨迹/BBox 叠加，半透明科技 HUD（FPS、延时毫秒、分辨率、ANE 推理状态）；
   - **Bento Live Rail (28% 区域)**：右侧纵向排列其它摄像头的微型状态卡片（包含微缩画面、AI 目标统计徽标 `👤 2` `🚗 1`、Motion Gate 运动热度条、在线状态）；
   - **智能自动追焦 (Auto Spotlight)**：后台摄像头发生报警时，卡片亮起呼吸红光，一键（或自动）将主视口切至告警画面，右侧滑出快照缩略图；
   - **布局切换器**：右上角提供“智能焦点流联动”与“全景自适应 Bento 矩阵”一键切换开关。
3. **Canvas 2D 零 Re-render 渲染**：
   - Canvas 绘制循环由 `requestAnimationFrame` 驱动，目标检测坐标存储在 `useRef`，高频绘制绝不触发 React 状态树组件重绘。

---

## 3. Acceptance Criteria (验收标准)

- [ ] 摄像头列表支持增删改查及手动探活，写操作正确记录入 `operation_logs` 审计表。
- [ ] 探活功能（`StreamProber`）支持解析 RTSP 视频流的 `width`, `height`, `codec`, `fps`，探活成功/失败异步落库并通过 WebSocket 广播。
- [ ] `StreamHub` 成功实现 `Arc<EncodedPacket>` 广播分发，慢速消费者自动丢弃旧帧，不阻塞 RTSP 拉流线程。
- [ ] `StreamHub` 按需拉流状态机运作正常：无 WebRTC 观众且无 AI 任务时 5 秒内优雅挂起，有观众或启用任务时瞬间唤醒。
- [ ] `/api/v1/webrtc/whep?cameraId=xxx` 成功处理 SDP Offer 并返回有效 SDP Answer，浏览器原生 WebRTC 建立连接并流畅播放视频（延迟 ≤ 300ms）。
- [ ] 关键帧缓存机制生效：浏览器连上视频流后 50ms 内即可看到首帧画面，无需漫长等待下一个 GOP。
- [ ] 前端 `<WhepPlayer />` 具备完整的 RAII 生命周期，切换摄像头或注销组件时立即断开 PeerConnection，后端观众计数正确递减。
- [ ] 监控大屏成功落地 **Smart Hero + Bento Live Rail** 现代化布局，支持一键切换主流画面、AI 目标统计与报警脉冲联动。
- [ ] Canvas 2D 高频绘制在 60fps 正常工作且不引起 React 树多余 Re-render。
- [ ] 后端通过 `cargo fmt --all -- --check`、`cargo clippy --all-targets -- -D warnings` 与 `cargo test --workspace`。
- [ ] 前端通过 `pnpm format`、`pnpm lint`、`pnpm typecheck`、`pnpm test` 与 `pnpm build`。
