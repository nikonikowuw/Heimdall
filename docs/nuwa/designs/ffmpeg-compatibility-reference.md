# Design: FFmpeg 兼容性参考与协议增强设计

> **状态**：Phase 0~2 落地中（URL 解析、错误分类、时钟映射已落地）  
> **定位**：以 FFmpeg 行为作为接入基准对照，增强 Heimdall RTSP/RTP、时间戳与封装稳健性，保持纯 Rust 闭环与硬件零拷贝管线。

---

## 1. 协议分层与 FFmpeg 对照矩阵

```text
[ RTSP 边界 (URL/Digest/SDP/SETUP/PLAY) ] ──► [ RTP 分片重组 (FU-A/AP/AAC) ]
                                                     │
                                                     ▼
[ FLV / WebCodecs 输出 ] ◄── [ Arc<EncodedPacket> 归一化时基 ] ──► [ 硬件解码 FrameRef ]
```

| 能力 | FFmpeg 参考位置 | Heimdall 实现入口 | 重点约束 |
| :--- | :--- | :--- | :--- |
| **RTSP 握手/认证** | `rtsp.c` / `rtspdec.c` | `crates/media/src/retina_ingest.rs` | 401 Digest 重试、CSeq/Session 追踪、严格 URL 脱敏 |
| **SDP 与 Track** | `sdp.c` | `crates/media/src/rtsp.rs` | 提取 SPS/PPS/VPS/AAC 格式头，支持相对/绝对 control URL |
| **RTP H.264/H.265**| `rtpdec_h264.c` / `_hevc.c` | Retina demuxer | 校验 FU 连续性与 NAL 类型一致，丢弃 sequence gap |
| **时钟与 PTS** | `timestamp.h` | `crates/media/src/rtsp.rs` | 32 位回绕保护、单调递增保障、音视频独立时钟状态 |
| **封装与 Header** | `flvenc.c` | `crates/media/src/flv.rs` | Enhanced FLV HEVC (`hvc1`)、AVC sequence header |

---

## 2. 关键设计契约

### 2.1 URL 与凭证清洗
- `authority` 仅按最后一个 `@` 分割，兼容含裸 `@` 的旧密码；
- `userinfo` 的用户名/密码各执行一次 RFC 3986 percent-decoding（如 `%40` $\to$ `@`），`path_and_query` 保持原样；
- 所有日志强制 `mask_rtsp_url()` 脱敏，严禁记录明文凭证。

### 2.2 RTSP 状态机与错误分类
禁止以“未收到帧”粗暴替代错误原因，严格分类：
- `401 Unauthorized`：解析新 challenge 并有限重试，失败后退避；不触发 TCP/UDP 盲目切换；
- `461 Unsupported Transport` / SETUP timeout：尝试切换另一种传输协议（TCP $\leftrightarrow$ UDP）；
- `454 Session Not Found`：重建 RTSP Session；
- RTP Sequence Gap / FU mismatch：丢弃残缺包，等待新关键帧恢复，不阻断网络连接。

### 2.3 RTP 时间戳与时钟映射
1. 优先使用源 RTP 时间戳按 track clock rate（如 90kHz）换算为 13 位 UTC 毫秒整数，严禁用网络到达时间替代源时间；
2. 完整处理 32 位 `u32::MAX` 回绕与时间戳回跳；音视频轨独立维护回绕状态；
3. 输出 `EncodedPacket.pts_ms` 维持单调非递减。

---

## 3. 验证与黄金对照命令

对照测试严禁提交真实凭证到仓库，标准排查指令：
```bash
# 验证 TCP interleaved 接入与 SDP
ffprobe -hide_banner -loglevel trace -rtsp_transport tcp 'rtsp://user:***@host:554/path'

# 验证码流直封装可行性与 NALU 完整性
ffmpeg -hide_banner -loglevel warning -rtsp_transport tcp -i 'rtsp://user:***@host:554/path' -c copy -f null -
```
单元测试覆盖：特殊字符 URL（`%40/%23`）、Digest challenge、SDP 边界限制、时间戳回绕与音视频不同起点。
