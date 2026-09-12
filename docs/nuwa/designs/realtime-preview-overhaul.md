# 实时预览管线全面重构设计

> **目标**：实现 ZLMediaKit 级别的流媒体分发稳定性，消除网络波动导致的全局丢帧与重连风暴。
> **决策锁定**：激进路线 / Shared Ring + Pull 模型 / Arena 变长内存 / HTTP-FLV + WebCodecs 双通道
> **扩展约束**：架构必须天然支持 GB28181（SIP 信令 + RTP/PS 推流）作为未来接入协议

---

## 1. 问题根因

当前架构的核心瓶颈是 `broadcast::channel(64)` 的全局广播模型：

```
生产者 (RetinaIngestor)
    │
    ▼
broadcast::channel(64)  ← 64 帧 = 2.56 秒 @25fps，全局共享
    │
    ├─▶ Client A (慢) ── socket.send().await 挂起
    │       ↓
    │   RecvError::Lagged → 丢弃旧帧 → 所有客户端同时触发
    │
    ├─▶ Client B (快) ── 也被迫丢帧
    │
    └─▶ AI Pump / RingBuffer ── 同样受影响
```

**三层缺陷**：

| 层级 | 缺陷 | 影响 |
|---|---|---|
| 分发层 | broadcast 全局丢帧 | 1 个慢客户端拖垮所有客户端 |
| 传输层 | 无 TCP_NODELAY、无合并写 | 小包延迟 40ms+，帧间隔抖动 |
| 恢复层 | 固定 3s 重连、无指数退避 | 抖动风暴：断→连→断→连 |

---

## 2. 架构总览

### 2.1 新旧对比

```
旧架构：
  RetinaIngestor ──▶ broadcast(64) ──▶ [所有消费者共享，丢帧全局生效]

新架构：
  RetinaIngestor ──▶ SharedRing(512 slots, Arena)
                          │
                 ┌────────┼────────┐
                 ▼        ▼        ▼
              Cursor_A  Cursor_B  Cursor_C
              (自适应)  (自适应)  (AI Pump)
                 │        │        │
                 ▼        ▼        ▼
              FlvPipe   FlvPipe  RawPacket
                 │        │        │
                 ▼        ▼        ▼
              HTTP-FLV  WebCodecs  推理
```

### 2.2 核心组件

| 组件 | 职责 | 新增/改造 |
|---|---|---|
| `SharedRingBuffer` | Arena 预分配 + 变长写入 + 多消费者游标 | **新增** |
| `RingCursor` | 单消费者的读游标 + 自适应丢帧 + Lagged 恢复 | **新增** |
| `MediaIngestor` (Trait) | 统一媒体接入接口，协议无关 | **新增** |
| `RtspIngestor` | Retina RTSP 拉流，适配 MediaIngestor Trait | **改造** |
| `Gb28181Ingestor` | SIP 信令 + RTP/PS 推流接收 + PS 解封装 | **新增** |
| `PsDemuxer` | MPEG-Program Stream 解封装，提取 H.264/H.265 NALU | **新增** |
| `StreamHub` | 从 broadcast 分发改为 SharedRing 分发 + Ingestor 注册表 | **改造** |
| `CameraStreamSession` | 协议无关化，移除裸 rtsp_url，引入 ProtocolParams 枚举 | **改造** |
| `FlvStreamPipeline` | 移除内部 Lagged 恢复逻辑（下沉到 RingCursor） | **简化** |
| `live.rs` | 移除 WS-FLV 通道，统一 HTTP-FLV + WebCodecs | **改造** |
| `LivePlayer.tsx` | 移除 WS-FLV 逻辑，增加指数退避 | **改造** |

---

## 3. SharedRingBuffer 详细设计

### 3.1 内存模型

```rust
/// Shared Ring Buffer — ZLMediaKit 风格的多消费者拉取模型
///
/// 内存布局：
/// ┌─────────────────────────────────────────────────┐
/// │  Arena (预分配连续内存块)                         │
/// │  ┌─────┬─────┬─────┬─────┬─────┬─────┬─────┐   │
/// │  │Slot0│Slot1│Slot2│ ... │Slot5│Slot6│Slot7│   │
/// │  │     │     │     │     │     │     │     │   │
/// │  │len  │len  │len  │     │len  │len  │len  │   │
/// │  │data │data │data │     │data │data │data │   │
/// │  └─────┴─────┴─────┴─────┴─────┴─────┴─────┘   │
/// └─────────────────────────────────────────────────┘
///                    │
///          ┌─────────┼─────────┐
///          ▼         ▼         ▼
///       Cursor_A  Cursor_B  Cursor_C
///       (pos,gen) (pos,gen) (pos,gen)
///
/// 关键不变量：
/// - write_pos 单调递增（由生产者独占写入）
/// - 每个 Cursor 的 pos ≤ write_pos（不能读到未来帧）
/// - 最旧的未被所有 Cursor 读取的 slot 不可覆盖
/// - generation 每次 wrap-around 递增，用于检测 Cursor 是否已失效
```

### 3.2 Arena 内存分配策略

```rust
const DEFAULT_RING_CAPACITY: usize = 512;     // 512 slots
const DEFAULT_ARENA_SIZE: usize = 16 * 1024 * 1024; // 16MB Arena

struct Arena {
    /// 预分配的连续内存块
    buf: Vec<u8>,
    /// 当前写入位置（字节偏移）
    write_offset: usize,
    /// Arena 总容量
    capacity: usize,
}

impl Arena {
    /// 分配一段变长内存，返回写入位置和可用切片
    /// 若剩余空间不足，自动回收最旧的 slot 并 compact
    fn alloc(&mut self, size: usize) -> &mut [u8] {
        // 1. 尝试从当前写入位置分配
        if self.write_offset + size <= self.capacity {
            let start = self.write_offset;
            self.write_offset += size;
            return &mut self.buf[start..start + size];
        }
        // 2. 空间不足，回收最旧 slot 后从头分配
        self.compact();
        let start = self.write_offset;
        self.write_offset += size;
        &mut self.buf[start..start + size]
    }
}
```

### 3.3 Slot 元数据

```rust
/// 单个 Ring Slot 的元数据（固定大小，不含帧数据）
#[repr(C)]
struct SlotMeta {
    /// 帧数据在 Arena 中的字节偏移
    offset: u32,
    /// 帧数据实际字节长度
    len: u32,
    /// 帧的 PTS 时间戳（毫秒）
    pts_ms: i64,
    /// 是否为关键帧
    is_keyframe: bool,
    /// 编码格式
    codec: CodecType,
    /// 流类型（视频/音频）
    stream_tag: StreamTag,
    /// Arena generation（wrap-around 时递增）
    generation: u32,
}
// sizeof(SlotMeta) = 4+4+8+1+1+1+4 = 23 bytes，对齐后 24 bytes
// 512 slots × 24 bytes = 12KB 元数据，极轻
```

### 3.4 多消费者游标

```rust
/// 单消费者的读游标
struct RingCursor {
    /// 当前读取位置（slot 索引）
    pos: u64,
    /// 上次读取时的 Arena generation
    generation: u32,
    /// 消费者名称（用于日志与调试）
    name: String,
    /// 自适应丢帧阈值：落后超过此帧数时自动跳到最新
    catch_up_threshold: usize,
}

impl RingCursor {
    /// 拉取下一帧，返回 (SlotMeta, &[u8]) 或 None
    /// 若落后过多，自动跳到最新帧（ZLMediaKit 的 fast-seek 行为）
    fn pull(&mut self, ring: &SharedRingBuffer) -> Option<(SlotMeta, &[u8])> {
        let write_pos = ring.write_pos.load(Ordering::Acquire);
        let behind = write_pos.saturating_sub(self.pos) as usize;

        // 1. 超过阈值 → 跳到最新帧（丢弃中间所有帧）
        if behind > self.catch_up_threshold {
            self.pos = write_pos.saturating_sub(1);
            self.generation = ring.current_generation();
        }

        // 2. 尝试读取当前位置的 slot
        let slot = ring.get_slot(self.pos)?;
        if slot.generation != self.generation {
            // Arena 已 wrap-around，游标失效，跳到最新
            self.pos = write_pos.saturating_sub(1);
            self.generation = ring.current_generation();
            return ring.get_slot(self.pos).map(|s| (s, ring.get_data(s)));
        }

        self.pos += 1;
        Some((slot, ring.get_data(&slot)))
    }
}
```

### 3.5 SharedRingBuffer 核心接口

```rust
/// 多生产者安全的 Shared Ring Buffer
struct SharedRingBuffer {
    /// Arena 原始内存
    arena: Mutex<Arena>,
    /// Slot 元数据数组（固定大小 512）
    slots: Box<[SlotMeta; RING_CAPACITY]>,
    /// 当前写入位置（单调递增，由生产者独占）
    write_pos: AtomicU64,
    /// Arena generation（wrap-around 时递增）
    generation: AtomicU32,
    /// 所有活跃游标的最小读取位置（用于 Arena 回收判定）
    min_cursor_pos: AtomicU64,
}

impl SharedRingBuffer {
    /// 生产者：推入一帧（RTSP 拉流线程调用）
    ///
    /// 返回 Ok(slot_index) 或 Err(ArenaFull) — Arena 满表示所有消费者都严重落后
    fn push(
        &self,
        pkt: &EncodedPacket,
    ) -> Result<u64, RingError> {
        let slot_idx = self.write_pos.load(Ordering::Relaxed);
        let arena_idx = (slot_idx % RING_CAPACITY as u64) as usize;

        // 1. 在 Arena 中分配变长内存
        let data_len = pkt.payload.len();
        let mut arena = self.arena.lock();
        let data_offset = arena.alloc_offset(data_len)
            .map_err(|_| RingError::ArenaFull)?;

        // 2. 写入帧数据
        arena.write_slice(data_offset, &pkt.payload);

        // 3. 写入 Slot 元数据（原子更新）
        self.slots[arena_idx] = SlotMeta {
            offset: data_offset as u32,
            len: data_len as u32,
            pts_ms: pkt.pts_ms,
            is_keyframe: pkt.is_keyframe,
            codec: pkt.codec,
            stream_tag: pkt.stream_tag,
            generation: self.generation.load(Ordering::Relaxed),
        };

        // 4. 推进写入位置（Release 确保数据对消费者可见）
        self.write_pos.store(slot_idx + 1, Ordering::Release);

        Ok(slot_idx)
    }

    /// 消费者：创建一个新的读游标
    fn create_cursor(&self, name: &str) -> RingCursor {
        let pos = self.write_pos.load(Ordering::Acquire);
        RingCursor {
            pos,
            generation: self.generation.load(Ordering::Acquire),
            name: name.to_string(),
            catch_up_threshold: 30, // 默认落后 30 帧（1.2 秒 @25fps）时跳到最新
        }
    }

    /// 获取当前 generation（Arena wrap-around 时递增）
    fn current_generation(&self) -> u32 {
        self.generation.load(Ordering::Acquire)
    }

    /// 获取指定位置的 slot 元数据
    fn get_slot(&self, pos: u64) -> Option<SlotMeta> {
        let arena_idx = (pos % RING_CAPACITY as u64) as usize;
        let slot = &self.slots[arena_idx];
        // 验证 slot 的 generation 是否匹配请求的 pos
        let expected_gen = (pos / RING_CAPACITY as u64) as u32;
        if slot.generation == expected_gen {
            Some(*slot)
        } else {
            None // slot 已被后续帧覆盖
        }
    }

    /// 获取 slot 对应的帧数据切片
    fn get_data(&self, slot: &SlotMeta) -> &[u8] {
        let arena = self.arena.lock();
        arena.read_slice(slot.offset as usize, slot.len as usize)
    }
}
```

### 3.6 Arena 回收策略

```rust
impl Arena {
    /// 当 Arena 空间不足时，回收最旧的 slot 并 compact
    ///
    /// 回收条件：所有 Cursor 都已读过该 slot（通过 min_cursor_pos 判定）
    fn compact(&mut self, min_cursor_pos: u64, slots: &[SlotMeta; RING_CAPACITY]) {
        // 计算可回收的最旧 slot 位置
        let reclaim_pos = min_cursor_pos;
        let reclaim_idx = (reclaim_pos % RING_CAPACITY as u64) as usize;
        let reclaim_slot = &slots[reclaim_idx];

        // 简单策略：将 reclaim_slot 之后的数据整体前移
        // 生产环境可优化为 ring-style 无拷贝（数据在 Arena 内循环写入）
        let reclaim_end = reclaim_slot.offset as usize + reclaim_slot.len as usize;
        let move_len = self.write_offset - reclaim_end;
        self.buf.copy_within(reclaim_end..self.write_offset, 0);
        self.write_offset = move_len;
    }
}
```

> **性能注释**：`compact()` 的 `copy_within` 是 O(n) 操作，但对于 16MB Arena 和平均 50KB/帧的场景，实际移动量极小（<1MB）。若需零拷贝，可将 Arena 改为环形写入（write_offset 回绕），但这会增加 slot 读取的复杂度。初始实现建议用 compact，后续 profiling 后再优化。

---

## 4. 多协议接入层设计（GB28181 扩展）

### 4.1 MediaIngestor Trait

任何协议（RTSP / GB28181 / WebRTC）只要能产出 `EncodedPacket` 即可接入 SharedRing。

```rust
/// 统一的媒体接入器 Trait
/// 所有协议实现此接口后，由 StreamHub 按 CameraProtocol 动态分发
#[async_trait]
pub trait MediaIngestor: Send + Sync {
    /// 启动接入循环，持续向 ring push 帧
    ///
    /// 实现者必须：
    /// 1. 建立协议连接（RTSP DESCRIBE/PLAY 或 SIP REGISTER/INVITE）
    /// 2. 持续接收媒体数据
    /// 3. 解封装为 Annex B NALU
    /// 4. 调用 `ring.push(&packet)` 写入 SharedRing
    /// 5. 监听 cancel 信号，优雅退出
    async fn run(
        self: Arc<Self>,
        ring: Arc<SharedRingBuffer>,
        cancel: CancellationToken,
    ) -> Result<(), IngestorError>;

    /// 当前是否正在运行
    fn is_running(&self) -> bool;

    /// 接入器名称（用于日志与调试）
    fn name(&self) -> &str;
}

/// Ingestor 错误类型
#[derive(Debug, thiserror::Error)]
pub enum IngestorError {
    #[error("协议连接失败: {0}")]
    ConnectFailed(String),

    #[error("媒体流静默超时 ({0:?})")]
    InactivityTimeout(Duration),

    #[error("数据流错误: {0}")]
    StreamError(String),

    #[error("Ring Buffer 已满，所有消费者严重落后")]
    RingFull,
}
```

### 4.2 RtspIngestor（现有 RetinaIngestor 适配）

```rust
/// RTSP 拉流器 — 适配 MediaIngestor Trait
/// 内部复用现有 RetinaIngestor 逻辑，仅将 broadcast::tx.send() 替换为 ring.push()
pub struct RtspIngestor {
    camera_id: String,
    rtsp_url: String,
    transport_policy: TransportPolicy,
    is_running: Arc<AtomicBool>,
}

#[async_trait]
impl MediaIngestor for RtspIngestor {
    async fn run(
        self: Arc<Self>,
        ring: Arc<SharedRingBuffer>,
        cancel: CancellationToken,
    ) -> Result<(), IngestorError> {
        self.is_running.store(true, Ordering::SeqCst);
        // 复用现有 Retina 会话逻辑
        // 关键变更：将 `let _ = self.tx.send(packet)`
        // 替换为 `ring.push(&packet).map_err(|_| IngestorError::RingFull)?`
        // ...
        self.is_running.store(false, Ordering::SeqCst);
        Ok(())
    }

    fn is_running(&self) -> bool {
        self.is_running.load(Ordering::Relaxed)
    }

    fn name(&self) -> &str {
        "rtsp"
    }
}
```

### 4.3 Gb28181Ingestor（新增）

```rust
/// GB28181 推流接收器
///
/// 工作流程：
/// 1. SIP REGISTER 注册到国标平台（GB28181 上级平台）
/// 2. 保持 SIP 心跳（Keepalive）
/// 3. 收到 SIP INVITE 后建立 RTP 会话
/// 4. 接收 RTP 包 → PS 解封装 → 提取 NALU → EncodedPacket
/// 5. ring.push(&packet)
pub struct Gb28181Ingestor {
    camera_id: String,
    device_id: String,
    channel_id: String,
    /// SIP 会话句柄（FFI: eXosip2 或自研轻量 SIP）
    sip_ctx: Arc<SipContext>,
    /// RTP 接收器
    rtp_receiver: Arc<RtpReceiver>,
    /// PS 解封装器
    ps_demuxer: Arc<Mutex<PsDemuxer>>,
    is_running: Arc<AtomicBool>,
}

#[async_trait]
impl MediaIngestor for Gb28181Ingestor {
    async fn run(
        self: Arc<Self>,
        ring: Arc<SharedRingBuffer>,
        cancel: CancellationToken,
    ) -> Result<(), IngestorError> {
        self.is_running.store(true, Ordering::SeqCst);

        // 1. SIP REGISTER 注册
        self.sip_ctx.register(&self.device_id).await
            .map_err(|e| IngestorError::ConnectFailed(format!("SIP 注册失败: {e}")))?;

        // 2. SIP 心跳循环（30 秒间隔）
        let keepalive_handle = self.sip_ctx.start_keepalive(self.device_id.clone());

        // 3. 等待 SIP INVITE（设备主动推流）
        let media_session = tokio::select! {
            _ = cancel.cancelled() => {
                keepalive_handle.abort();
                return Ok(());
            }
            session = self.sip_ctx.wait_for_invite(&self.channel_id) => {
                session.map_err(|e| IngestorError::ConnectFailed(format!("INVITE 等待失败: {e}")))?
            }
        };

        // 4. 建立 RTP 接收（UDP 端口由 SDP 协商）
        self.rtp_receiver.bind(media_session.rtp_port).await
            .map_err(|e| IngestorError::ConnectFailed(format!("RTP 绑定失败: {e}")))?;

        tracing::info!(
            device_id = %self.device_id,
            channel_id = %self.channel_id,
            rtp_port = media_session.rtp_port,
            "GB28181 媒体会话建立，开始接收 RTP/PS 流"
        );

        // 5. RTP 接收 → PS 解封装 → NALU 提取 → ring.push()
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                rtp_pkt = self.rtp_receiver.recv() => {
                    let rtp_payload = match rtp_pkt {
                        Ok(p) => p,
                        Err(e) => {
                            tracing::warn!(error = %e, "RTP 接收错误");
                            continue;
                        }
                    };

                    // PS demux: RTP payload → Annex B NALU
                    let mut demuxer = self.ps_demuxer.lock().await;
                    let packets = demuxer.demux_rtp_payload(&rtp_payload);
                    drop(demuxer);

                    for pkt in packets {
                        if let Err(e) = ring.push(&pkt) {
                            tracing::warn!(error = %e, "GB28181 Ring Buffer 写入失败");
                            break;
                        }
                    }
                }
            }
        }

        keepalive_handle.abort();
        self.is_running.store(false, Ordering::SeqCst);
        Ok(())
    }

    fn is_running(&self) -> bool {
        self.is_running.load(Ordering::Relaxed)
    }

    fn name(&self) -> &str {
        "gb28181"
    }
}
```

### 4.4 PS Demuxer（MPEG-Program Stream 解封装）

```rust
/// MPEG-PS 解封装器
///
/// GB28181 的 RTP 负载格式：
///   RTP Header → PS Header (00 00 01 BA) → [PS System Header (00 00 01 BB)]
///   → PES Packet (00 00 01 E0=video / 00 00 01 C0=audio) → H.264/H.265 NALU
///
/// 本解封装器：
/// 1. 从 RTP 序列号重组乱序包
/// 2. 解析 PS 容器头
/// 3. 从 PES 中提取 Access Unit (AU)
/// 4. 判断关键帧（通过 random_access_indicator 或 NALU type）
/// 5. 封装为 EncodedPacket
pub struct PsDemuxer {
    /// PES 重组缓冲区（处理 RTP 序列号乱序与拆包）
    pes重组_buf: VecDeque<PsPacket>,
    /// 当前 PES 流状态
    pes_state: PesState,
    /// 从 SDP/PS header 推断的编码格式
    codec_hint: Option<CodecType>,
}

impl PsDemuxer {
    /// 从 RTP 负载中提取 NALU，返回一个或多个 EncodedPacket
    pub fn demux_rtp_payload(&mut self, rtp_payload: &[u8]) -> Vec<EncodedPacket> {
        let mut packets = Vec::new();
        let mut offset = 0;

        while offset < rtp_payload.len() {
            // 1. 解析 PS Pack Header
            //    固定起始码: 00 00 01 BA
            if rtp_payload.len() < offset + 14 {
                break;
            }
            if &rtp_payload[offset..offset + 4] != b"\x00\x00\x01\xba" {
                // 非 PS 包头，跳过（可能是 PES 续包）
                offset += 1;
                continue;
            }

            // 2. 解析 PS System Header（可选）
            //    起始码: 00 00 01 BB
            let mut pos = offset + 14; // PS Header 固定 14 字节
            if pos + 6 <= rtp_payload.len()
                && &rtp_payload[pos..pos + 4] == b"\x00\x00\x01\xbb"
            {
                let sys_header_len = ((rtp_payload[pos + 4] as usize) << 8)
                    | (rtp_payload[pos + 5] as usize);
                pos += 6 + sys_header_len;
            }

            // 3. 解析 PES Packet
            while pos + 4 <= rtp_payload.len() {
                if &rtp_payload[pos..pos + 3] != b"\x00\x00\x01" {
                    break;
                }
                let stream_id = rtp_payload[pos + 3];
                if stream_id < 0xBC {
                    break; // 非 PES 包
                }

                let pes_header_len = ((rtp_payload[pos + 4] as usize) << 8)
                    | (rtp_payload[pos + 5] as usize);
                let pes_data_start = pos + 6 + pes_header_len;
                if pes_data_start > rtp_payload.len() {
                    break;
                }

                // 4. 提取 PES payload（即 H.264/H.265 Access Unit）
                let pes_payload = &rtp_payload[pes_data_start..];

                // 5. 判断是否为关键帧
                //    - PS system_header 中的 random_access_indicator
                //    - 或 PES 数据的前几个字节中的 NALU type
                let is_keyframe = self.detect_keyframe(pes_payload, stream_id);

                // 6. 检测编码格式
                let codec = self.codec_hint.unwrap_or_else(|| {
                    self.guess_codec_from_nalu(pes_payload)
                });

                packets.push(EncodedPacket {
                    pts_ms: self.extract_pts_from_pes(&rtp_payload[pos..]),
                    is_keyframe,
                    codec,
                    payload: Bytes::copy_from_slice(pes_payload),
                    stream_tag: if stream_id == 0xE0 {
                        StreamTag::Video
                    } else {
                        StreamTag::Audio
                    },
                });

                pos = pes_data_start + pes_payload.len();
            }

            offset = pos;
        }

        packets
    }

    /// 从 PES 头部提取 PTS
    fn extract_pts_from_pes(&self, pes_header: &[u8]) -> i64 {
        // PES header 最少 6 字节，PTS 在第 9-13 字节（若 PTS_flag=1）
        if pes_header.len() < 9 {
            return 0;
        }
        let pts_flag = (pes_header[7] >> 6) & 0x01;
        if pts_flag == 0 {
            return 0;
        }
        // PTS 编码为 5 字节：'0010' + 32-bit + '1' / '0011' + 32-bit + '1'
        let pts_bytes = &pes_header[9..14];
        let pts_32 = ((pts_bytes[1] as u64) << 22)
            | ((pts_bytes[2] as u64) << 14)
            | ((pts_bytes[3] as u64) << 7)
            | ((pts_bytes[4] as u64) >> 1);
        (pts_32 / 90) as i64 // 90kHz 时基 → 毫秒
    }

    /// 检测关键帧（通过 NALU type）
    fn detect_keyframe(&self, pes_payload: &[u8], stream_id: u8) -> bool {
        if stream_id != 0xE0 {
            return false; // 非视频流
        }
        // H.264: NALU type = byte[0] & 0x1F，IDR = type 5
        // H.265: NALU type = (byte[0] >> 1) & 0x3F，IDR = type 19/20
        if pes_payload.is_empty() {
            return false;
        }
        match self.codec_hint {
            Some(CodecType::H264) => (pes_payload[0] & 0x1F) == 5,
            Some(CodecType::H265) => {
                pes_payload.len() >= 2 && matches!((pes_payload[0] >> 1) & 0x3F, 19 | 20)
            }
            _ => {
                // 启发式：H.264 IDR 或 H.265 IDR
                let nalu_type = pes_payload[0] & 0x1F;
                nalu_type == 5 || (pes_payload.len() >= 2 && (pes_payload[0] & 0x80) != 0)
            }
        }
    }

    /// 从 NALU 前几个字节猜测编码格式
    fn guess_codec_from_nalu(&self, pes_payload: &[u8]) -> CodecType {
        if pes_payload.is_empty() {
            return CodecType::H264;
        }
        let nalu_type = pes_payload[0] & 0x1F;
        if (1..=5).contains(&nalu_type) {
            CodecType::H264
        } else {
            CodecType::H265
        }
    }
}
```

> **GB28181 Rust 生态现状**：
> - SIP 协议栈：无成熟 Rust crate，推荐 FFI 接入 eXosip2（工业级 C 库，约 300 行 FFI 垫片）
> - RTP 解包：`rtp-rs` crate 可用（纯 Rust，MIT 协议）
> - PS 解封装：需自研（本文档已提供完整设计，约 500 行）
> - 总代码量：GB28181 接入层约 800~1200 行 Rust

---

## 5. StreamHub 改造

### 5.1 核心变更

```rust
// 旧：broadcast::channel(64) + 硬编码 RetinaIngestor
let (broadcast_tx, _) = broadcast::channel(64);
let _ = self.tx.send(packet);  // 火忘语义，无背压

// 新：SharedRingBuffer + MediaIngestor Trait + Ingestor 注册表
let ring = Arc::new(SharedRingBuffer::new(512, 16 * 1024 * 1024));
ring.push(&packet)?;  // 返回 Result，可感知 Arena 满
```

### 5.2 CameraStreamSession 协议无关化

```rust
pub struct CameraStreamSession {
    pub camera_id: String,
    pub protocol: CameraProtocol,           // Rtsp | Gb28181
    pub protocol_params: ProtocolParams,    // 按协议分发的参数
    pub shared_ring: Arc<SharedRingBuffer>,  // 共享 Ring Buffer
    pub ingestor: Arc<dyn MediaIngestor>,   // 多态接入器
    pub active_viewers: Arc<AtomicUsize>,
    pub keyframe_cache: Arc<RwLock<KeyframeCache>>,
    // ...其余字段不变
}

/// 协议参数枚举 — 替代裸 rtsp_url
#[derive(Clone)]
pub enum ProtocolParams {
    Rtsp {
        url: String,
        transport_policy: TransportPolicy,
    },
    Gb28181 {
        device_id: String,
        channel_id: String,
        sip_server: String,
        sip_port: u16,
        /// SIP 注册凭证（可选，部分平台无需认证）
        sip_realm: Option<String>,
        sip_user: Option<String>,
        sip_password: Option<String>,
    },
}
```

### 5.3 Ingestor 注册表（按 CameraProtocol 动态分发）

```rust
/// Ingestor 工厂 — 根据协议类型创建对应的 MediaIngestor
pub struct IngestorFactory;

impl IngestorFactory {
    pub fn create(
        camera_id: &str,
        params: &ProtocolParams,
    ) -> Arc<dyn MediaIngestor> {
        match params {
            ProtocolParams::Rtsp { url, transport_policy } => {
                Arc::new(RtspIngestor::new(
                    camera_id.to_string(),
                    url.clone(),
                    *transport_policy,
                ))
            }
            ProtocolParams::Gb28181 {
                device_id,
                channel_id,
                sip_server,
                sip_port,
                sip_realm,
                sip_user,
                sip_password,
            } => {
                Arc::new(Gb28181Ingestor::new(
                    camera_id.to_string(),
                    device_id.clone(),
                    channel_id.clone(),
                    sip_server.clone(),
                    *sip_port,
                    sip_realm.clone(),
                    sip_user.clone(),
                    sip_password.clone(),
                ))
            }
        }
    }
}
```

### 5.4 subscribe / unsubscribe 变更

```rust
impl StreamHub {
    /// 旧：返回 broadcast::Receiver
    pub async fn subscribe(...) -> Result<broadcast::Receiver<Arc<EncodedPacket>>, MediaError>

    /// 新：返回独立 RingCursor + Ring 引用
    pub async fn subscribe(
        &self,
        stream_key: &str,
        params: ProtocolParams,
    ) -> Result<(RingCursor, Arc<SharedRingBuffer>), MediaError> {
        let session = self.get_or_create_session(stream_key, params).await;
        session.active_viewers.fetch_add(1, Ordering::SeqCst);

        // 为每个消费者创建独立游标
        let cursor = session.shared_ring.create_cursor(&format!("flv:{}", stream_key));
        Ok((cursor, session.shared_ring.clone()))
    }

    /// 确保接入器正在运行
    fn ensure_ingestor_running(session: &Arc<CameraStreamSession>) {
        if session
            .ingestor_running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            let ingestor = session.ingestor.clone();
            let ring = session.shared_ring.clone();
            let cancel = session.cancel_token.clone();
            let session_clone = session.clone();

            tokio::spawn(async move {
                if let Err(e) = ingestor.run(ring, cancel).await {
                    tracing::error!(
                        camera_id = %session_clone.camera_id,
                        ingestor = ingestor.name(),
                        error = %e,
                        "媒体接入器运行异常"
                    );
                }
                session_clone
                    .ingestor_running
                    .store(false, Ordering::SeqCst);
            });
        }
    }
}
```

---

## 6. live.rs 改造

### 5.1 移除 WS-FLV 通道

```rust
// 旧路由
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/{camera_id}", get(handle_http_flv))
        .route("/{camera_id}/flv", get(handle_http_flv))
        .route("/{camera_id}/ws", get(handle_ws_live))      // ← 移除
        .route("/{camera_id}/webcodecs", get(handle_ws_webcodecs))
}

// 新路由
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/{camera_id}", get(handle_http_flv))
        .route("/{camera_id}/flv", get(handle_http_flv))
        .route("/{camera_id}/webcodecs", get(handle_ws_webcodecs))
}
```

### 5.2 HTTP-FLV 消费循环改造

```rust
async fn handle_http_flv(...) -> Response {
    // 旧：broadcast::Receiver
    // let mut packet_rx = state.stream_hub.subscribe(...).await?;

    // 新：RingCursor
    let mut cursor = state.stream_hub.subscribe(...).await?;
    let ring = state.stream_hub.get_ring(&str_key).await?;

    let stream = async_stream::stream! {
        let _guard = PreviewSessionGuard { ... };

        // ① 发送 FLV Header
        yield Ok::<Bytes, Infallible>(FlvMuxer::flv_header(include_audio));

        let mut flv_pipe = FlvStreamPipeline::new(include_audio);

        // ② 注入缓存中的 GOP
        if let Some(cache) = ring.get_keyframe_cache(&str_key_clone).await {
            let init_tags = flv_pipe.inject_cache(&cache, fallback_codec);
            for tag in init_tags {
                yield Ok(tag);
            }
        }

        // ③ 从 SharedRing 拉取帧（每帧独立游标，互不干扰）
        loop {
            let pkt = tokio::select! {
                _ = shutdown_rx.recv() => break,
                // 从 Ring 拉取，非阻塞，自带丢帧恢复
                pulled = async { ring.pull(&mut cursor) } => {
                    match pulled {
                        Some((meta, data)) => {
                            // 从 SlotMeta 重建 EncodedPacket
                            EncodedPacket {
                                pts_ms: meta.pts_ms,
                                is_keyframe: meta.is_keyframe,
                                codec: meta.codec,
                                payload: Bytes::copy_from_slice(data),
                                stream_tag: meta.stream_tag,
                            }
                        }
                        None => {
                            // Ring 为空，短暂 yield 后重试
                            tokio::time::sleep(Duration::from_millis(5)).await;
                            continue;
                        }
                    }
                }
            };

            // FLV 封装与下发
            for tag in flv_pipe.process_packet(&pkt) {
                yield Ok(tag);
            }
        }
    };
    // ...response headers...
}
```

### 5.3 WebCodecs 通道改造

```rust
async fn serve_ws_webcodecs(mut socket: WebSocket, ..., mut cursor: RingCursor, ring: Arc<SharedRingBuffer>) {
    // ① 初始秒开：从 ring 的 keyframe cache 注入
    // ② 实时消费循环：ring.pull(&mut cursor)
    // ③ 网络积压时：cursor.catch_up_threshold 自动跳帧
    // 无需 awaiting_keyframe_after_lag 手动标记，RingCursor 内置 fast-seek
}
```

---

## 7. FlvStreamPipeline 简化

### 6.1 移除内部 Lagged 恢复逻辑

```rust
// 旧：FlvStreamPipeline 自己处理 Lagged
pub fn handle_lagged(&mut self) {
    self.has_first_keyframe = false;
    self.bframe_mgr.reset();
}

// 新：Lagged 恢复下沉到 RingCursor
// FlvStreamPipeline 不再感知网络层丢帧
// RingCursor 的 fast-seek 保证拉到的每一帧都是可用的
// FlvStreamPipeline 只需关注：
// 1. Sequence Header 注入
// 2. B 帧时序矫正
// 3. SPS/PPS 动态突变检测
```

---

## 8. 前端改造

### 7.1 移除 WS-FLV 逻辑

```tsx
// LivePlayer.tsx — 移除 WS-FLV 相关代码
// 旧：WebCodecs 失败 → 降级到 WS-FLV → 再降级到 HTTP-FLV
// 新：WebCodecs 失败 → 直接降级到 HTTP-FLV
```

### 7.2 指数退避重连

```tsx
// 旧：固定 3s 重连
autoRetryTimerRef.current = setTimeout(() => {
  setRetryKey(k => k + 1)
}, 3000)

// 新：指数退避 + 抖动
function scheduleRetry(attempt: number) {
  const base = Math.min(1000 * Math.pow(2, attempt), 30000)
  const jitter = base * 0.1 * Math.random()
  autoRetryTimerRef.current = setTimeout(() => {
    setRetryKey(k => k + 1)
  }, base + jitter)
}
```

### 7.3 延迟测量修正

```tsx
// 旧：随机数模拟
setLatencyMs(Math.floor(100 + Math.random() * 40))

// 新：基于视频 element 的 currentTime 与 wall clock 差值估算
const measureLatency = () => {
  if (!videoEl || !videoEl.paused) return
  // 利用 mpegts.js 的 statisticsInfo 获取真实延迟
  // 或基于 FLV tag 的 DTS 与当前时间差
}
```

---

## 9. 数据流全景

```
┌─────────────────────────────────────────────────────────────────┐
│                        接入层（多协议）                           │
│                                                                  │
│  ┌──────────────┐  ┌──────────────────┐  ┌──────────────────┐  │
│  │ RtspIngestor │  │ Gb28181Ingestor  │  │ (未来) WebRtc    │  │
│  │              │  │                  │  │    Ingestor      │  │
│  │ SIP 信令     │  │ SIP REGISTER     │  │                  │  │
│  │ Retina RTSP  │  │ RTP/PS 接收      │  │  WHIP/WHEP      │  │
│  │ Annex B 输出 │  │ PS Demux → NALU  │  │  WebRTC SFU      │  │
│  └──────┬───────┘  └────────┬─────────┘  └────────┬─────────┘  │
│         │                   │                     │             │
│         └───────────────────┼─────────────────────┘             │
│                             ▼                                    │
│                    EncodedPacket (Annex B NALU)                  │
└─────────────────────────────┬───────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                     SharedRingBuffer (协议无关)                  │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │  Arena (16MB 预分配)                                     │   │
│  │  ┌────┬────┬────┬────┬────┬────┬────┬────┐              │   │
│  │  │ S0 │ S1 │ S2 │ S3 │ S4 │ S5 │ S6 │ S7 │ ... (512)   │   │
│  │  └────┴────┴────┴────┴────┴────┴────┴────┘              │   │
│  │                                                          │   │
│  │  write_pos: AtomicU64 (单调递增)                          │   │
│  │  generation: AtomicU32 (wrap-around 递增)                 │   │
│  └─────────────────────────────────────────────────────────┘   │
│                              │                                  │
│            ┌─────────────────┼─────────────────┐                │
│            ▼                 ▼                 ▼                │
│       RingCursor_A      RingCursor_B      RingCursor_C          │
│       (HTTP-FLV)        (WebCodecs)      (AI Pump)              │
│       catch_up: 30      catch_up: 15     catch_up: 5            │
└─────────────────────────────┬───────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────┐
│                        协议输出层                                 │
│                                                                  │
│  HTTP-FLV:                                                       │
│    RingCursor_A ──▶ FlvStreamPipeline ──▶ Body::from_stream     │
│    (TCP_NODELAY + Merge-Write)                                   │
│                                                                  │
│  WebCodecs:                                                      │
│    RingCursor_B ──▶ pack_webcodecs_frame ──▶ WebSocket::send    │
│    (二进制帧头 + 原始 NALU)                                      │
│                                                                  │
│  AI 推理:                                                        │
│    RingCursor_C ──▶ FrameRef ──▶ NPU 预处理                     │
│    (设备侧零拷贝)                                                │
└─────────────────────────────────────────────────────────────────┘
```

---

## 10. 性能预算

| 指标 | 旧架构 | 新架构 | 改善 |
|---|---|---|---|
| 单客户端丢帧影响 | 全局广播 | 仅自身 | 隔离 |
| 内存开销/客户端 | 无独立 buffer | ~200KB 元数据 | 可控 |
| Ring 总内存 | N/A | 16MB 固定 | 不随客户端增长 |
| Arena 帧拷贝 | 0（Arc 引用） | ~50KB memcpy | 可接受 |
| 首帧延迟 | ~500ms（等 GOP） | ~200ms（cache 注入） | -60% |
| 网络抖动恢复 | 1~2 秒冻结 | <100ms（fast-seek） | -90% |
| TCP 小包延迟 | 40ms+（Nagle） | <1ms（NODELAY） | -97% |

---

## 11. 迁移策略

### 阶段 1：SharedRing 实现与单元测试（不改现有代码）
- 实现 `SharedRingBuffer`、`RingCursor`、`Arena`
- 完整单元测试：push/pull、多 cursor 隔离、fast-seek、generation wrap-around

### 阶段 2：MediaIngestor Trait + RtspIngestor 适配
- 定义 `MediaIngestor` Trait
- 将现有 `RetinaIngestor` 适配为 `RtspIngestor`
- 实现 `IngestorFactory`
- 单元测试：Trait 对象调用、工厂分发

### 阶段 3：StreamHub 改造（API 兼容）
- `CameraStreamSession` 新增 `shared_ring` + `ingestor` 字段
- `CameraStreamSession` 引入 `ProtocolParams` 枚举
- `subscribe()` 返回 `(RingCursor, Arc<SharedRingBuffer>)`
- 内部缓存监听器改为从 Ring 读取
- `ensure_ingestor_running()` 改为调用 `ingestor.run(ring, cancel)`

### 阶段 4：live.rs 改造
- 移除 WS-FLV 通道
- HTTP-FLV 改为 RingCursor 消费
- WebCodecs 改为 RingCursor 消费
- TCP_NODELAY 配置

### 阶段 5：前端改造
- 移除 WS-FLV 降级路径
- 指数退避重连
- 延迟测量修正

### 阶段 6：清理与验证
- 移除 `broadcast::channel` 残留代码
- 全链路压力测试：8 路 1080P × 4 客户端 × 网络注入丢包
- 内存泄漏检测：Valgrind / ASAN

### 阶段 7（未来）：GB28181 接入
- FFI 封装 eXosip2（SIP 信令层，约 300 行垫片）
- 实现 `Gb28181Ingestor`（SIP REGISTER/INVITE + RTP 接收）
- 实现 `PsDemuxer`（MPEG-PS 解封装，约 500 行）
- 接入 `rtp-rs` crate（RTP 包解析）
- 集成测试：SIP 注册 → INVITE → RTP/PS 推流 → Ring → HTTP-FLV 出流

---

## 12. 风险与降级

| 风险 | 影响 | 降级方案 |
|---|---|---|
| Arena compact 拷贝开销 | 高帧率时 CPU 占用上升 | 改为环形写入（零拷贝） |
| Generation wrap-around 竞态 | 极端情况下 Cursor 读到脏数据 | CAS 重试 + 校验和 |
| 内存不足（RK3588 2GB） | Arena 分配失败 | 动态缩减 RING_CAPACITY 到 256 |
| 前端 mpegts.js 兼容性 | 部分浏览器 MSE 行为不一致 | 保留 HTTP-FLV 作为兜底 |
| GB28181 SIP 信令复杂度 | eXosip2 FFI 维护成本 | 可替换为自研轻量 SIP（UDP 直发） |
| GB28181 PS 封装厂商差异 | 部分 IPC 的 PS 头部非标 | PsDemuxer 增加容错解析路径 |
| GB28181 设备主动推送 | 无法控制设备何时推流 | Ingestor 支持热插拔，设备离线时 Ring 自动降级 |

---

## 13. 验证门禁

```bash
# 1. SharedRing 单元测试
cargo test --workspace -- shared_ring

# 2. MediaIngestor Trait 测试
cargo test --workspace -- media_ingestor

# 3. 压力测试：模拟 100 客户端并发拉流
# 在测试环境启动 8 路 RTSP 模拟流
cargo test --workspace -- ring_stress -- --ignored

# 4. 网络注入测试：tc netem 模拟丢包 5%、延迟 100ms
tc qdisc add dev lo root netem loss 5% delay 100ms
# 观察客户端是否无冻结、自动恢复

# 5. 内存泄漏检测
valgrind --leak-check=full ./target/release/heimdall

# 6. TCP_NODELAY 验证
ss -tnp | grep 8000
# 确认 nodelay 标志位

# 7. GB28181 集成测试（阶段 7，需 SIP 服务器环境）
cargo test --workspace -- gb28181 -- --ignored

# 8. 多协议交叉测试
cargo test --workspace -- multi_protocol -- --ignored
# 同时启动 RTSP 摄像头 + GB28181 设备，验证共享 Ring 互不干扰
```
