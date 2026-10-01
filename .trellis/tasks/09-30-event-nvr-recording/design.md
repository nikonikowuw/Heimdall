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

## EventBus 设计（已修正）

**实现修正：不新建独立 EventBus，直接复用 `PipelineManager.analysis_event_tx`。**

原设计拟新建 `EventBus` 结构，但实现时发现项目已有等价的基建：
`PipelineManager::analysis_event_tx` 是 `tokio::sync::broadcast::Sender<PipelineAnalysisEvent>`，
已广播 `Alarm` / `Capture` 事件，且 `subscribe_analysis_events()` 已对外开放。

新建第二套事件机制会造成：
- 重复基建，两个事件源需同步维护
- 未来告警上报（Webhook/MQTT）必须二选一，产生歧义

**正确做法**：给 `PipelineAnalysisEvent` 增加筛选订阅帮助函数，
`RecordingWorker` 的触发桥接任务通过它接收 `Alarm` 并转换为 `RecordingTrigger`。

### 触发源修正：抓拍不得驱动录像

原设计将 `Capture`（航迹结算）与 `Alarm` 并列作为触发源，实现后发现该口径不可接受：

- `Capture` 是**客观通行事实**，每条轨迹结算都会产生，与是否命中底库无关；
- 以它为触发源，录像会被无差别写满（真机实测：443 条抓拍 → 每通道数十段录像），
  绝大多数片段没有任何检索价值，反而挤压存储并淹没真正有意义的证据；
- 录像证据应只收敛于**告警**与**有识别结果的记录**。

**修正后的触发口径**：

| 触发源 | 事件类型 | 事件 ID | 接线位置 |
|--------|----------|---------|----------|
| 规则告警 | `alarm` | `alarm_records.event_id` | 事件桥接（`to_trigger`） |
| 1:N 识别命中 | `recognition` | `recognition_records.recognition_id` | 识别对账落库后直接投递 |

识别命中的触发不在事件桥接中产生，因为“命中”判定属于识别对账职责：
`CaptureDispatchService` 在 1:N 比对落库成功后调用
`RecordingDispatchService::notify_recognition()`，从而只有真正落库的识别结果才带录像。

这也使 `recording_events.event_id` 回到实体注释声明的语义
（引用 `alarm_records.event_id` 或 `recognition_records.recognition_id`），
修正了先前误存 `capture_id` 的偏离。

```rust
// 桥接：Tokio 侧订阅广播 → std channel → RecordingWorker OS 线程
// 只接告警；抓拍在此处被显式忽略
fn to_trigger(camera_id: &str, event: &PipelineAnalysisEvent) -> Option<RecordingTrigger> {
    match event {
        PipelineAnalysisEvent::Alarm(alarm) if alarm.camera_id == camera_id => {
            Some(RecordingTrigger {
                event_id: alarm.event_id.clone(),
                event_type: RecordingEventType::Alarm,
                event_time_ms: alarm.timestamp,
            })
        }
        _ => None,
    }
}

// 识别命中：由识别对账落库后投递（非阻塞，通道不可用即丢弃，绝不反压识别链路）
recording_service
    .notify_recognition(&event.camera_id, &recognition_id, event.timestamp)
    .await;
```

**时序影响**：识别在抓拍结算后经攒批（500ms）+ 比对才落库，
触发到达 Worker 时比事件时刻晚约 0.5～2s；`event_time_ms` 仍取抓拍帧 PTS，
前置缓冲（`preCaptureSeconds` 5～30s）足以覆盖，文件内偏移语义不变。

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
  │     ├── StreamHub.subscribe(ConsumerKind::Recording) → StreamSubscription
  │     ├── 桥接 task: analysis_event_tx → std mpsc Sender<RecordingTrigger>
  │     ├── std::thread::spawn RecordingWorker
  │     │     └── 入参: subscription, trigger_rx, config, data_dir, on_finished
  │     └── 保存 RecordingWorker 句柄
  │
  ├── Pipeline 运行...
  │
App 停机
  │
  ├── RecordingWorker::request_stop()
  ├── Worker 排空 trigger_rx → 闭合当前录像 → 退出
  └── Drop 时看护线程有界 join（不阻塞 Tokio）
```

### 关键契约（实现中确立）

| 契约 | 说明 |
|------|------|
| `recv_blocking` 返回值 | 必须区分 `Item` / `Timeout` / `Closed`，超时不能当作关闭 |
| 停机顺序 | 先排空 `trigger_rx` 再检查 stop，否则最后一个事件的合并关联丢失 |
| drain 语义 | `drain_from_keyframe` 取**最早**关键帧，最大化前置覆盖 |
| 前置缓冲复用 | 文件闭合后前置缓冲重新积累，但当前实现直接复用（不 clear，避免丢失连续数据） |

## 已知缺陷：B 帧流的时间戳错乱

真机验证时发现，**含 B 帧的码流录出的文件时序错乱**（与触发源无关，属 fMP4 写入路径既有问题）：

```
含 B 帧源（High profile, has_b_frames=2）：
  ffmpeg -f null：non monotonically increasing dts   ← 告警
  输出样本时序：0.00, 0.16, 0.32, 0.32, 0.32, 0.32, ...  ← 零长度样本
无 B 帧源（Main profile, has_b_frames=0）：
  零错误
```

**根因**：`EncodedPacket` 只携带 `pts_ms`，**没有 DTS 字段**；
`FMP4Writer::push` 用相邻包 PTS 差值计算 sample duration 并把负数 `.max(0)` 截为 0。
B 帧流在解码（到达）顺序下 PTS 非单调，负差值被截断成零长度样本，
累积出重复 DTS，违反 MP4 对解码时序单调性的要求。

**影响面**：现网摄像头为 Main profile 无 B 帧，实测全部录像零错误，不受影响；
但具备 B 帧的主流机型会录出时序错乱的文件（可解码但帧序/seek 不可靠）。

**正确修法**（需牵动 media 层包结构，故未随本次改动一并处理）：

1. `EncodedPacket` 增加 `dts_ms`，由接入层（RTSP 客户端）填充；
2. fMP4 写入改用 DTS 构建 `tfdt` / `trun` 时间轴；
3. PTS 与 DTS 的偏差写入 `trun` 的 `sample_composition_time_offset`（CTTS 语义），
   才能同时保留解码顺序与可重排的显示顺序。

临时规避：码流侧关闭 B 帧（`x264 --bframes 0` / 摄像头编码配置 Main profile）。

## 前端集成

### 录像配置 UI

- 位置：`CameraDetailDrawer` 内新增分组卡片
- 组件：Toggle 开关 + 折叠的 pre/post 秒数数字输入
- API：复用现有 camera update 接口，扩展 `recording_config` 字段

### 告警 / 识别回放 UI

回放入口与触发源一一对应：**有录像的事件才显示播放器**。

| 浮层 | 查询键 | 说明 |
|------|--------|------|
| `AlarmLightboxModal` | `by-event/alarm/{eventId}` | 规则告警回放 |
| `RecognitionReviewModal` | `by-event/recognition/{recognitionId}` | 识别命中回放 |
| `CaptureLightboxModal` | — | **无回放入口**：抓拍不再触发录像 |

- 条件渲染：仅当后端返回关联录像（`data` 非 null）时才出现“证据图 / 录像回放”切换；
  “查不到录像”与“查询失败”是两种状态，后者必须给出可操作的重试。
- 播放：`<video>` 直链 fMP4（后端支持 Range），元数据就绪后按 `offsetMs` 自动定位到事件时刻；
  抓拍灯箱等带滚轮缩放的浮层需在切到视频时关闭缩放钩子（原生 wheel 监听拦不住冒泡）。
- 导出：fetch fMP4 → **mediabunny** 转标准非分片 MP4 → Blob 下载。

> 实现修正：mp4box.js 不可用。其 `addSample` / `getBuffer` 本身就是分片写入路径，
> 产物仍是 `moof` 序列（实测 290 个 moof + `mvex`，`nb_frames` 不可读），
> 拿不到带完整样本表的普通 MP4。改用 mediabunny 后，
> 真机样本（9.7s / 54.5s / 截断 51.0s）均产出无 `moof`/`mvex`、
> 含 `stts`/`stsz`/`stco`/`stsc`/`stss` 的标准 MP4，时长一致且零解码错误。
> mediabunny 体积较大而导出是低频操作，故按需 `import()`，避免拖累告警页首屏
>（实测 AlarmsPage chunk 700KB → 126.8KB）。

### API 端点

| Method | Path | 描述 |
|--------|------|------|
| GET | `/api/v1/recordings/:id` | 获取录像元数据 |
| GET | `/api/v1/recordings/:id/file` | 获取录像文件（支持 Range） |
| GET | `/api/v1/events/:eventType/:eventId/recording` | 通过事件查关联录像 |
| DELETE | `/api/v1/recordings/:id` | 手动删除录像 |
