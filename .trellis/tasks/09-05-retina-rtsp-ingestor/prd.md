# PRD: 接入 Retina 打造生产级 RTSP 接入内核

## 1. 业务与技术背景

当前 Heimdall 的 `crates/media/src/rtsp.rs` 实现了纯 Rust 基础 RTSP 客户端原型（TCP Interleaved 模式与基础 H.264/H.265 解包）。然而在真实工业与安防监控边缘场景中，自研原型缺乏真实摄像头“屎山”兼容性，且不支持 UDP RTP / RTCP 状态反馈。

`retina` (v0.4.20) 是经过工业级考验（Moonfire NVR 核心）的纯 Rust 异步 RTSP 客户端，具备以下核心能力：
- 完整 RFC 2326 / ONVIF 状态机支持；
- 支持 TCP Interleaved、UDP 单播与组播；
- 经过上千款真实摄像头毒打的非标 SDP 容错（如动态 SPS/PPS、非标参数集注入）；
- 原生支持 H.264 与 H.265 (HEVC) 组包重组，并支持输出 Annex B (`FrameFormat::SIMPLE`)；
- 原生 RTCP 保活与统计，消灭僵尸连接。

本任务目标是在 `crates/media` 中接入 `retina`，打造工业生产级的 RTSP 接入内核，同时无缝兼容现有的 `StreamHub` 广播、`FlvStreamPipeline` 预览与 `MainStreamRingBuffer` 靶向快拍管线。

---

## 2. 目标与非目标

### 2.1 目标 (In Scope)
1. **引入 `retina = "0.4.20"`**：作为 `crates/media` 的生产级 RTSP 接入引擎；
2. **实现 `RetinaIngestor`**：
   - 提取 RTSP URL 中的凭证（用户名/密码），通过 `retina::client::Credentials` 安全鉴权；
   - 适配 `TransportPolicy`：严格支持 `TransportPolicy::Tcp`、`TransportPolicy::Udp` 及 `Auto`；
   - 提取视频轨道（支持 H.264 与 H.265），配置 `FrameFormat::SIMPLE` 自动输出标准 Annex B 码流；
   - 将 `retina::codec::VideoFrame` 优雅映射为系统通用的 `types::EncodedPacket`；
   - 维持单调递增的 13 位 UTC Unix 毫秒时戳（`pts_ms`）；
3. **连接生命周期与自愈重连**：
   - 结合 Tokio `watch` / `select!` 实现确定性的毫秒级取消安全（Cancellation Safety）；
   - 保留指数退避重连机制（1s → 2s → 4s ... 30s）；
4. **与上层无缝对接**：
   - `StreamHub` 原生调度 `RetinaIngestor`，新旧拉流内核解耦平滑过渡；
   - 保证 HTTP-FLV / WS-FLV 预览与 MainStreamRingBuffer 靶向快拍 100% 正常运行；
5. **探活引擎 (StreamProber) 统一收敛**：
   - 废弃 `probe.rs` 中脆弱的原始 TCP 手工握手与 Digest 鉴权解析代码，全盘接入 `retina::client::Session::describe`；
   - 优先通过 `ParametersRef::Video` 读取真实宽、高与 FPS，降级利用 `parse_sdp` 解析 sprop 参数集，实现探活与拉流 100% 协议一致；
6. **单元测试与质量验证**：
   - 编写针对 URL 凭证剥离与转换、数据包映射、TransportPolicy 映射的测试用例；
   - 保证 `cargo fmt`、`cargo clippy`、`cargo test` 全绿通过。

### 2.2 非目标 (Out of Scope)
- 引入 `xiu` 完整服务包（因 `xiu` 存在废弃 `failure` 库和 `axum 0.6` 冲突，暂不引入其整体包，后续独立单独立项 RTMP 纯净解析）；
- 修改平台硬件解码（VideoToolbox / MPP / DVPP）逻辑；
- 修改前端 `LivePlayer` 播放器与 Canvas 渲染逻辑。

---

## 3. 验收标准 (Acceptance Criteria)

- [x] `crates/media` 成功引入 `retina = "0.4.20"`，无编译冲突与 ABI 警告；
- [x] `RetinaIngestor` 完整实现并支持 TCP / UDP 策略选择与凭证自动隔离；
- [x] 视频帧成功解包为 Annex B 格式并推入 `StreamHub` 的广播通道；
- [x] `StreamProber` 统一基于 Retina DESCRIBE 与 VideoParameters 提取尺寸与帧率，并保留非标 SDP 解析兜底；
- [x] `StreamHub` 订阅、退订、5 秒待机冷却与取消流程正常协同；
- [x] 现有 `cargo test -p media --all-targets` 单元测试及新增 Retina 单元测试 100% 通过；
- [x] `cargo clippy --all-targets -- -D warnings` 零警告。
