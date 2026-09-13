# Design: 边缘流媒体录像与回放引擎 (Video Recording & Playback Engine)

> **状态**: Draft（规划草案）  
> **关联规范**: [全局约定](../guides/conventions.md)、[媒体管线](../backend/media-pipeline.md)、[数据库规范](../backend/database-guidelines.md)

---

## 1. 核心架构准则

1. **绝对纯流直封装（Direct Remuxing）**：
   仅解复用并再封装原始 `EncodedPacket`（H.264/H.265/AAC）为 MP4/fMP4 切片，**0 CPU 软编、0 VPU 硬编**，AI 实时推理算力受 0 挤占。
2. **切片首包 I 帧严格对齐**：
   每个切片首包必须是关键帧（IDR/IRAP），切片时长约 10 秒；禁止机械硬截断，确保任意切片独立可播。
3. **独立有界 Mailbox 隔离**：
   录像作为 `StreamHub` 的独立消费者（`ConsumerKind::Recording`）；磁盘 I/O 阻塞或写入抖动由私有队列丢帧自愈，严禁反压阻塞底层拉流与 AI 推理。
4. **纯 Rust 内嵌交付**：
   单二进制闭环，不依赖系统外挂 `ffmpeg` 等外部命令行子进程。

---

## 2. 模式与内存预算

### 2.1 录像工作模式
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RecordingMode {
    #[default]
    Disabled,   // 默认关闭，0 视频写盘
    EventOnly,  // 边缘推荐：日常纯内存 5~15s 滑动窗口（磁盘 0 写入），告警时刷盘
    Continuous, // 24/7 全天切片录像（面向外挂 HDD/SSD）
    Hybrid,     // 子码流全天连续 + 主码流告警高清
}
```

### 2.2 15 秒前置预录内存账本（压缩包 vs 裸像素）
内存环仅缓存压缩网络包（`EncodedPacket`），严禁缓存解码后 NV12 裸像素：
- 1080P H.265 (2 Mbps)：15 秒纯码流仅需 **~3.7 MB** RAM；
- 8 路全开 15 秒预录总内存仅消耗 **~30 MB**（< 0.5% 系统内存），以极低内存换取板载 eMMC 闪存零磨损。

---

## 3. 数据流拓扑

```text
[ StreamHub ] ──► [ Consumer: Recording (独立 Mailbox) ]
                         │
                         ▼
             [ RecordingCoordinator ]
             ├─ Epoch 时钟校验 (重连触发 SourceReset 立即封口切片)
             └─ I 帧对齐判定 (10s 周期 + is_keyframe)
                         │
       ┌─────────────────┴─────────────────┐
       ▼                                   ▼
[ Continuous 模式 ]                [ EventOnly 模式 ]
(全包直推 MP4 切片)               (写入内存 PreCaptureRingBuffer)
       │                                   │ (收到告警事件)
       │                                   ▼
       │                    (提取 T-前置秒 至 T+后置秒)
       └─────────────────┬─────────────────┘
                         ▼
             [ 专用 OS 阻塞写入 Worker ]
             (聚合 64~128KB 页面写入 MP4 文件)
                         │
                         ▼
             [ var/data/recordings/{cam}/ ]
                         │
                         ▼
             [ SQLite record_segments 登记 ]
                         │
                         ▼
             [ StorageCleaner 两阶段级联淘汰 ]
```

---

## 4. 数据契约与持久化

### 4.1 数据库表结构 (`record_segments`)
```sql
CREATE TABLE record_segments (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    camera_id     TEXT NOT NULL,
    stream_type   TEXT NOT NULL DEFAULT 'main',
    start_time_ms INTEGER NOT NULL,
    end_time_ms   INTEGER NOT NULL,
    duration_ms   INTEGER NOT NULL,
    file_path     TEXT NOT NULL, -- 统一相对路径
    file_size     INTEGER NOT NULL,
    has_motion    INTEGER NOT NULL DEFAULT 0,
    has_alarm     INTEGER NOT NULL DEFAULT 0,
    alarm_ids     TEXT,          -- JSON 数组 ["alm_1", "alm_2"]
    status        TEXT NOT NULL DEFAULT 'ready',
    created_at_ms INTEGER NOT NULL
);

CREATE INDEX idx_record_segments_cam_time ON record_segments(camera_id, start_time_ms, end_time_ms);
CREATE INDEX idx_record_segments_evict ON record_segments(status, has_alarm, start_time_ms);
```

### 4.2 API 契约
1. **时间轴元数据检索**：`GET /api/v1/cameras/{camera_id}/recordings/timeline?startMs={ts}&endMs={ts}&streamType=main`
2. **点播切片流**：`GET /api/v1/recordings/{segment_id}/stream`（必须支持 HTTP 206 Partial Content，规范化路径防穿越）。
3. **跨切片合并导出**：`POST /api/v1/recordings/export`（指定起止时间，纯 Rust 串联多个 10 秒切片）。

---

## 5. 存储防护与嵌入式可靠性

1. **eMMC 寿命防护**：
   - 无外挂大容量存储时，强制默认 `EventOnly` 模式；
   - 用户态聚合 64KB~128KB 批量落盘，降低写放大；
   - 联动 `/sys/block/mmcblk*/device/life_time` 探测磨损，损耗达 80% 触发运维预警。
2. **断网重连与时序突变自愈**：
   收到 `StreamItem::SourceReset` 时立即闭合当前切片，新切片强制等待下一个自然 IDR 关键帧重新对齐单调时钟，杜绝时间戳倒流。
3. **异常掉电保护**：
   采用 Fragmented MP4 (fMP4) 标准格式（每个 GOP 包含独立 `moof` + `mdat`）。断电仅丢失末尾 1~2 秒未刷盘数据，已落盘切片 100% 完整可播。
4. **两阶段原子级联淘汰**：
   接入 `StorageCleaner`。水位超限时四级淘汰：无告警过期 $\to$ 水位超限早期无告警 $\to$ 过期告警关联。执行两阶段操作（`deleting` 标记 $\to$ `.tombstone/` 原子移动 $\to$ 单事务 DB 删除 $\to$ 物理 unlink）。
