# Implementation Plan: Camera Ingestion & Real-Time Live Preview

## Phase 1: 基础设施与数据层补齐 (Infrastructure & Types)
1. **`types` crate**:
   - 补齐 `CodecType`、`TransportPolicy`、`CameraVo`、`ProbeResult` 及相关 WebSocket 消息定义；
   - 编写单元测试验证序列化/反序列化。
2. **`media` crate 纯 Rust SPS 参数集解析与探活**:
   - 实现 `media::sps`：Exp-Golomb 算术解码器，准确解析 SPS 提取分辨率（`width` / `height`）、Profile 与帧率；
   - 实现 `media::probe`：基于轻量 RTSP 握手提取 SDP 及 SPS 并解析；
   - 编写针对典型 1080P/720P/4K H.264 SPS 字节流的单测验证。

## Phase 2: 纯 Rust RTSP 接入与 StreamHub 码流分发 (Ingestor & StreamHub)
1. **`media::rtsp` 纯 Rust RTSP 客户端与 NALU 提取**:
   - 实现 RTSP 1.0 协议交互（OPTIONS, DESCRIBE, SETUP, PLAY）；
   - 支持 TCP Interleaved RTP 拆包与 NALU 拼装（SPS, PPS, IDR, Slice）；
   - 封装 `EncodedPacket` 与指数退避重连机制。
2. **`media::stream_hub` 调度、双轨探活与关键帧缓存**:
   - 实现 `StreamHub` 广播通道、活跃订阅计数（`active_viewers` + `ai_task_enabled`）；
   - 实现 5 秒静默冷却挂起与按需唤醒状态机；
   - 实现双轨健康度感知与三态防抖（Healthy 🟢 / Degraded 🟡 / Failed 🔴）；
   - 实现 SPS/PPS 与 IDR 内存缓存。

## Phase 3: WebRTC / WHEP 服务端与 API 路由对接 (WebRTC WHEP & Camera API)
1. **`api` crate 引入 `webrtc-rs`**:
   - 实现 `/api/v1/webrtc/whep` 端点：处理 SDP Offer、生成 Answer、建立 PeerConnection；
   - 绑定 H.264 Track，注入秒开关键帧缓存并持续分发 RTP 数据包；
   - 监听连接关闭事件，正确减少 `active_viewers`。
2. **完善摄像头 CRUD 与异步探活路由 (`api::routes::camera`)**:
   - 实现 `POST /api/v1/cameras`、`PUT /api/v1/cameras/:id`、`DELETE /api/v1/cameras/:id`、`POST /api/v1/cameras/:id/probe`；
   - 接入 `oplog` 审计记录与异步探活落库。

## Phase 4: 前端 `<WhepPlayer />` 与 Smart Hero Bento 控制台 (Web Console)
1. **封装原生 `<WhepPlayer />` 组件**:
   - 基于原生 `RTCPeerConnection` 实现 WHEP 握手与媒体流渲染；
   - 封装 Canvas 2D 离屏 60fps 绘制 Hook（`requestAnimationFrame` + `useRef`）；
   - 严格在 `useEffect` 清理函数中关闭 WebRTC 连接。
2. **重构 `LivePage.tsx` 交互大屏**:
   - 实现 **Smart Hero (72%) + Bento Live Rail (28%)** 现代化指挥舱布局；
   - 呈现沉浸式 HUD、AI 徽标、运动波形与一键聚焦；
   - 联动告警脉冲与智能追焦（Auto Spotlight）；
   - 提供全景 Bento 矩阵与主副流联动视图切换。

## Phase 5: 全链路联调与门禁验证 (Verification & Quality Gate)
1. **后端验证**：
   - `cargo fmt --all -- --check`
   - `cargo clippy --all-targets -- -D warnings`
   - `cargo test --workspace`
2. **前端验证**：
   - `pnpm format`
   - `pnpm lint`
   - `pnpm typecheck`
   - `pnpm test`
   - `pnpm build`
