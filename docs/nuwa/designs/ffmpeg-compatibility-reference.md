# FFmpeg 兼容性参考与媒体协议增强设计

> **状态**：持续实施中；Phase 0、Phase 1 的错误分类/transport 策略，以及 Phase 2 的时间戳映射与跳变约束已落地
>
> **目标**：以 FFmpeg 的摄像机兼容性行为作为参考，增强 Heimdall 的 RTSP/RTP 接入、时间戳处理、压缩封装与验证体系，同时保持 Retina 主导的 Rust 异步接入和平台原生硬件零拷贝管线。
>
> **范围**：`crates/media` 的 RTSP、SDP、RTP、时间戳、FLV/WebCodecs 边界，以及相关测试与诊断工具。

## 1. 背景与现状

Heimdall 当前使用 Retina 作为生产 RTSP 客户端，并在媒体层完成以下工作：

```text
RTSP 摄像机
    │
    ├─ TCP 控制连接：DESCRIBE / SETUP / PLAY / keepalive
    ├─ RTP：TCP interleaved 或 UDP
    └─ SDP：视频、音频轨道与 codec 参数
            │
            ▼
        Retina demux
            │
            ▼
    Arc<EncodedPacket>
       ├─ StreamHub / KeyframeCache
       ├─ 硬件解码与推理
       ├─ 主码流 RingBuffer
       ├─ HTTP-FLV
       └─ WebCodecs
```

FFmpeg 不作为 Heimdall 的核心 RTSP 运行时依赖。它作为事实上的兼容性参考实现，用于：

- 对照摄像机实际接受的 RTSP 请求；
- 提供 RTP/SDP/封装边界的行为基线；
- 生成协议测试样本和 golden fixtures；
- 排查 Retina 与非标摄像机之间的兼容性差异。

近期子码流问题暴露了一个具体缺陷：ffplay 会把 `tentcoo%4012` 解码为 `tentcoo@12`，而 Heimdall 自定义 URL 解析器原先将 `%4012` 当作密码字面量传给 Retina，导致 Digest `401`。该问题已通过 userinfo percent-decoding 修复。

## 2. 设计目标

### 2.1 目标

1. 统一 RTSP URL、userinfo、Digest challenge 和错误分类行为。
2. 提高非标摄像机在 RTSP 握手、SDP 和 RTP 分片方面的兼容性。
3. 保证 RTP timestamp、PTS、DTS、关键帧和参数集处理可验证。
4. 用 FFmpeg 生成可重复的协议对照样本，不依赖人工观察播放器窗口。
5. 保持 `FrameRef` 和平台原生设备 buffer 的边界不变。
6. 保持所有帧、GOP、重试和消费者队列有界。
7. 将平台差异继续收敛在 `media` / `infer`，不向 API 和 pipeline 扩散。

### 2.2 非目标

- 不用 FFmpeg 替换 Retina 的生产 RTSP 客户端。
- 不在常驻推理路径引入 FFmpeg CPU 解码、CPU 色彩转换或像素拷贝。
- 不复制 FFmpeg 源码实现第二套完整 RTSP 状态机。
- 不在本设计中解决所有摄像机厂商的 H.265 编码器固件问题。
- 不把 FFmpeg 的硬件上下文直接当作 MPP、RGA、VideoToolbox、DVPP 或 RKNN 的统一实现。

## 3. FFmpeg 对照矩阵

| 能力 | FFmpeg 参考位置 | Heimdall 对应位置 | 参考重点 |
| --- | --- | --- | --- |
| RTSP 会话与认证 | `libavformat/rtsp.c`、`rtspdec.c` | `retina_ingest.rs`、`rtsp.rs` | CSeq、Session、401、keepalive、状态码分类 |
| SDP 与 track | `libavformat/sdp.c`、`rtsp.c` | `probe.rs`、`retina_ingest.rs` | control URL、fmtp、sprop、音视频轨道 |
| RTP 通用逻辑 | `libavformat/rtpdec.c` | Retina demux / `rtsp.rs` | sequence、timestamp、marker、乱序与丢包 |
| H.264 RTP | `libavformat/rtpdec_h264.c` | H.264 depacketizer | Single NAL、STAP-A、FU-A、参数集 |
| H.265 RTP | `libavformat/rtpdec_hevc.c` | H.265 depacketizer | Single NAL、AP、FU、FU 类型一致性 |
| MPEG4-GENERIC/AAC | `libavformat/rtpdec_mpeg4.c` | AAC 音频轨道 | AU headers、采样率、声道和帧边界 |
| timestamp | `libavutil/mathematics.c`、`timestamp.h` | `rtsp.rs`、`retina_ingest.rs` | 时钟转换、回绕、PTS/DTS、无效时间戳 |
| Annex B 转换 | `libavcodec/bsf/*annexb*` | `probe.rs`、解码入口 | SPS/PPS/VPS、NAL start code、extradata |
| FLV 封装 | `libavformat/flvenc.c`、`avc.c` | `flv.rs` | AVC/HVCC、AAC sequence header、composition time |
| 硬件帧上下文 | `libavutil/hwcontext*.c` | `decoders/`、`infer/` | 生命周期、设备帧元数据、映射边界 |

FFmpeg 的源文件只作为行为参考。若需要复制具体代码，必须按对应文件的 LGPL/GPL 许可处理；默认方案是依据 RFC 和行为样本进行独立 Rust 实现。

## 4. 分层架构

```text
┌──────────────────────────────────────────────────────────────┐
│ API / Pipeline                                               │
│ 只消费 MediaError、EncodedPacket、FrameRef，不接触 RTSP 细节 │
└──────────────────────────────┬───────────────────────────────┘
                               │
┌──────────────────────────────▼───────────────────────────────┐
│ media::StreamHub / RetinaIngestor                            │
│ 物理连接复用、TransportPolicy、重连、广播、健康状态          │
└──────────────────────────────┬───────────────────────────────┘
                               │
┌──────────────────────────────▼───────────────────────────────┐
│ RTSP protocol boundary                                       │
│ URL/userinfo、Digest、DESCRIBE、SDP、SETUP、PLAY、keepalive   │
└──────────────────────────────┬───────────────────────────────┘
                               │
┌──────────────────────────────▼───────────────────────────────┐
│ RTP / compressed packet boundary                             │
│ H.264/H.265/AAC 分片、序列号、timestamp、参数集、关键帧     │
└──────────────────────────────┬───────────────────────────────┘
                               │
┌──────────────────────────────▼───────────────────────────────┐
│ EncodedPacket / FrameRef boundary                            │
│ 压缩包分发；硬件解码后转平台原生 FrameRef                    │
└──────────────────────────────────────────────────────────────┘
```

Retina 继续负责主 RTSP 会话和 demux。自定义代码只承担以下边界职责：

- URL 清洗、凭证提取和安全脱敏；
- vendor compatibility policy；
- `EncodedPacket` 时间戳和参数集归一化；
- Retina 输出与 StreamHub、RingBuffer、硬件解码入口之间的适配。

不得在 `retina_ingest.rs` 之外重新创建一套并行 RTSP 状态机，避免 CSeq、Session 和重连语义分裂。

## 5. 详细设计

### 5.1 URL 与 credentials

规则：

1. authority 只按最后一个 `@` 分割，以兼容历史上保存的裸 `@` 密码。
2. userinfo 中的 username/password 各执行一次 RFC 3986 percent-decoding。
3. `path_and_query` 保持原始字符串，不进行 userinfo 规则之外的解码。
4. 解码后的凭证只传给 Retina，不重新拼接到 clean URL。
5. 所有错误日志使用脱敏 URL，禁止记录明文密码。
6. `%40`、`%23`、`%3F`、`%25` 和 UTF-8 percent-encoding 必须有单元测试。

当前实现入口：

- `../../../crates/media/src/rtsp.rs`
- `../../../crates/media/src/retina_ingest.rs`

### 5.2 RTSP 握手、认证和错误分类

RTSP 控制流程：

```text
connect TCP
  -> OPTIONS（按设备兼容策略，可选）
  -> DESCRIBE + SDP
  -> SETUP video
  -> SETUP audio（可选）
  -> PLAY
  -> RTP/RTCP + keepalive
  -> TEARDOWN / reconnect
```

内部应区分错误类别，而不是仅用 `frames_streamed == 0` 判断传输失败：

| 错误 | 处理 | 是否切换 TCP/UDP |
| --- | --- | --- |
| `401 Unauthorized` | 解析新 challenge，有限次数重试；失败后退避 | 否 |
| `403 Forbidden` | 记录权限错误，延迟重试 | 否 |
| `404 Not Found` | 记录 URL/track 错误，等待配置变化 | 否 |
| `454 Session Not Found` | 重建 RTSP session | 否 |
| `461 Unsupported Transport` | 尝试另一种媒体传输 | 是 |
| TCP connect timeout/refused | 退避重连，RTSP 控制连接始终使用 TCP | 否 |
| SETUP transport timeout | 退避并尝试另一种媒体传输 | 是 |
| PLAY 成功后 RTP 静默 | 清理 session，按策略重连 | 可切换 |
| RTP sequence gap | 丢弃当前不完整 FU，继续等待下一帧 | 否 |

日志必须同时记录：

```text
attempted_transport
next_transport
phase = connect | describe | setup | play | receive
error_class
retry_after_ms
```

不能在切换 `current_transport` 后再用新值记录刚刚失败的尝试。

### 5.3 SDP 和 track control URL

SDP 解析必须验证：

- 至少存在可用 video track；
- `a=control` 的绝对 URL、相对 URL 和 session base URL 组合结果；
- 视频 codec 在 H.264/H.265 白名单内；
- H.264 的 SPS/PPS 和 H.265 的 VPS/SPS/PPS 能从 `fmtp` 提取；
- AAC 的 `config`、采样率和声道参数合法；
- SDP 和单个 header/body 有大小上限。

任何 SDP 缺陷都应在握手阶段返回协议错误，不应伪装为 UDP 失败。

### 5.4 RTP H.264/H.265

#### H.264

支持并测试：

- Single NAL Unit；
- STAP-A；
- FU-A；
- sequence number 不连续时丢弃当前未完成 FU；
- IDR、SPS、PPS 缓存和首帧注入。

#### H.265

支持并测试：

- Single NAL Unit；
- AP；
- FU；
- FU 起始片、连续片和结束片的原始 NAL type 一致性；
- sequence number、timestamp 和 marker 的边界；
- VPS/SPS/PPS/IRAP 缓存；
- 单个 NALU 最大尺寸和聚合包长度限制。

出现如下异常时：

```text
IdrWRadl -> FdNut
```

应丢弃当前不一致的 FU，不把不同 NAL 类型拼接成一个输出包。当前生产路径由 Retina 负责 FU 重组；Heimdall 在 Retina 公共 API 边界已落地视频/音频 frame loss 统计，并在丢包、静默、EOF 和 demux 错误日志中记录累计值。Retina 0.4.20 未向上层暴露 FU header mismatch 与 RTP marker 统计，因此以下计数仍需 Retina 升级、补丁或脱离生产路径的协议 fixture 支持：

```text
rtp_packet_loss_total
rtp_fu_reset_total
rtp_fu_header_mismatch_total
rtp_unknown_payload_total
```

如果 TCP interleaved 下仍持续出现 mismatch，应把它归类为发送端 packetization 异常或解析兼容性问题，而不是简单归咎于 UDP 丢包。

### 5.5 时间戳和输出 PTS

设计规则：

1. 优先使用 RTP/Retina 提供的媒体 timestamp，不使用包到达时间替代源时间。
2. 按 track 的 clock rate 转换到系统毫秒时间基。
3. 处理 32 位 RTP timestamp 回绕。
4. 音视频 track 分别维护 timestamp 状态，不共享回绕状态。
5. 对外 `EncodedPacket.pts_ms` 保持单调非递减。
6. 检测大幅时间跳变并记录 source timestamp、previous timestamp 和 delta。
7. 不在媒体层丢失 B-frame 所需的 DTS/PTS 语义；如果当前协议只传 PTS，必须在封装边界明确说明。

验证样本至少覆盖：

- 正常 25/30 fps；
- timestamp 逼近 `u32::MAX` 后回绕；
- 摄像机时间回跳；
- 音视频起始时间不同；
- B-frame 或乱序输出。

### 5.6 `EncodedPacket`、硬件解码和零拷贝边界

FFmpeg 的 `AVPacket`/`AVFrame` 分离方式可作为概念参考，但 Heimdall 的类型契约保持不变：

```text
RTP payload
  -> Arc<EncodedPacket>
  -> hardware decoder
  -> native FrameRef
  -> RGA/VPC/VideoToolbox/其他设备预处理
  -> NPU/GPU
```

约束：

- 压缩包可以使用 `Bytes`/`Arc` 共享，避免消费者之间复制；
- 常驻推理主路径不做 CPU 像素 readback；
- `FrameRef` 由平台媒体层创建并管理生命周期；
- FFmpeg 的 `hwcontext` 只参考设备帧生命周期和元数据组织，不替代厂商 SDK；
- snapshot/readback 仍是显式低频例外。

### 5.7 FLV 与 WebCodecs 输出

封装层使用 FFmpeg 生成的样本做 golden 对照：

- H.264 AVC sequence header；
- H.265 Enhanced FLV `hvc1`；
- VPS/SPS/PPS 更新；
- AAC AudioSpecificConfig；
- composition time；
- FLV tag timestamp；
- 断流、丢帧和重新注入关键帧后的 decoder state。

浏览器输出不应重新定义媒体 payload 结构，统一复用 `EncodedPacket` 和共享 WebCodecs header/guard。

## 6. 测试与验证

### 6.1 单元测试

| 领域 | 用例 |
| --- | --- |
| URL | `%40`、`%23`、`%3F`、`%25`、UTF-8、裸保留字符、path/query 不误解码 |
| Digest | 初次 challenge、nonce 更新、stale、Basic/Digest 选择、错误凭证 |
| RTSP | CSeq、Session、Transport、状态码与 phase 错误分类 |
| SDP | 相对/绝对 control URL、多 track、缺失 fmtp、大小上限 |
| H.264 | Single NAL、STAP-A、FU-A、丢包恢复 |
| H.265 | Single NAL、AP、FU、类型 mismatch、sequence gap |
| Timestamp | clock rate、回绕、跳变、音视频起点差异 |
| FLV/WebCodecs | sequence header、extradata、时间戳、关键帧恢复 |

### 6.2 FFmpeg 对照测试

使用相同 URL、相同账号和相同传输方式：

```bash
ffprobe -hide_banner -loglevel trace \
  -rtsp_transport tcp \
  'rtsp://user:<password>@host:554/path'

ffprobe -hide_banner -loglevel trace \
  -rtsp_transport udp \
  'rtsp://user:<password>@host:554/path'

ffmpeg -hide_banner -loglevel warning \
  -rtsp_transport tcp \
  -i 'rtsp://user:<password>@host:554/path' \
  -map 0:v:0 -c copy -f null -
```

禁止把包含真实密码的命令、Authorization header 或抓包文件提交到仓库。测试日志只保留：

- RTSP method、CSeq、status；
- challenge 的 scheme/algorithm/qop 类型；
- 脱敏 URI；
- codec、分辨率、clock rate；
- packet loss 和时间戳统计。

### 6.3 真机测试矩阵

| 维度 | 最低覆盖 |
| --- | --- |
| 摄像机 | 海康、大华、宇视或当前现场设备 |
| 码流 | 主码流、子码流、H.264、H.265 |
| transport | TCP interleaved、UDP |
| auth | 普通密码、含 `@/#/?/%` 密码、Digest challenge |
| 网络 | 正常、丢包、乱序、RTSP 半开、重连 |
| 输出 | 硬件解码、HTTP-FLV、WebCodecs、AI 分析 |

真实摄像机和硬件测试使用 `#[ignore]` 或独立集成测试标记，不阻断开发机默认 `cargo test`。

## 7. 分阶段落地计划

### Phase 0：URL 与认证边界

- [x] userinfo percent-decoding；
- [x] 保留裸特殊字符兼容；
- [x] path/query 不误解码；
- [x] 增加 `%40` 回归测试；
- [ ] 记录 Retina 与 ffplay 的 Digest challenge 差异。

### Phase 1：RTSP 错误分类和重试

- [x] 定义内部错误分类，不再以 `frames_streamed == 0` 代替错误类型；
- [ ] 401 nonce/stale 有限重试；
- [x] 记录 attempted/next transport；
- [x] 只有 transport 类错误才触发 TCP/UDP 切换；
- [ ] 增加 OPTIONS、GET_PARAMETER、SET_PARAMETER 兼容策略。

### Phase 2：RTP 和时间戳稳健性

- [ ] 补齐 H.264/H.265 RTP vector fixtures；
- [ ] FU mismatch、sequence gap、marker 边界统计；
- [x] RTP timestamp 回绕和大跳变处理；
- [x] 明确音频/视频 track 的时钟和 PTS 映射。

### Phase 3：封装与浏览器输出

- [ ] 生成并提交脱敏 golden fixtures；
- [ ] 对照 FFmpeg 验证 FLV AVC/HEVC/AAC sequence header；
- [ ] 验证参数集更新和丢帧后的关键帧恢复；
- [ ] 验证 WebCodecs header 与 Annex B payload 边界。

### Phase 4：平台硬件路径

- [ ] 对照 FFmpeg hwcontext 梳理设备帧生命周期；
- [ ] 验证 Retina compressed packet 到各平台硬件解码器的所有权转移；
- [ ] 分平台验证 DMA-BUF、CVPixelBuffer、device memory 和 cache sync；
- [ ] 不因 FFmpeg 对照测试引入 CPU fallback 到生产快路径。

## 8. 风险与决策

### 8.1 依赖和许可证

FFmpeg 是 LGPL/GPL 混合许可项目，具体源文件许可证必须单独确认。默认采用：

- 参考 RFC 和 FFmpeg 的可观察行为；
- 使用 Rust 独立实现；
- 不复制 FFmpeg 源码；
- 使用 ffprobe/ffplay 作为开发机验证工具，而不是生产运行时依赖。

如未来必须直接移植 FFmpeg 源码，应在引入前确认文件许可证、版权保留、源码提供和二进制分发义务，并补充项目许可证文件。

### 8.2 Retina 与 FFmpeg 的职责冲突

Retina 继续拥有生产 RTSP session 和 RTP demux 的主路径。FFmpeg 只用于：

- 行为对照；
- fixture 生成；
- 现场诊断；
- 明确的可选调试回退。

不能让同一路物理连接同时由 Retina 和 FFmpeg 拉流，否则会增加摄像机连接数、带宽和时间基差异。

### 8.3 非标摄像机行为

摄像机可能存在：

- Digest nonce 每次请求变化；
- 只支持 Basic 或错误实现 Digest；
- RTSP absolute URI 与 path URI 要求不同；
- H.265 FU 打包不符合 RFC；
- SDP control URL 不完整；
- TCP/UDP 对同一资源行为不同。

每个兼容性特例必须绑定到设备型号、固件版本和抓包事实，不能用无条件全局兼容分支污染所有摄像机。

## 9. 验收标准

本设计完成的最低标准：

1. 带 `%40`、`%23` 等编码凭证的主/子码流可通过 Retina Digest 鉴权。
2. `401` 不再触发错误的 UDP 降级日志。
3. `461`、连接超时和 RTP 静默能够按传输错误处理。
4. H.264/H.265 FU 丢包后不会跨 NALU 拼接。
5. RTP timestamp 回绕不会造成对外 PTS 倒退。
6. FFmpeg/ffprobe 和 Heimdall 对同一 SDP、codec、轨道和封装结果有可解释差异。
7. 常驻推理路径仍无 CPU 像素拷贝，所有队列和缓存仍有固定上限。
8. 单元测试、媒体层 Clippy 和 workspace 相关质量门禁结果可追溯。
