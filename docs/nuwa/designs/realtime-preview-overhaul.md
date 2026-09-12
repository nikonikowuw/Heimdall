# 实时预览管线重构设计

> **目标**：在 4~32 路安防摄像头、每路 1~5 个客户端的边缘场景下，达到工业级实时预览的稳定性要求：消费者隔离、丢帧可恢复、源流可重连、资源有上限、故障可观测。
>
> **准确定位**：本设计对齐 ZLMediaKit 在“单路压缩流多消费者分发”上的核心机制，不声称替代 ZLMediaKit 的完整多协议服务器能力，也不在未通过长稳和故障注入验收前宣称已经达到工业级交付状态。
>
> **原则**：复用已有 `Arc<EncodedPacket>` 共享压缩帧；不引入自定义 Arena；所有队列、缓存、连接数和任务都有固定上限；所有可恢复故障都有指标、日志和验证路径。

---

## 1. 范围与现状

### 1.1 本次范围

本设计只处理以下实时预览路径：

- RTSP 输入：现有 `RetinaIngestor`。
- HTTP-FLV 输出：`FlvStreamPipeline` + Axum body stream。
- WebCodecs 输出：WebSocket 二进制帧 + 浏览器 `VideoDecoder`。
- 分析输入：`AnalysisPump` 和主码流证据 `RingBuffer`。
- 源流断开、消费者慢读、客户端半开、重连和运行时观测。

### 1.2 明确不包含

以下内容不在本设计内：

- GB28181 SIP / RTP / PS 接入。
- RTMP、HLS、WebRTC SFU 等新输出协议。
- 自建 epoll 事件循环替换 Tokio/Hyper。
- 自定义 Arena、裸指针内存池或无锁共享字节区。
- 把预览分发层改造成通用 CDN。

GB28181 应单独建立接入层设计，复用本设计定义的 `StreamItem`、源流 epoch 和消费者生命周期契约，但不能把 SIP/PS 解封装逻辑混入本文件。

### 1.3 当前真实问题

当前 `CameraStreamSession` 使用 `tokio::sync::broadcast::channel`：

```
RetinaIngestor
    │
    ▼
broadcast::channel(64)          ← 所有消费者共享同一个覆盖窗口
    │
    ├─▶ HTTP-FLV Client A (慢)  ── recv() 不及时
    │       ↓
    │   RecvError::Lagged(n)
    │
    ├─▶ HTTP-FLV Client B (快)  ── 也被迫进入 Lagged 恢复
    ├─▶ WebCodecs Client C     ── 也被迫丢帧
    ├─▶ AnalysisPump           ── 也受到影响
    └─▶ KeyframeCache 监听器    ── 自身也可能 Lagged
```

现有代码已经具备部分可靠性基础：

- `RetinaIngestor` 已有握手超时、静默流 watchdog、指数退避和 `Auto` 传输降级。
- `KeyframeCache` 已保存 SPS/PPS/VPS 和最近 GOP，并用于新客户端首屏注入。
- `FlvStreamPipeline` 已有 B 帧 DTS/CTS 矫正和参数集变化检测。
- `LivePlayer` 的实际降级链已经是 WebCodecs → HTTP-FLV，WS-FLV 路径没有被前端主动使用。

本次设计不重复发明这些能力，而是修复它们之间的分发耦合和长期运行缺口。

---

## 2. 稳定性目标与验收口径

“工业级”不能由设计图直接证明。文档定义可验证目标，只有通过长稳、故障注入和资源预算测试后，才能作为生产能力宣称。

### 2.1 分发层目标

| 目标 | 验收口径 |
|---|---|
| 消费者隔离 | 一个慢客户端进入恢复或被驱逐时，其他正常客户端和 AI 消费者不产生额外丢帧 |
| 丢帧恢复 | 队列溢出后优先使用最近完整 GOP 的 `Replay` 消息，不继续发送残缺 P/B 帧 |
| 无缓存时恢复 | 尚无完整 GOP 时等待新的关键帧，不能把不可解码的 P/B 帧交给下游 |
| 半开连接处理 | 消费者在配置的无进展时间内没有读取进度时被关闭，不无限持有订阅和帧引用 |
| 资源边界 | 单流消费者数、全局消费者数、GOP 大小、队列元素数和单包大小均有硬上限 |
| 运行时可观测 | 能查询源流状态、队列深度、丢帧、恢复、驱逐、拒绝订阅和重连计数 |

### 2.2 建议默认预算

默认值必须进入配置，不在业务代码中散落硬编码；下列值是边缘设备的初始建议值，需用目标硬件实测校准：

```rust
pub struct PreviewDistributionConfig {
    /// 单路物理流最大预览/分析消费者数。
    pub max_consumers_per_stream: usize, // 默认 16
    /// 所有物理流合计最大消费者数。
    pub max_total_consumers: usize, // 默认 128
    /// 预览客户端 mailbox 元素数；Replay 作为一个元素。
    pub preview_mailbox_capacity: usize, // 默认 96
    /// 分析/证据消费者 mailbox 元素数；Replay 仍作为一个元素。
    pub analysis_mailbox_capacity: usize, // 默认 32
    /// GOP 最多保存的压缩包数量。
    pub max_gop_packets: usize, // 默认 75
    /// GOP 所有 payload 的总字节上限。
    pub max_gop_bytes: usize, // 默认 8 MiB，需按码率校准
    /// 单个压缩包的保护上限；超过时记录并进入恢复/降级路径。
    pub max_packet_bytes: usize,
    /// 消费者连续无读取进展的驱逐阈值。
    pub zombie_no_progress_ms: u64, // 默认 10_000
    /// HTTP-FLV 应用层合并写刷新周期。
    pub http_merge_flush_ms: u64, // 默认 10
    /// HTTP-FLV 单次合并 buffer 上限。
    pub http_merge_max_bytes: usize, // 默认 64 KiB
}
```

`preview_mailbox_capacity` 和 `analysis_mailbox_capacity` 限制的是 `StreamItem` 元素数量，不是 payload 字节数。各消费者共享同一个 `Arc<EncodedPacket>`，因此不能用“队列数 × 帧大小”简单估算物理 payload 内存；必须同时限制 GOP 字节数、单包大小和消费者总数，并在压力测试中测量 RSS。

### 2.3 不可宣称的内容

在没有真实环境记录前，不得写入“固定低于 100ms”“与 ZLMediaKit 完全等价”或“支持万路并发”等结论。延迟、吞吐和内存结论必须附测试设备、码率、客户端数量、浏览器版本和网络注入参数。

---

## 3. 总体架构

```
RetinaIngestor
    │
    │ publish(packet)
    │   ├─ 更新唯一 KeyframeCacheStore
    │   ├─ 更新时间和 source epoch
    │   └─ 分发 StreamItem
    ▼
PacketDispatcher
    │
    ├─▶ ConsumerMailbox A → HTTP-FLV
    │       └─ Replay(GopSnapshot) → reset pipeline → 注入参数集/GOP
    │
    ├─▶ ConsumerMailbox B → WebCodecs
    │       └─ Replay(GopSnapshot) → decoder.reset → 重放 GOP
    │
    ├─▶ ConsumerMailbox C → AnalysisPump
    │       └─ Replay/SourceReset → decoder flush/reset → 恢复解码
    │
    └─▶ ConsumerMailbox D → MainStream RingBuffer Attach

每个 ConsumerMailbox：
  - 固定容量 VecDeque
  - parking_lot::Mutex 只保护短时内存操作
  - tokio::sync::Notify 唤醒异步消费者
  - 满载时清空残缺队列并插入一个 Replay，而不是阻塞生产者
  - Drop/关闭时由 RAII 取消注册并唤醒 recv()
```

这里采用的是“共享 `Arc` payload + 每消费者独立有界 mailbox”的模型，等价于 ZLMediaKit 的 `shared_ptr<Packet>` + per-consumer 缓冲列表。它不是共享 Arena，也不是让消费者通过定时器轮询共享内存。

---

## 4. 消息与源流 epoch

### 4.1 显式消息类型

只传 `Arc<EncodedPacket>` 无法表达“这里发生过丢帧，需要重置下游状态”。因此消费者通道传输显式的 `StreamItem`：

```rust
#[derive(Debug, Clone)]
pub enum StreamItem {
    /// 正常压缩包。
    Packet(Arc<EncodedPacket>),
    /// 丢帧或新订阅恢复时的完整 GOP 快照。
    /// 快照内含参数集和从关键帧开始的压缩包引用。
    Replay(Arc<GopSnapshot>),
    /// RTSP 源会话重建。消费者必须丢弃旧解码状态，等待下一次 Replay/关键帧。
    SourceReset { epoch: u64 },
}

#[derive(Debug, Clone)]
pub struct GopSnapshot {
    pub epoch: u64,
    pub codec: CodecType,
    pub sps: Option<Bytes>,
    pub pps: Option<Bytes>,
    pub vps: Option<Bytes>,
    pub packets: Arc<[Arc<EncodedPacket>]>,
    pub first_pts_ms: i64,
    pub last_pts_ms: i64,
    pub total_payload_bytes: usize,
}
```

`Replay` 是一个 mailbox 元素，内部 `packets` 使用 `Arc` 共享；恢复时不会为每个消费者复制 NALU payload。`GopSnapshot` 的长度和总字节数受 `PreviewDistributionConfig` 限制。

### 4.2 源流 epoch

RTSP 会话重连后，源 PTS、参数集和解码参考链可能发生变化。必须显式增加源流 epoch：

1. `RetinaIngestor` 检测到非正常会话结束且准备重连时，调用 `dispatcher.source_reset()`。
2. `source_reset()` 原子递增 `epoch`，清空唯一 `KeyframeCacheStore`，清理每个消费者的旧队列，并插入 `SourceReset`。
3. 新会话收到 SPS/PPS/VPS 和关键帧后，生成新 `GopSnapshot`，消费者收到 `Replay` 后恢复。
4. 正常取消和服务停机不做重连，只关闭所有 mailbox 并唤醒等待者。

这样不会把旧 RTSP 会话的 PTS、参数集或残缺参考链带入新会话。

---

## 5. 唯一 KeyframeCacheStore

### 5.1 消除重复缓存

原方案同时在 `CameraStreamSession` 和 `PacketDispatcher` 中保存 GOP，容易出现两份状态不一致。改为一个唯一缓存：

```rust
pub struct KeyframeCacheStore {
    inner: parking_lot::RwLock<KeyframeCacheState>,
}

struct KeyframeCacheState {
    epoch: u64,
    codec: Option<CodecType>,
    sps: Option<Bytes>,
    pps: Option<Bytes>,
    vps: Option<Bytes>,
    current_gop: Option<Arc<GopSnapshot>>,
}
```

`CameraStreamSession.keyframe_cache` 和 `PacketDispatcher.cache` 持有同一个 `Arc<KeyframeCacheStore>`。新客户端首屏注入和慢客户端 Replay 都读取同一份状态，不再维护两个 `Vec<Arc<EncodedPacket>>`。

### 5.2 更新顺序

生产者每收到一个 `EncodedPacket`，严格按以下顺序执行：

```text
1. 校验包大小、codec、stream_tag 和 source epoch
2. KeyframeCacheStore.update(packet)
3. 更新 last_packet_time_ms
4. PacketDispatcher.publish(packet)
```

缓存更新只做短时内存操作，不能持锁 `.await`、IO 或 FFI。`parking_lot::RwLock` 只允许在该边界使用；HTTP handler 读取快照后立即释放锁，不把 guard 带入异步流程。

### 5.3 缓存完整性

- H.264 必须确认 SPS、PPS 和关键帧已形成可恢复快照。
- H.265 必须确认 VPS、SPS、PPS 和 IRAP/关键帧已形成可恢复快照。
- 超过 `max_gop_packets` 或 `max_gop_bytes` 时，当前候选 GOP 标记为不可恢复并清空；不能为了凑容量截断参考链。下一次关键帧到达后重新开始构建快照。
- 若单个关键帧或参数集超过保护上限，记录 `oversized_packet`，清空当前快照并等待下一次合法关键帧；不能把不完整快照标记为可恢复。
- 源 epoch 改变时清空所有旧参数集和 GOP。

---

## 6. ConsumerMailbox：正确实现丢旧、唤醒和 Replay

### 6.1 为什么不直接使用裸 `tokio::sync::mpsc`

裸 `mpsc::Sender` 无法安全地从生产者侧删除已经排队的旧元素。只用 `try_send()` 后再设置 `needs_keyframe` 会产生一个实质问题：旧 P/B 帧仍然占满队列，后续 GOP 回灌无法进入，消费者可能一直恢复失败。

因此本设计不把裸 `mpsc` 当作媒体帧队列，而是使用有界 mailbox：

```rust
struct MailboxState {
    queue: VecDeque<StreamItem>,
    closed: bool,
    recovering: bool,
}

pub struct ConsumerMailbox {
    state: parking_lot::Mutex<MailboxState>,
    notify: tokio::sync::Notify,
    capacity: usize,
    last_progress_mono_ms: AtomicU64,
    dropped_packets: AtomicU64,
    replay_count: AtomicU64,
}
```

媒体生产者永远不等待消费者；入队和清理都是有限的内存操作。

### 6.2 正常入队与恢复算法

```text
publish(packet):
  A. 先更新 KeyframeCacheStore，得到当前可用 gop_snapshot
  B. 遍历消费者快照，对每个 mailbox 执行 offer(packet, gop_snapshot)

mailbox.offer(packet, snapshot):
  1. mailbox 已关闭：返回 Closed
  2. mailbox 正在 recovering：
       - snapshot 可用：清空 queue，插入 Replay(snapshot)，退出 recovering
       - snapshot 不可用：丢弃 packet，等待下一次可恢复关键帧
  3. queue 未满：插入 Packet(packet)
  4. queue 已满：
       - 记录本次及被清理元素的丢帧数
       - 清空旧 queue，避免残缺 P/B 帧继续排队
       - snapshot 可用：插入一个 Replay(snapshot)，退出 recovering
       - snapshot 不可用：设置 recovering=true
  5. notify_one()
```

当队列溢出时，如果当前已有完整 GOP，恢复是**立即排队**，不需要等待下一个自然关键帧；如果没有完整 GOP，才等待新关键帧形成快照。这是“快速恢复”和“无错误参考链”的必要条件。

### 6.3 异步接收

`recv()` 使用 `Notify`，不能用 `sleep(5ms)` 轮询：

```rust
pub async fn recv(&self) -> Option<StreamItem> {
    loop {
        // 先创建 notified future，再检查状态，避免检查与等待之间丢通知。
        let notified = self.notify.notified();
        let result = {
            let mut state = self.state.lock();
            if let Some(item) = state.queue.pop_front() {
                Some(Ok(item))
            } else if state.closed {
                Some(Err(()))
            } else {
                None
            }
        };

        match result {
            Some(Ok(item)) => {
                self.last_progress_mono_ms
                    .store(monotonic_ms(), Ordering::Relaxed);
                return Some(item);
            }
            Some(Err(())) => return None,
            None => {
                notified.await;
            }
        }
    }
}
```

实际实现必须在一次锁作用域内完成 `closed` 和队列检查，避免伪代码中重复加锁造成竞态；不能持有 `parking_lot` guard `.await`。

### 6.4 订阅生命周期

`StreamHub::subscribe()` 不返回裸 receiver，而返回携带 RAII 的订阅对象：

```rust
pub struct MediaSubscription {
    pub id: ConsumerId,
    mailbox: Arc<ConsumerMailbox>,
    dispatcher: Arc<PacketDispatcher>,
}

impl Drop for MediaSubscription {
    fn drop(&mut self) {
        self.dispatcher.unsubscribe(self.id);
        self.mailbox.close();
    }
}
```

HTTP body、WebSocket 服务函数和 `AnalysisPump` 必须持有 `MediaSubscription` 直到消费循环退出。这样正常断开、HTTP 升级失败、任务取消、panic unwind 和服务停机都能释放消费者计数与队列引用。

---

## 7. PacketDispatcher 与资源准入

### 7.1 结构

```rust
pub struct PacketDispatcher {
    consumers: parking_lot::RwLock<HashMap<ConsumerId, Arc<Consumer>>>,
    next_id: AtomicU64,
    epoch: AtomicU64,
    cache: Arc<KeyframeCacheStore>,
    limits: PreviewDistributionConfig,
    metrics: Arc<DispatcherMetrics>,
}

struct Consumer {
    id: ConsumerId,
    name: String,
    kind: ConsumerKind,
    mailbox: Arc<ConsumerMailbox>,
}

#[derive(Debug, Clone, Copy)]
pub enum ConsumerKind {
    HttpFlv,
    WebCodecs,
    Analysis,
    MainStreamEvidence,
}
```

`consumers` 保存 `Arc<Consumer>`，遍历时先复制 `Arc` 快照再释放注册表读锁，避免在队列操作期间阻塞 subscribe/unsubscribe。锁内不得执行 IO、日志格式化、`.await` 或大块复制。

### 7.2 订阅准入

`StreamHub` 维护全局 `total_consumers: AtomicUsize`，并将它与现有 `active_viewers`/AI lease 生命周期分开：`active_viewers` 只表示预览输出订阅，AI 保活继续由已有 lease 计数管理。每个成功创建的 `MediaSubscription` 持有一次性释放的全局计数租约；`Drop`、显式取消和 watchdog 驱逐都必须通过同一个幂等释放路径归还计数。

准入检查至少包含：

1. 单路 `max_consumers_per_stream`。
2. 全局 `max_total_consumers`。
3. 按 `ConsumerKind` 选择 `preview_mailbox_capacity` 或 `analysis_mailbox_capacity`，并检查分类预算。
4. 服务停机状态。
5. 请求鉴权和摄像头权限。

达到上限时，领域层返回明确的 `TooManyConsumers`；API 层映射稳定 HTTP 状态和业务码，不能返回成功后再静默丢弃订阅。单路 dispatcher 注册和全局 `total_consumers` 增加必须有回滚顺序：先完成全局 CAS 预留，再尝试单路注册；单路失败立即归还全局计数。计数必须在注册成功后增加，注册失败不能泄漏 `active_viewers`。

### 7.3 僵尸判定

仅凭“连续 `try_send` 失败次数”不够可靠：一个低速但仍有读取进展的客户端可能偶尔成功入队。因此判定依据是 mailbox 的 `last_progress_mono_ms`：

- 队列达到高水位且无读取进展超过 `zombie_no_progress_ms`：标记为候选。
- 候选期间若收到读取进展，取消驱逐。
- 超时仍无进展：从 dispatcher 移除、关闭 mailbox、记录驱逐原因。
- 驱逐动作不关闭其他消费者，不取消物理 RTSP ingestor。

所有驱逐必须是幂等的，避免消费者 Drop 与 watchdog 同时移除造成计数下溢。

---

## 8. 下游消费者契约

### 8.1 HTTP-FLV

处理 `StreamItem`：

```text
Packet(packet):
  - 正常调用 FlvStreamPipeline.process_packet/process_audio_packet

Replay(snapshot):
  - 调用 reset_after_discontinuity()
  - 用 snapshot 的 SPS/PPS/VPS 重新生成 Sequence Header
  - 注入 snapshot 的完整 GOP
  - 重新建立音频 Sequence Header 状态

SourceReset(epoch):
  - 清理当前 pipeline 的参考链、DTS/CTS 和音频状态
  - 暂停发送普通 P/B 帧
  - 等待 Replay 或新的合法关键帧
```

`FlvStreamPipeline::handle_lagged()` 不应简单删除，而应改成有明确输入契约的 `reset_after_discontinuity()`。它只由 `Replay` 或 `SourceReset` 调用，不由普通 packet 猜测触发。

### 8.2 WebCodecs

WebSocket 二进制帧协议当前已有 4 字节 flags 字段，定义其中一个保留位：

- `flags & 0x01 != 0`：该帧是 Replay/Discontinuity 后的第一帧。
- 前端收到该标记后执行 `VideoDecoder.reset()`，再按关键帧和后续 GOP 解码。
- PTS 保持 `i64`，不截断成 `u32`。

`Replay(snapshot)` 中的每个视频包按现有 12 字节帧头发送，第一帧带 discontinuity 标记；WebCodecs 不使用 HTTP-FLV 的 10ms Merge Write，保持逐帧低延迟。

### 8.3 AnalysisPump

`AnalysisPump` 收到 `Replay` 时：

1. 清空当前待解码状态。
2. 调用 `VideoDecoder::reset()`；若后端无法原地 reset，执行明确的 flush/recreate 流程，受固定超时保护。
3. 按 GOP 顺序逐包解码，恢复参考帧链。
4. 不把 Replay 中的每一帧重复计入普通实时接收丢帧指标；单独计 `replay_packets`。

`VideoDecoder` Trait 需要增加默认安全契约：

```rust
async fn reset(&mut self) -> Result<(), MediaError> {
    let _ = self.flush().await?;
    Ok(())
}
```

硬件后端可覆盖该方法完成平台专用 reset；FFI 调用仍必须在专用硬件线程执行，不能在 Tokio worker 中直接调用。

### 8.4 主码流证据 RingBuffer

证据 RingBuffer 是独立消费者，不允许因预览客户端慢读而被迫 Lagged。它应使用 `MediaSubscription`，按 `Replay` 和 `SourceReset` 清理不完整 GOP，并等待新的关键帧恢复；不能把残缺 GOP 写入证据环。

---

## 9. HTTP-FLV 合并写与时间戳

### 9.1 应用层合并写

当前 HTTP-FLV 循环对每个 tag 单独 `yield`。改为 `BytesMut` 合并多个完整 FLV tag：

- `merge_flush_ms` 默认 10ms。
- `merge_max_bytes` 默认 64KiB；大关键帧达到上限立即 flush。
- 只在 FLV tag 边界拼接，不能拆开一个 tag。
- body stream 退出前 flush 残留 buffer。
- 记录合并前后 chunk 数、flush 次数和最大 buffer 深度。

这属于应用层 chunk coalescing，不应在文档中承诺每个 chunk 一定对应一个内核 `write()`；实际 TCP 系统调用次数必须通过目标环境 `strace`/eBPF 测量。WebCodecs 通道不合并。

### 9.2 TCP_NODELAY

HTTP accepted socket 是否启用 `TCP_NODELAY` 必须以实际 socket 验证为准。若当前 `axum::serve` 入口无法在 accept 后设置，应使用明确的自定义 accept/service 装配，而不是在代码注释中声称已经开启。

TCP_NODELAY 是补充优化，不能替代消费者隔离、队列上限、合并写和健康检查。RTSP 上游连接已有的 nodelay 设置与 HTTP 下游连接是两个独立边界，不能混淆。

### 9.3 时间戳契约

- 内部和跨层帧时间戳继续使用 `i64` 的 13 位 UTC Unix 毫秒或源帧时间基准，不能用回调到达时间替代源 PTS。
- RTSP 会话内使用单调时间映射，不能逐帧调用 `Utc::now()` 生成 PTS，避免系统校时造成跳变。
- 新源 epoch 必须重置输出 pipeline 的 base PTS、DTS/CTS 状态和 B 帧观察状态。
- `FlvStreamPipeline` 内部使用 `i64`/`u64` 计算相对时间，序列化到 FLV 前检查 `u32` 边界。
- 在 `u32::MAX` 前设置可配置的 rebase guard；只在关键帧执行输出 epoch 重建，或在达到阈值前主动结束 HTTP-FLV 响应让客户端重新订阅。不得静默溢出回绕。
- WebCodecs 协议已经使用 8 字节 PTS，不得为了复用 FLV 逻辑截断时间戳。

必须增加接近 `u32::MAX` 的时间戳单元测试和模拟长连接测试。没有这项测试，不能宣称支持数十天连续 HTTP-FLV 连接。

---

## 10. 源流故障与恢复

### 10.1 RTSP 源故障

保留现有 `RetinaIngestor` 的：

- DESCRIBE/SETUP/PLAY 握手超时。
- 6 秒默认静默 watchdog。
- TCP/UDP Auto 降级。
- 单路指数退避重连。
- `compare_exchange` 防止同一 session 重复启动。

新增：

- 断流进入重连前发布 `SourceReset`，清除旧 GOP 和参数集。
- 重连计数、最近错误、当前退避时长和最后收包时间进入健康快照。
- 多路同时断流时，对重连等待加入确定性或随机抖动，避免设备恢复时形成连接洪峰。
- 重连后必须重新收到合法参数集和关键帧，不能把上一 epoch 的 cache 当作新流首包。
- 连续重连失败不退出整个进程，只将该物理流标为 `Degraded/Failed` 并保留控制面。

### 10.2 客户端重连

前端使用指数退避 + 抖动，但只有连接稳定运行一段时间后才重置 attempt，避免短暂 `MEDIA_INFO` 事件把退避计数过早清零：

```tsx
const baseMs = Math.min(1000 * 2 ** attempt, 30_000)
const jitterMs = baseMs * 0.1 * Math.random()
```

连接错误、HTTP 401、用户主动暂停和服务停机必须分别处理；用户主动暂停不应自动重连。

---

## 11. 运行时观测与运维接口

工业级服务不能只依靠日志。`PacketDispatcher` 增加原子计数器和有界快照：

```rust
#[derive(Debug, Default)]
pub struct DispatcherMetrics {
    pub total_published: AtomicU64,
    pub total_dropped: AtomicU64,
    pub total_replays: AtomicU64,
    pub total_replay_packets: AtomicU64,
    pub total_source_resets: AtomicU64,
    pub total_zombie_evictions: AtomicU64,
    pub total_subscribe_rejected: AtomicU64,
    pub current_consumers: AtomicUsize,
}
```

每个消费者快照至少包含以下 Rust 字段；通过 DTO 的 `#[serde(rename_all = "camelCase")]` 输出 API 字段：

```rust
pub struct ConsumerHealthSnapshot {
    pub consumer_id: u64,
    pub kind: ConsumerKind,
    pub queue_depth: usize,
    pub queue_capacity: usize,
    pub dropped_packets: u64,
    pub replay_count: u64,
    pub recovering: bool,
    pub last_progress_at_ms: i64,
}
```

每路流快照至少包含：

```rust
pub struct StreamHealthSnapshot {
    pub stream_key: String,
    pub source_state: String,
    pub source_epoch: u64,
    pub last_packet_at_ms: i64,
    pub reconnect_count: u64,
    pub active_consumers: usize,
    pub max_consumers: usize,
    pub gop_packets: usize,
    pub gop_bytes: usize,
    pub consumers: Vec<ConsumerHealthSnapshot>,
}
```

对外接口必须遵循现有 `/api/v1`、`camelCase`、统一响应信封、权限和 i18n 约定。可新增受保护的流健康查询接口，或并入已有系统概览，但不能建立第二套错误信封。

日志策略：

- 订阅拒绝、SourceReset、Replay、僵尸驱逐和时间戳重基准记录结构化日志。
- 逐帧丢弃不逐帧 `warn`，按流和消费者做计数/采样，避免日志反过来阻塞服务。
- 日志中只使用 `streamKey`、consumer kind 和内部 id，不输出 RTSP 凭证或完整 URL。

建议告警条件：

- 源流静默超过 watchdog 阈值。
- 单流或全局消费者达到 80% 上限。
- 丢帧率持续超过配置阈值。
- Replay 频率持续异常。
- GOP 快照因字节上限频繁失效。
- 任务、fd 或 RSS 在长稳期间持续增长。

---

## 12. WS-FLV 移除

当前前端实际使用 WebCodecs → HTTP-FLV，WS-FLV 与 HTTP-FLV 的消费逻辑重复。迁移确认所有外部调用方后：

```rust
// 保留
.route("/{camera_id}", get(handle_http_flv))
.route("/{camera_id}/flv", get(handle_http_flv))
.route("/{camera_id}/webcodecs", get(handle_ws_webcodecs))

// 移除
.route("/{camera_id}/ws", get(handle_ws_live))
```

删除 `handle_ws_live`、`handle_ws_flv`、`serve_ws_flv` 前，必须搜索仓库和部署配置中的外部调用，不能仅凭当前 React 组件判断兼容性。

---

## 13. 迁移步骤

### Phase 1：定义契约和限额

- 新增 `PreviewDistributionConfig`、`StreamItem`、`GopSnapshot`、`DispatcherMetrics`。
- 定义 `TooManyConsumers`、`SourceReset` 和 mailbox 关闭语义。
- 定义 `VideoDecoder::reset()` 契约。
- 先写预算、epoch、Replay 和 timestamp boundary 的单元测试。

### Phase 2：KeyframeCacheStore 与 ConsumerMailbox

- 将现有 KeyframeCache 收敛为唯一 `KeyframeCacheStore`。
- 实现有界 `VecDeque + Notify` mailbox。
- 测试满载清空、立即 Replay、无 cache 等待关键帧、SourceReset 唤醒、Drop 释放。
- 不接入 HTTP/WS，先完成纯逻辑测试。

### Phase 3：PacketDispatcher 接入 StreamHub

- `CameraStreamSession.broadcast_tx` 替换为 `dispatcher`。
- `StreamHub::subscribe` 返回带 RAII 的 `MediaSubscription`。
- `RetinaIngestor` 通过 dispatcher 发布 packet，并在异常重连前发布 SourceReset。
- 删除 broadcast 内部 KeyframeCache 监听协程。
- 保留单路物理 RTSP 复用和现有 AI lease/cooldown 生命周期。

### Phase 4：消费者迁移

依次迁移并验证：

1. HTTP-FLV。
2. WebCodecs。
3. `AnalysisPump`。
4. `PipelineManager::attach_main_stream`。

每个消费者都必须处理 `Packet`、`Replay`、`SourceReset` 三种消息，不能只把 `broadcast::Receiver` 替换成另一种 receiver。

### Phase 5：HTTP-FLV 输出和前端

- 增加 Merge Write buffer 和 flush 上限。
- 增加 FLV timestamp rebase guard。
- WebCodecs flags 增加 discontinuity 标记。
- 前端实现 decoder reset、指数退避和真实缓冲延迟显示。
- 确认稳定连接后再重置 retry attempt。

### Phase 6：资源、观测和 WS-FLV 清理

- 接入单流/全局消费者准入。
- 加入消费者 watchdog 和幂等驱逐。
- 接入 `StreamHealthSnapshot` 和受保护查询入口。
- 在确认外部调用方后删除 WS-FLV。
- TCP_NODELAY 在真实 accepted HTTP socket 上完成并验证。

### Phase 7：压力、故障和长稳验收

- 通过门禁后才能更新设计状态为“已验证”。
- 未通过的项必须在交付说明中明确标注，不用“工业级”文字掩盖测试缺口。

---

## 14. 验证门禁

### 14.1 自动化测试

```bash
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test --workspace

# 目标逻辑
cargo test --workspace -- dispatcher
cargo test --workspace -- mailbox
cargo test --workspace -- timestamp
cargo test --workspace -- source_epoch

cd web
pnpm format
pnpm lint
pnpm typecheck
pnpm test
pnpm build
```

至少覆盖：

- 多消费者互不影响。
- mailbox 满载后旧队列被清理，Replay 一定能入队。
- Replay 只出现一次，GOP 顺序和参数集完整。
- 没有完整 GOP 时不发送残缺 P/B 帧。
- SourceReset 清空旧 epoch 并唤醒等待者。
- 订阅拒绝不增加 active viewer，不泄漏 consumer。
- Drop、取消、超时驱逐并发执行时计数不下溢。
- Replay 后 FLV pipeline、WebCodecs decoder、硬件 decoder 都重新对齐。
- FLV 时间戳接近 `u32::MAX` 时不静默溢出。
- WebCodecs discontinuity flag 可被前端正确处理。

### 14.2 故障注入

- 一路客户端人为限速或暂停读取；其他客户端和 AI 不得被拖慢。
- 注入 500ms 延迟、5% 丢包和短时断网；验证源流 watchdog 与重连。
- 建立半开 TCP/后台标签页，验证无读取进展后被驱逐。
- 同一时间启动大量客户端，验证单流和全局上限拒绝。
- 模拟无关键帧、超大 IDR、损坏 SPS/PPS、codec/分辨率切换。
- 在 RTSP 重连前后验证 source epoch 和旧 cache 不串流。
- 触发服务优雅停机，确认所有 mailbox 被唤醒、任务在超时内退出。
- 使用 `strace`/eBPF 测量 HTTP-FLV 合并前后的 write/chunk 行为；不把测试工具输出当成理论结论。

### 14.3 长稳测试

目标负载：至少 8 路 1080P、每路 4 个预览消费者、1 个分析消费者；在目标边缘设备上持续 72 小时，发布前建议扩展到 7 天。

验收条件：

- 进程无 panic、无任务泄漏、无 fd 泄漏。
- 稳态 RSS 不持续线性增长，增长量在预先记录的预算内。
- 消费者、GOP、mailbox、source session 数量回收后回到基线。
- 单路断流不影响其他路，批量断流恢复不造成连接风暴。
- 丢帧、Replay、驱逐和重连指标与故障注入次数一致。
- HTTP-FLV 和 WebCodecs 在参数集切换、源流重连、长时间时间戳运行后仍能恢复播放。

---

## 15. 风险与明确取舍

| 风险/取舍 | 处理 |
|---|---|
| `VecDeque + Notify` 比裸 mpsc 代码更多 | 这是为了真正支持丢旧、清队列、Replay 和队列深度观测；裸 mpsc 无法可靠实现这些契约 |
| GOP Replay 会让恢复瞬间产生一小段突发发送 | Replay 是有界快照，使用共享 `Arc`，并受 GOP 包数/字节上限保护 |
| Merge Write 可能增加最多一个 flush 周期 | HTTP-FLV 默认 10ms；WebCodecs 不合并，低延迟路径不受影响 |
| HTTP/Hyper 不保证一个 body chunk 对应一个 write | 文档只承诺应用层合并，系统调用收益必须实测 |
| 连接上限会拒绝合法客户端 | 这是边缘设备资源保护机制；API 返回明确错误，运维可调配置 |
| 旧 decoder 可能没有 reset 能力 | 必须补充 `VideoDecoder::reset()`；无法 reset 的后端要走受限 flush/recreate，不能静默继续解码残缺参考链 |
| 复杂故障仍可能暴露浏览器兼容性差异 | WebCodecs/MSE 需在目标浏览器矩阵实测，不能仅以 Rust 单测宣布兼容 |
| 与 ZLMediaKit 的比较有范围限制 | 本设计只对齐压缩流分发稳定性；完整协议覆盖、万路并发和成熟运营能力不在目标内 |

---

## 16. 结论

这份设计可以作为工业级实时预览的**工程实现基线**，但“工业级”是验收结果，不是类型定义或架构图自带的属性。

达到生产交付前必须满足三点：

1. 用有界 mailbox 正确实现消费者隔离、残缺 GOP 清理和显式 Replay。
2. 用资源上限、源流 epoch、时间戳边界、RAII 订阅和运行时指标解决长期运行问题。
3. 通过目标硬件上的 72 小时以上长稳、断流/半开/限速/参数集突变和优雅停机验证。

在这些证据产生之前，准确表述应是：**设计覆盖了 ZLMediaKit 分发层的关键稳定性机制，具备达到工业级预览服务的路径，但尚未被验证为工业级产品。**
