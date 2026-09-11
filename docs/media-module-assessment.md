# Media 模块现状与后续问题记录

> 评估范围：当前工作区中的 `crates/media` 以及它被 `api::routes::live` 使用的实时预览链路。
> 
> 评估基线：2026-09-04，当前未提交的 `camera-live-preview` 改动。
> 
> 本文的目的不是要求当前版本一次性实现完整 AI 媒体管线，而是区分：
> 
> 1. 当前 RTSP 接入与实时预览范围内已经存在的可靠性问题；
> 2. 当前阶段明确不包含、但后续工业化需要补齐的能力。

## 1. 当前阶段范围

当前版本的媒体模块定位为：

```text
摄像头 RTSP
  -> RTSP 1.0 握手
  -> TCP Interleaved RTP
  -> H.264/H.265 RTP 解包
  -> NALU 重组
  -> StreamHub 广播
  -> HTTP-FLV / WebSocket-FLV 实时预览
```

当前范围内已经实现或正在实现的能力：

- RTSP `OPTIONS`、`DESCRIBE`、`SETUP`、`PLAY` 基础流程；
- Basic/Digest 鉴权基础支持；
- TCP Interleaved RTP 接收；
- H.264 Single NALU、STAP-A、FU-A；
- H.265 Single NALU、AP、FU；
- H.264/H.265 SPS 基础解析；
- SPS/PPS/VPS 与最近关键帧缓存；
- `StreamHub` 按需启动、观众计数和 5 秒冷却；
- 指数退避重连；
- H.264 FLV 和 Enhanced FLV H.265 基础封装；
- HTTP-FLV / WebSocket-FLV 实时预览；
- 主码流到子码流地址的常见设备规则推导。

## 2. 当前阶段不包含的能力

以下能力属于后续阶段，不应作为当前 RTSP 预览 MVP 的验收前置条件：

- VideoToolbox、MPP、DVPP 或 FFmpeg 实际解码器接入；
- `FrameRef`、DMA-BUF、CVPixelBuffer、Device Memory；
- 硬件预处理（RGA、VPC、Metal）和 NPU 推理；
- 运动检测、抽帧、ROI、跟踪和告警管线；
- 录像、截图、存储水位与保留策略；
- WebRTC/WHEP 服务端；
- 完整的音视频同步与音频轨道；
- 多路硬件资源调度和设备级性能调优。

这些内容作为后续演进项记录在本文第 5 节，不作为当前版本“缺少已承诺功能”的缺陷。

## 3. 当前实现评估

### 3.1 已达到的部分

当前实现已经具备原型到 MVP 的基础：

- 代码按 `probe`、`rtsp`、`stream_hub`、`flv`、`sub_stream` 拆分，职责边界基本清晰；
- RTSP 响应已支持多次 partial read，不再假设一次 `read()` 能读完整响应；
- H.264/H.265 分片包具备基础重组能力；
- NALU 重组设置了 4 MB 尺寸保护；
- 关键帧和参数集缓存可以支持新预览连接初始化；
- 预览分发采用固定容量 `broadcast::channel(64)`，没有使用无界 channel；
- 当前媒体 crate 的基础单元测试和 clippy 检查通过。

当前已验证：

```text
cargo test -p media --all-targets      # 22 passed
cargo clippy -p media --all-targets -- -D warnings  # passed
```

上述结果只说明纯逻辑测试和静态检查通过，不等于真实摄像头协议兼容或长时间运行稳定。

### 3.2 当前范围内需要修正的问题

这些问题不依赖后续硬解或 AI 功能，属于当前 RTSP/实时预览链路的可靠性问题。

#### A. 停止流程不能可靠唤醒 RTSP 读取

位置：`crates/media/src/rtsp.rs`，`RtspIngestor::stream_session()`。

主循环使用原子变量检查取消状态，但实际读取是：

```rust
while !cancel_signal.load(Ordering::Relaxed) {
    let bytes_read = read_half.read(&mut read_scratch).await?;
}
```

如果 TCP 连接保持打开但长时间没有数据，设置 `cancel_signal` 不会唤醒 `read().await`。5 秒冷却或进程关闭可能无法及时停止拉流任务。

后续修正方向：

- 使用 `CancellationToken` 或可唤醒 channel；
- 对 socket read 使用 `tokio::select!`；
- 必要时增加读超时；
- 停止时显式关闭 socket；
- 保存并等待拉流任务结束。

#### B. Keep-Alive task 未纳入会话生命周期

位置：`crates/media/src/rtsp.rs`，`stream_session()` 中的 `tokio::spawn`。

Keep-Alive 任务的 `JoinHandle` 没有保存。读取端因网络错误退出后，写端任务仍可能持有旧 socket，直到下一次唤醒或共享取消标志改变。

可能结果：

- 重连过程中残留旧 keep-alive task；
- 旧 socket 延迟释放；
- 长时间断流/重连后任务和连接积累；
- 关闭流程无法确认所有资源已经释放。

后续修正方向：每个 RTSP session 保存独立的 keep-alive task handle 和取消 token，在 session 返回前停止并 join。

#### C. `unsubscribe()` 存在计数下溢风险

位置：`crates/media/src/stream_hub.rs`，`StreamHub::unsubscribe()`。

当前使用：

```rust
fetch_sub(1, Ordering::SeqCst)
```

如果异常路径或重复清理导致计数为 0，再执行退订会下溢为 `usize::MAX`，随后流可能无法满足 `active_viewers == 0` 的停止条件。

后续修正方向：使用 CAS 循环实现饱和减法，或者将订阅生命周期封装为带 `Drop` 的 subscription guard，避免重复退订。

#### D. 冷却 timer 可能互相覆盖

位置：`crates/media/src/stream_hub.rs`，`start_cooldown_timer()`。

当前先创建 task，再获取锁写入 handle。并发调用时，旧 timer 可能丢失引用，无法被取消。

后续修正方向：在同一把锁内取消旧 timer、创建新 timer，或使用 generation/token 方式保证只有最新 timer 有效。

#### E. RTSP `SETUP`/`PLAY` 响应状态校验不完整

位置：`crates/media/src/rtsp.rs`。

`SETUP` 主要根据是否能提取 `Session` 继续执行，`PLAY` 的响应也没有严格校验是否为 `200 OK`。

后续修正方向：

- OPTIONS、DESCRIBE、SETUP、PLAY 统一解析状态码；
- 非 2xx 立即返回带上下文的协议错误；
- 不允许仅凭 Session header 判定 SETUP 成功。

#### F. RTSP 响应缓冲没有最大长度

位置：`read_rtsp_response()`。

当前循环会持续向 `BytesMut` 追加数据，直到找到 `\r\n\r\n`，没有 header、body、Content-Length 上限。

异常设备或 malformed response 可能造成内存持续增长。

后续修正方向：

- RTSP header 设置最大长度；
- SDP body 设置最大长度；
- Content-Length 超限立即拒绝；
- 对不完整或非法响应返回明确错误。

#### G. 探活把 `v=0` 当成视频轨存在的依据

位置：`crates/media/src/probe.rs`。

当前判断：

```rust
if !sdp_resp.contains("m=video") && !sdp_resp.contains("v=0")
```

`v=0` 只是 SDP 版本字段，不表示存在视频轨。因此仅有音频的 SDP 也可能被接受。

后续修正方向：必须严格解析并确认 `m=video` media section，不能使用 `v=0` 作为替代条件。

#### H. 探活失败时使用固定的 1080P/25fps fallback

位置：`crates/media/src/probe.rs`，`StreamProber::probe_internal()` 和 `parse_sdp()`。

当 SPS 无法解析时，当前会返回固定的：

```text
1920 x 1080 @ 25fps
```

这适合演示，但不适合可靠的设备信息展示和后续资源分配。当前阶段建议至少将其标记为“参数未知”，而不是伪装成准确探活结果。

#### I. RTSP URL 可能将密码写入日志

位置：`crates/media/src/probe.rs`、`crates/media/src/rtsp.rs`。

当前日志字段直接使用 RTSP URL，而 URL 可能包含用户名和密码。

后续修正方向：实现统一的 URL 脱敏函数，日志只保留 host、port 和脱敏后的 path，禁止输出密码。

#### J. `StreamHub` 内部缓存监听器可能因 `Lagged` 永久退出

位置：`crates/media/src/stream_hub.rs`。

当前内部 listener 使用：

```rust
while let Ok(pkt) = internal_rx.recv().await {
    ...
}
```

broadcast 发生 `Lagged` 后，循环会退出，之后不再更新关键帧缓存和 `last_packet_time`。

后续修正方向：明确处理 `RecvError::Lagged`，恢复监听或改用独立的内部状态更新通道；不能让关键帧缓存依赖一个失败后永久退出的 listener。

## 4. 当前阶段的验收建议

当前 RTSP 实时预览阶段建议将验收标准限定为以下内容：

### 功能验收

- [ ] RTSP 无鉴权摄像头可以连接；
- [ ] Basic 鉴权摄像头可以连接；
- [ ] Digest 鉴权摄像头可以连接；
- [ ] H.264 Single NALU 可以预览；
- [ ] H.264 FU-A 可以预览；
- [ ] H.264 STAP-A 参数集可以初始化播放；
- [ ] H.265 Single NALU/FU 可以完成协议层转发；
- [ ] HTTP-FLV 客户端断开后订阅计数正确减少；
- [ ] 5 秒无人观看后拉流能够停止；
- [ ] 新客户端能够获得必要的参数集和关键帧。

### 可靠性验收

- [ ] 摄像头主动断开后能够指数退避重连；
- [ ] 拉流任务在停止时可被及时唤醒；
- [ ] 重连 100 次后 task 数量和 fd 数量不持续增长；
- [ ] 重复订阅/退订不会造成计数下溢；
- [ ] RTSP 异常响应不会被误判为在线；
- [ ] malformed response 和超长 response 不会导致内存无限增长；
- [ ] 日志不输出 RTSP 密码。

### 真实流验证

当前环境已确认 FFmpeg 版本为：

```text
ffmpeg 8.1.2
```

实际摄像头验证时应使用受保护的 URL，并完成：

```bash
ffprobe ...
ffmpeg ... -t 30 -map 0:v:0 -f null -
```

需要记录：

- codec、分辨率、FPS、time base；
- 是否存在 RTP 丢包；
- 是否出现 timestamp 回退；
- 是否出现 SPS/PPS 缺失；
- 首帧延迟；
- 断流后的重连时间；
- 30 分钟或更长时间内的 RSS、fd、task 数量变化。

## 5. 后续工业化演进项

以下内容不是当前实时预览 MVP 的阻塞项，但在接入 AI 分析和边缘设备长期运行前必须单独立项。

### 5.1 解码和帧管线

- 实现实际的 `VideoDecoder`；
- 根据平台接入 FFmpeg、VideoToolbox、MPP 或 DVPP；
- 设计 `FrameRef` 的所有权和 Drop 生命周期；
- 实现固定容量的帧 buffer pool；
- 解码线程与消费线程之间使用有界队列；
- 明确满载时丢旧帧还是丢新帧；
- 禁止在生产路径使用 CPU 像素拷贝作为默认方案。

### 5.2 硬件零拷贝

- Rockchip：MPP -> DMA-BUF -> RGA -> RKNN；
- Ascend：DVPP -> VPC/AIPP -> ACL；
- Apple：VideoToolbox -> CVPixelBuffer/IOSurface -> Metal/Core ML；
- 记录 stride、cache sync、alignment 和 fd 生命周期；
- 对 FFI 结构体增加 size/alignment/offset 双侧断言。

### 5.3 AI 管线

- 按摄像头配置抽帧；
- 小图运动检测；
- ROI/Mask/Line 规则；
- 推理 worker 固定数量；
- NMS、跟踪和事件判定；
- 推理失败时按帧降级，不拖垮整路或整个进程。

### 5.4 长期运行和可观测性

- 每路流的接收 FPS、bitrate、丢包率和重连次数；
- decode error、NALU drop、broadcast lag；
- buffer pool 使用率；
- socket fd 和 task 数量；
- 24~72 小时多路稳定性测试；
- ASan/UBSan/TSan 或 Rust 等价并发测试；
- systemd watchdog 与进程自愈。

### 5.5 录像和存储

- 录像/截图固定容量或按天数保留；
- 写入前检查磁盘水位；
- 低水位自动清理；
- 极低水位熔断写入但保持 API 可用；
- 避免帧级日志和无界内存缓存。

## 6. 阶段性结论

当前媒体模块可以继续作为：

```text
RTSP 接入 + H.264/H.265 码流转发 + 实时预览 MVP
```

但在修复第 3.2 节中的当前范围可靠性问题前，不建议称为“生产级 RTSP 服务”。

同时，以下判断应留到后续功能真正开始时再评估：

- 是否达到硬解和零拷贝要求；
- 是否达到 NPU 推理要求；
- 是否达到多路边缘设备长期运行要求；
- 是否达到录像、告警和无人值守部署要求。

当前阶段的合理目标是：

1. 先把 RTSP 握手、RTP 解包、预览分发和停止重连做成稳定 MVP；
2. 使用真实 H.264/H.265 摄像头完成协议回归；
3. 再为硬解和 `FrameRef` 设计独立的后续任务，避免提前引入未使用的复杂抽象。
