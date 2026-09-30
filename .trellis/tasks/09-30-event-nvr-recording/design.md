# 事件驱动 NVR 录像 — 技术设计

## 架构总览

```
                         StreamHub Dispatcher
                               │
          ┌────────────────────┼────────────────────┐
          ▼                    ▼                    ▼
   HttpFlv/WebCodecs    Analysis/Evidence    Recording Consumer
   (现有消费者)          (现有消费者)         (新增 ConsumerKind)
                                                    │
                                              bounded channel
                                                    │
                                                    ▼
                                           RecordingWorker
                                           (专用 OS 线程)
                                            ├─ PreCaptureRingBuffer
                                            ├─ 状态机 (Idle / Recording)
                                            └─ fMP4 Writer → 磁盘
```

## Crate 归属

| 组件 | 所属 Crate | 理由 |
|------|-----------|------|
| `FMP4Writer` | `media` | 媒体封装器，与解码器/RingBuffer 同层 |
| `PreCaptureRingBuffer` | `media` | 压缩流内存环，和 `MainStreamRingBuffer` 同层 |
| `RecordingWorker` + 状态机 | `pipeline` | 管线编排，和 `AnalysisPump`/`SnapshotWorker` 同层 |
| `EventBus` | `types` 或 `pipeline` | 轻量广播，无 IO 依赖 |
| `ConsumerKind::Recording` | `media` | 已有枚举扩展 |
| `recordings` / `recording_events` 表 | `db` | 数据库迁移与 Repository |
| 录像 API handlers | `api` | HTTP 路由与 DTO |
| 录像 UI | `web` | 前端组件 |

## 数据流

### 常驻路径（Idle 状态）

```
Dispatcher ──► RecordingWorker mailbox (bounded channel)
                    │
                    ▼
            Arc<EncodedPacket> push 到 PreCaptureRingBuffer
                    │
                    ▼
            超出窗口的旧包被淘汰（永不落盘）
```

### 事件触发路径

```
Pipeline 规则判定 → EventBus.publish(EventSignal)
                          │
                          ▼
                   RecordingWorker 收到信号
                          │
                   ┌──────┴──────┐
                   │ 当前 Idle?   │
                   └──────┬──────┘
                     Yes  │  No (已在 Recording)
                     ▼         ▼
              Flush buffer    重置 post-capture 倒计时
              到 fMP4 文件    追加事件关联到 recording_events
              切换到 Recording
                     │
                     ▼
              后续实时包继续追加 fMP4 fragment
                     │
              post-capture 倒计时结束
                     │
                     ▼
              闭合文件 → DB 写入 recordings + recording_events
              切换回 Idle
```

### SourceReset 路径

```
Dispatcher ──► StreamItem::SourceReset { epoch }
                    │
              ┌─────┴─────┐
              │ Idle?      │
              └─────┬─────┘
                Yes │  No (Recording)
                ▼         ▼
          清空 buffer   闭合当前 fMP4 (status=truncated)
          等待新 KF     DB 写入 → 清空 buffer → Idle
```

## PreCaptureRingBuffer 设计

```rust
pub struct PreCaptureRingBuffer {
    queue: VecDeque<Arc<EncodedPacket>>,
    max_duration_ms: i64,        // 来自通道配置 pre_capture_seconds * 1000
    max_bytes: usize,            // 码率保护上限，防止异常码率打爆内存
    current_bytes: usize,
    keyframe_count: usize,
}
```

- 与 `MainStreamRingBuffer` 结构类似但职责不同：此 buffer 服务于录像 flush，不需要 `get_gop_for_timestamp` 精准索引。
- 核心操作：`push(pkt)` 入队 + 淘汰、`drain_all() -> Vec<Arc<EncodedPacket>>` flush 全部数据。
- drain 时必须从最近的前置 keyframe 开始，保证 fMP4 第一个 fragment 可独立解码。

## fMP4 Writer 设计

### Box 结构

```
ftyp (isom, iso5, iso6, msdh, msix)
moov
  ├── mvhd
  ├── trak
  │   ├── tkhd
  │   └── mdia
  │       ├── mdhd
  │       ├── hdlr (vide)
  │       └── minf → stbl (空 sample table)
  └── mvex
      └── trex (track_id=1)

[Fragment 1]
  moof
    ├── mfhd (sequence_number=1)
    └── traf
        ├── tfhd (track_id=1)
        ├── tfdt (base_decode_time)
        └── trun (sample_count, sizes, durations, flags)
  mdat (raw NALU payloads, length-prefixed)

[Fragment 2]
  moof + mdat
  ...
```

### Track 抽象（开闭原则）

```rust
pub trait Track {
    fn codec_config(&self) -> CodecConfig;       // avcC / hvcC
    fn handler_type(&self) -> HandlerType;        // vide / soun
    fn timescale(&self) -> u32;                   // 90000 for video
    fn track_id(&self) -> u32;
}

pub struct VideoTrack { ... }
// 未来: pub struct AudioTrack { ... }

pub struct FMP4Writer<W: Write> {
    writer: W,
    tracks: Vec<Box<dyn Track>>,
    sequence_number: u32,
}
```

### 关键约束

- NALU 从 Annex-B (startcode) 转换为 AVCC (length-prefixed) 格式写入 mdat。
- SPS/PPS/VPS 提取自首个 keyframe，写入 moov 的 avcC/hvcC box。
- fragment 以 GOP 为边界：每收到一个 keyframe 开始新 fragment，前一个 fragment 闭合。
- timescale 固定 90000，sample duration 从 PTS 差值计算。

## 数据库迁移

```sql
CREATE TABLE recordings (
    id              TEXT    PRIMARY KEY,
    camera_id       TEXT    NOT NULL REFERENCES cameras(camera_id),
    file_path       TEXT    NOT NULL,
    start_time      INTEGER NOT NULL,  -- UTC ms
    end_time        INTEGER,           -- UTC ms, NULL = 正在录制
    duration_ms     INTEGER,
    file_size       INTEGER,           -- bytes, 闭合时更新
    codec           TEXT    NOT NULL,   -- 'h264' | 'h265'
    status          TEXT    NOT NULL DEFAULT 'recording',  -- recording | completed | truncated
    created_at      INTEGER NOT NULL
);

CREATE INDEX idx_recordings_camera_time ON recordings(camera_id, start_time);
CREATE INDEX idx_recordings_status ON recordings(status);
CREATE INDEX idx_recordings_created ON recordings(created_at);

CREATE TABLE recording_events (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    recording_id    TEXT    NOT NULL REFERENCES recordings(id) ON DELETE CASCADE,
    event_type      TEXT    NOT NULL,  -- 'alarm' | 'recognition'
    event_id        TEXT    NOT NULL,
    event_time      INTEGER NOT NULL,  -- UTC ms
    offset_ms       INTEGER NOT NULL   -- 事件在录像文件内的偏移
);

CREATE INDEX idx_recording_events_recording ON recording_events(recording_id);
CREATE INDEX idx_recording_events_event ON recording_events(event_type, event_id);
```

## EventBus 设计

```rust
pub struct EventSignal {
    pub event_id: String,
    pub event_type: EventType,     // Alarm | Recognition
    pub timestamp_ms: i64,
    pub camera_id: String,
}

pub struct EventBus {
    subscribers: RwLock<Vec<crossbeam::channel::Sender<EventSignal>>>,
}

impl EventBus {
    pub fn subscribe(&self) -> crossbeam::channel::Receiver<EventSignal> { ... }
    pub fn publish(&self, signal: EventSignal) { ... }
}
```

- 使用 `crossbeam::channel::bounded` 而非 `tokio::sync::broadcast`，因为主要消费者（RecordingWorker）在 OS 线程。
- `publish` 遍历所有 subscriber sender，send 失败（满/断开）静默跳过，不阻塞 Pipeline。
- 未来 Tokio 侧的订阅者（如 WebhookNotifier）可用小型桥接 task 转发。

## 存储淘汰集成

淘汰阶梯（从先删到最后删）：

1. 无关联事件的普通行迹抓拍 (Captures)
2. **TTL 过期或水位告急的事件录像 (Recordings)** ← 新增
3. 已核验/已处理的告警证据图 (Alarms 已闭环)
4. 未处理的告警证据图 (Alarms 待处理)

录像淘汰流程：
1. 定时任务扫描 `recordings` 表，`created_at < now - ttl_days` 的记录进入候选
2. 单事务内：DELETE `recording_events` (CASCADE) → DELETE `recordings` → 删除物理文件
3. 清理空日期目录

## 线程生命周期

```
App 启动
  │
  ├── 对每个 camera（启用录像的）
  │     ├── Dispatcher 注册 ConsumerKind::Recording → 获得 mailbox rx
  │     ├── EventBus.subscribe() → 获得 event rx
  │     ├── std::thread::spawn RecordingWorker
  │     │     └── 入参: mailbox_rx, event_rx, config, data_dir
  │     └── 保存 JoinHandle
  │
  ├── Pipeline 运行...
  │
App 停机
  │
  ├── Drop mailbox tx + event tx (发送端关闭)
  ├── RecordingWorker 检测到 channel 断开
  │     ├── Recording 状态 → 闭合当前文件
  │     └── 线程退出
  └── join 所有 RecordingWorker handles
```

## 前端集成

### 录像配置 UI

- 位置：`CameraDetailDrawer` 内新增分组卡片
- 组件：Toggle 开关 + 折叠的 pre/post 秒数数字输入
- API：复用现有 camera update 接口，扩展 `recording_config` 字段

### 告警回放 UI

- 位置：告警详情 Modal/Drawer 内新增 "事件录像" tab
- 条件渲染：仅当 `recording_events` 中存在该告警的关联记录时显示
- 播放：`<video src="/api/v1/recordings/:recordingId/file#t=offsetSec">`
- 导出按钮：点击后 fetch fMP4 → mp4box.js remux → Blob 下载

### API 端点

| Method | Path | 描述 |
|--------|------|------|
| GET | `/api/v1/recordings/:id` | 获取录像元数据 |
| GET | `/api/v1/recordings/:id/file` | 获取录像文件（支持 Range） |
| GET | `/api/v1/events/:eventType/:eventId/recording` | 通过事件查关联录像 |
| DELETE | `/api/v1/recordings/:id` | 手动删除录像 |
