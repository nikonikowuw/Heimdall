# Design: 边缘流媒体录像与回放引擎 (Video Recording & Playback Engine)

> **状态**: Draft  
> **作者**: Heimdall Engineering  
> **日期**: 2025-07-28  
> **关联规范**: [AGENTS.md](../../../AGENTS.md)、[媒体管线](../backend/media-pipeline.md)、[并发模型](../backend/concurrency-guidelines.md)、[数据库规范](../backend/database-guidelines.md)、[API 规范](../backend/api-guidelines.md)、[存储防护与自适应淘汰](../../../crates/pipeline/src/storage_cleaner/mod.rs)

---

## 1. 背景与问题陈述

### 1.1 现状与业务痛点

Heimdall 当前核心架构聚焦于**“高并发实时流接入 (`StreamHub`) + 异构硬件零拷贝推理 (`infer_fast_path`) + 靶向低频高保真快照 (`snapshot_readback_path`)”**。

在实际工业安防与边缘智能监控落地中，客户存在强烈的“事后视频取证与连续追溯”诉求：
1. **单帧快照证据链不完整**：目前告警发生时仅抓取单帧全景大图与特写裁切图（JPEG），缺乏事发前 5~10 秒与事发后 10~30 秒的动态视频片段，难以还原人员走位、动作意图与连续行径；
2. **缺乏全天巡检回溯能力**：对于未触发预设 AI 算法规则的突发事件（如漏水、火情隐患未达标、物品遗失），系统因无连续录像而无法调取历史录像进行复盘；
3. **传统 NVR 方案割裂**：用户若要具备录像能力，通常需要并列部署一套昂贵的传统海康/大华硬件 NVR 或重量级第三方流媒体服务，造成拉流带宽翻倍、边缘设备重复堆叠与运维割裂。

### 1.2 业界标杆借鉴与机制辨析（Frigate 录像机制深度剖析）

开源 NVR 标杆 **Frigate** 在树莓派及低功耗边缘端凭借轻量级录像机制获得了巨大成功。深入剖析其机制，需明确“取其精粹，规避其工程缺陷”：

| 机制 / 维度 | Frigate 原始做法 | 在工业级边缘系统（RK3588/昇腾/Apple）上的评估 | Heimdall 本设计决策 |
| :--- | :--- | :--- | :--- |
| **视频流处理** | **纯流复制（Direct Stream Copy）**：对原始 H.264/H.265 NALU 仅做解复用与再封装，**0 二次编码** | **极致高效**：消除 CPU/GPU 编解码负载，仅产生微小的 I/O 与打包开销 | **完全采纳**：依托 `StreamHub` 直接封装原始 Annex-B `EncodedPacket`，严禁二次转码 |
| **文件切片机制** | 按固定时长（默认 10 秒）分割为 MP4 切片文件，路径形如 `%Y-%m-%d/%H/%M.%S.mp4` | **工业级标准**：切片化存储利于时间范围检索、原子删除与无缝 HLS/Range 回放 | **完全采纳**：按 GOP 关键帧对齐的 10 秒分段存储 |
| **执行载体** | Python 进程管理外挂多个 `ffmpeg` CLI 命令行子进程管道（`-c copy -f segment`） | **严重工程缺陷**：<br>1. 产生大量跨进程管道与僵尸进程隐患；<br>2. 崩溃后断流恢复机制脆弱；<br>3. 难以精确感知纳秒/毫秒时间戳与内存共享。 | **坚决摒弃**：<br>**纯 Rust 内置 Muxer 实现**，单二进制内闭环，零子进程、零外部工具依赖 |
| **事件前后缓冲 (Pre/Post-capture)** | 依赖内存切片临时驻留或基于已落盘切片的时间窗口重叠匹配 | **有效但对存储有磨损**：在事件录像模式下若仍先落盘再过滤，对板载 eMMC 磨损严重 | **超越标杆**：<br>复用/扩展现有 `MainStreamRingBuffer`，**纯内存保留前置 5~10 秒原始 GOP**，无事件绝不刷盘 |
| **存储生命周期** | 简单的目录时间扫描与无保护 `rm` 删除 | **易引发高 I/O 抖动与孤儿文件**：无法感知 Inode 耗尽与 eMMC 寿命，易因文件系统锁死崩溃 | **超越标杆**：<br>深度接入现有 `StorageCleaner`（`statvfs` 水位、eMMC 磨损探测、两阶段 tombstone 级联淘汰） |

---

## 2. 核心架构与设计原则

### 2.1 架构黄金准则

1. **绝对零二次转码（Zero Re-encoding Overhead）**：
   录像管线只允许操作原始已压缩的数据包（`EncodedPacket`）。严禁在录像链路引入任何 CPU 软解/软编或 VPU 硬解/硬编，录像吞吐仅受磁盘顺序写带宽约束，对 AI 实时分析算力**零挤占**。
2. **切片边界严格以 I 帧对齐（Keyframe-Aligned Slicing）**：
   切片分割不能以物理定时器生硬截断。每个 MP4 切片的第一帧**必须是关键帧（IDR / I-Frame）**，确保任意一个 10 秒切片均可独立送入解码器播放，杜绝首屏 1~2 秒绿屏或解码花屏。
3. **多消费者隔离与非阻塞分发（Decoupled Consumer Isolation）**：
   录像消费者（`ConsumerKind::Recording`）通过独立的 `ConsumerMailbox` 订阅 `StreamHub`。当物理磁盘由于 Flush、Sync 或文件系统清理出现高延时（如 >500ms）时，录像侧的背压必须通过其私有 Mailbox 溢出丢弃策略自愈，**严禁反压阻塞底层 RTSP 物理拉流与其他实时预览、AI 推理消费者**。
4. **单二进制与嵌入式友好（Pure Rust Single-Binary）**：
   不依赖操作系统安装 `ffmpeg`、`gstreamer` 或 `mp4box` 等外部工具链。MP4 容器封装由纯 Rust 实现，静态编译并内嵌至 Heimdall 主程序。
5. **分级存储与原子淘汰契约（Tiered Retention & Cascade Eviction）**：
   录像元数据全量落入 SQLite `record_segments` 表。存储水位预警时，遵循“无事件切片 $\to$ 过期连续切片 $\to$ 告警关联切片”的严格优先级级联清除，杜绝产生孤儿切片文件。

---

## 3. 总体数据拓扑与工作流

### 3.1 数据流架构拓扑

```
                           [ RTSP IP Camera ]
                                   │
                                   ▼
                            [ StreamHub ]
                                   │
       ┌───────────────────────────┼───────────────────────────┐
       ▼                           ▼                           ▼
[ Consumer: Preview ]       [ Consumer: AI Pump ]       [ Consumer: Recording ]
(HTTP-FLV / WebCodecs)      (VPU -> NPU 推理管线)        (录像专用有界 Mailbox)
                                                               │
                                                               ▼
                                                    [ StreamItem 分发通道 ]
                                                               │
                                                               ▼
                                                  ┌──────────────────────────┐
                                                  │   RecordingCoordinator   │
                                                  │                          │
                                                  │ 1. 录像模式判定 (模式A/B)│
                                                  │ 2. 时序单调校验 (Epoch)  │
                                                  │ 3. I 帧对齐与切片决策    │
                                                  └─────────────┬────────────┘
                                                                │
                                   ┌────────────────────────────┴────────────────────────────┐
                                   ▼                                                         ▼
                       [ Continuous 模式流水线 ]                                  [ Event-based 模式流水线 ]
                     (全包直推 MP4 切片封装器)                                   (写入内存环形队列 RingBuffer)
                                   │                                                         │
                                   │                                                         │ (收到 rules 告警事件)
                                   │                                                         ▼
                                   │                                            (回溯导出 T-前置秒 至 T+后置秒)
                                   │                                                         │
                                   └────────────────────────────┬────────────────────────────┘
                                                                ▼
                                                [ Dedicated Blocking Worker ]
                                                (纯 Rust MP4 容器封装 & 聚合写)
                                                                │
                                                                ▼
                                                [ var/data/recordings/{cam}/ ]
                                                                │
                                                                ▼
                                                [ SQLite: record_segments 登记 ]
                                                                │
                                                                ▼
                                                [ StorageCleaner 纳入级联淘汰 ]
```

---

## 4. 录像模式与触发机制

### 4.0 配置契约与可选启闭策略 (Optional Configuration)

系统严格遵循“按需开启、默认零侵入”原则。用户可按全局系统级、单摄像头级以及分析任务级分别配置录像行为。

```rust
/// 摄像机/分析任务维度的录像配置契约
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingConfig {
    /// 录像功能总开关（默认 false，保持现有零写盘轻量状态，向前 100% 兼容）
    pub enabled: bool,

    /// 录像工作模式
    pub mode: RecordingMode,

    /// 事件事前预录时长（秒，默认 10 秒，基于内存 RingBuffer 提取）
    pub pre_capture_secs: u32,

    /// 事件事后延时录制时长（秒，默认 10 秒）
    pub post_capture_secs: u32,

    /// 是否同步录制音轨（默认 false，需源流包含 AAC 音频）
    pub record_audio: bool,

    /// 录像切片保留天数（默认 30 天，受 StorageCleaner 水位强约束）
    pub retention_days: u32,
}

/// 录像工作模式枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RecordingMode {
    /// 禁用视频录像（仅保留现有静态 JPEG 快照行为，0 视频写盘）
    #[default]
    Disabled,

    /// 仅事件触发录像（边缘强烈推荐：平时 0 刷盘，告警时落盘 15~30 秒事件片段）
    EventOnly,

    /// 全天 24/7 连续录像（适合挂接大容量外部 SSD/HDD 的场景）
    Continuous,

    /// 双流混合录像（子码流全天连续低码率，告警时主码流 4K 高清录制）
    Hybrid,
}
```

当 `enabled == false` 或 `mode == RecordingMode::Disabled` 时：
1. **0 额外线程**：系统不启动该摄像头的 `RecordingWorker` 任务；
2. **0 订阅开销**：`StreamHub` 不为该通道挂载 `ConsumerKind::Recording` 消费者邮箱；
3. **0 磁盘写放大**：告警触发时严格维持当前轻量化逻辑，仅调用 `SnapshotEngine` 生成全景与特写 JPEG。

### 4.1 模式 A：连续录像 (Continuous 24/7)
- **适用场景**：外挂大容量机械硬盘（HDD）或企业级 SSD，关键重要防区（如大门、财务室）；
- **工作机制**：
  1. `RecordingCoordinator` 持续接收并打包所有 `EncodedPacket`；
  2. 当切片累计时长 $\ge 10000\,\text{ms}$ 且遇到下一个 `is_keyframe == true` 时，封口当前文件；
  3. 写入 `record_segments` 表，初始标记 `has_alarm = false`；若该时间段内有 AI 告警触发，则异步反写标记 `has_alarm = true` 并关联 `alarm_id`。

### 4.2 模式 B：事件触发录像 (Event-only / Motion-based，边缘强烈推荐)
- **适用场景**：板载 eMMC 存储受限、对写入寿命敏感的无风扇嵌入式工控机；
- **工作机制**：
  1. **日常巡航（静默期）**：数据包仅写入内存中定长的 `PreCaptureRingBuffer`（保留最近 5~15 秒），**磁盘完全不写，0 闪存损耗**；
  2. **事件触发（激活期）**：当 `MotionGate` 判定有显著运动，或 `RulesEngine` 触发越界绊线/区域入侵告警时：
     - 触发器向 `RecordingCoordinator` 发送 `EventStart { event_id, pre_secs: 10 }`；
     - 协调器立即从内存 RingBuffer 倒序提取前置 10 秒的完整 GOP，作为本段录像的起始点；
     - 持续将后续实时数据推入封装器，直至收到 `EventEnd` 并追加记录设定的 `post_secs`（如 10 秒）；
  3. **封口写盘**：生成一段完整的事件 MP4，关联 `alarm_id` 落库。

### 4.3 模式 C：双流能效混合录像 (Dual-stream Hybrid)
- **日常状态**：对低码率**子码流（Sub Stream）**执行连续 24/7 录像（文件体积极小，码率约 300~500Kbps）；
- **事件状态**：一旦 AI 检测到告警，立即激活**主码流（Main Stream 1080P/4K）**的高清事件录像切片；
- **回放呈现**：日常看低码率巡检，事件发生点自动无缝切换为 4K 高保真视频。

### 4.4 内存消耗量化数学模型 (15 秒前置预录真实开销)

许多边缘工程师担心在内存中维持 15 秒视频会耗尽 RAM。**该担心源于混淆了解码后的裸像素（Raw Pixels）与网络压缩码流（Compressed NALUs）**：

- **若是解码后 NV12 像素**：$1920\times 1080\times 1.5 = 3.1\text{ MB/帧}$，25fps 下 15 秒共 375 帧，高达 **$1.16\text{ GB}$**，在边缘端是致命反模式；
- **系统实际缓存的是原始网络压缩包 (`EncodedPacket`)**：数据大小仅由码率决定：

$$\text{Memory Usage} = \left(\frac{\text{Bitrate}}{8}\times \text{Duration (s)}\right) + (\text{Frame Count}\times \text{Struct Overhead})$$

各主流规格 15 秒前置内存缓冲实测账本：

| 规格与流类型 | 典型码率 (Bitrate) | 15 秒纯码流体积 | 375 帧结构体开销 | 单路 15s 总内存消耗 | 8 路总内存消耗 |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **1080P H.265 (主流)** | 2.0 Mbps | 3.66 MB | ~24 KB | **~3.7 MB** | **~29.6 MB** |
| **1080P H.264 (传统)** | 4.0 Mbps | 7.32 MB | ~24 KB | **~7.4 MB** | **~59.2 MB** |
| **1080P H.265 (低码率)** | 1.0 Mbps | 1.83 MB | ~24 KB | **~1.9 MB** | **~15.2 MB** |
| **4K 超高清 H.265** | 8.0 Mbps | 14.65 MB | ~24 KB | **~14.7 MB** | **~117.6 MB** |
| **子码流 (640×360)** | 512 Kbps | 0.94 MB | ~24 KB | **~0.96 MB** | **~7.7 MB** |

**结论**：在 8GB/16GB 的 RK3588 或 Atlas 200I 板卡上，即使 8 路全开 1080P H.265 15 秒预录，总内存仅占用约 **30 MB**（< 0.5% 系统内存），以极微小的内存周转代价彻底消除了无事件时对 eMMC 闪存的持续暴力写入。

### 4.5 磁盘存储体积与容量规划 (15 秒 MP4 磁盘占用与配额)

由于采用 Direct Remuxing（直封装）模式，MP4 容器的 Box 元数据（`ftyp`、`moov`、`mdat`）对于 15 秒切片仅产生 **20~35 KB** 的开销（占比 < 0.5%）。

- **单切片文件大小**：
  - 1080P H.265 (2 Mbps)：**约 3.7 MB**；
  - 1080P H.264 (4 Mbps)：**约 7.4 MB**；
  - 静态抓拍对比：单次告警全景+特写 JPEG 约 0.4~0.5 MB。**一个 15 秒完整动态连贯视频的体积仅相当于 7~9 张静态全景照片**，但证据价值提升数倍。
- **存储配额容量推算**：
  - **10 GB 存储配额**：可容纳约 **2,700 个** 15 秒 1080P 高清事件短视频（按每日 50 次告警计算，可保存 **54 天**）；
  - **32 GB 存储配额**：可容纳约 **8,600 个** 完整事件视频。

---

## 5. 详细模块设计与实现规范

### 5.1 媒体层扩展：`ConsumerKind::Recording` 与纯 Rust MP4 封装

#### 5.1.1 分发器扩展 (`crates/media/src/dispatcher.rs`)
在现有 `ConsumerKind` 枚举中新增录像类型，配置独立的 `MailboxConfig`：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConsumerKind {
    HttpFlv,
    WebCodecs,
    Analysis,
    MainStreamEvidence,
    Recording, // 新增：录像分发隔离通道
}
```

录像消费者邮箱配置：
- **容量上限**：`capacity = 512`（可缓冲约 15~20 秒 25fps 数据，应对存储 Flush 抖动）；
- **慢读溢出策略**：丢弃非关键 P/B 帧，优先保障当前正在写入切片的完整性；若严重超时，触发 `StreamItem::SourceReset` 重新对齐。

#### 5.1.2 纯 Rust MP4 容器封装器 (`crates/media/src/recording/`)
新增模块 `crates/media/src/recording/mp4_muxer.rs`，实现轻量级 ISOBMFF / fMP4 封装：

```rust
/// 录像切片封装器配置
#[derive(Debug, Clone)]
pub struct Mp4SegmenterConfig {
    /// 目标切片时长（毫秒，默认 10000ms）
    pub target_segment_duration_ms: i64,
    /// 最大允许切片时长（毫秒，默认 15000ms，防异常大 GOP 导致文件过大）
    pub max_segment_duration_ms: i64,
    /// 是否录制音频（若源流包含 AAC 且配置开启）
    pub record_audio: bool,
}

/// 录像切片产物描述
#[derive(Debug)]
pub struct CompletedSegment {
    pub file_path: PathBuf,
    pub start_pts_ms: i64,
    pub end_pts_ms: i64,
    pub duration_ms: i64,
    pub file_size_bytes: u64,
    pub video_codec: CodecType,
    pub has_audio: bool,
}
```

**核心封装技术要点**：
1. **AVCC / HVCC 转换**：
   - RTSP 接入层提供的是 Annex-B 格式（`0x00000001` 分隔符）；
   - MP4 容器要求使用 Length-prefixed 格式（NALU 前缀 4 字节大端长度）；
   - 从 SPS/PPS/VPS 中解析出 `avcC` / `hvcC` 初始化盒（Sample Description Box），写入文件头部的 `moov`（或 fMP4 的 `ftyp` + `moov`）。
2. **Moov Box 前置与 FastStart**：
   - 传统 MP4 的 `moov` 往往在文件末尾，导致浏览器必须完全下载或执行多次 Range 请求查找末尾才能开始播放；
   - 封装器采用内存累积索引，在 `finalize()` 时将 `moov` 置于 `mdat` 之前（FastStart 模式），或者采用 **Fragmented MP4 (fMP4)** 结构（`moof` + `mdat` 串行写入），实现**天然流式可播**与**异常断电零损坏**。
3. **I 帧对齐分割判定**：
   ```rust
   // 判定当前包是否应该作为新切片的起点
   let duration_so_far = current_packet.pts_ms - segment_start_pts;
   if current_packet.is_keyframe 
       && (duration_so_far >= config.target_segment_duration_ms 
           || duration_so_far >= config.max_segment_duration_ms) 
   {
       // 闭合当前切片，产生 CompletedSegment
       // 开启新切片并以当前关键帧为首包
   }
   ```

---

### 5.2 存储层设计：数据库实体与存储淘汰

#### 5.2.1 数据库模式设计 (`crates/db/src/entity/record_segment.rs`)

```sql
CREATE TABLE record_segments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    camera_id TEXT NOT NULL,
    stream_type TEXT NOT NULL,          -- 'main' | 'sub'
    start_time_ms INTEGER NOT NULL,     -- 13 位 UTC 毫秒
    end_time_ms INTEGER NOT NULL,       -- 13 位 UTC 毫秒
    duration_ms INTEGER NOT NULL,       -- 持续时长毫秒
    file_path TEXT NOT NULL UNIQUE,     -- 相对路径: recordings/{cam}/{date}/{file}.mp4
    file_size_bytes INTEGER NOT NULL,
    video_codec TEXT NOT NULL,          -- 'h264' | 'h265'
    has_audio BOOLEAN NOT NULL DEFAULT 0,
    has_motion BOOLEAN NOT NULL DEFAULT 0,
    has_alarm BOOLEAN NOT NULL DEFAULT 0,
    alarm_ids TEXT,                     -- 关联告警 ID 列表 (JSON Array 格式)
    status TEXT NOT NULL DEFAULT 'ready', -- 'ready' | 'deleting'
    created_at INTEGER NOT NULL
);

-- 时间轴查询复合索引 (覆盖 camera_id 与时间范围过滤)
CREATE INDEX idx_record_segments_camera_time 
ON record_segments (camera_id, start_time_ms, end_time_ms);

-- 状态与淘汰检索索引
CREATE INDEX idx_record_segments_eviction 
ON record_segments (status, has_alarm, start_time_ms);
```

#### 5.2.2 存储淘汰策略矩阵 (`StorageCleaner` 扩展)

将 `record_segments` 作为一级市民纳入 `crates/pipeline/src/storage_cleaner/`：

```
[ 存储水位淘汰优先级 (从最高优先淘汰 到 最低优先淘汰) ]

Level 1 (最先删) ──► 超过普通保留天数 (如 3 天) 且 has_alarm == false 的无事件切片
Level 2 (次先删) ──► 高水位预警 (Trigger Free Ratio < 15%) 时，按时间正序淘汰最早的无事件切片
Level 3 (再次删) ──► 空间极度紧缺 (Emergency Level < 8%) 时，淘汰超过告警保留期 (如 30 天) 的告警切片
Level 4 (最后防线) ─► 空间绝望水位 (Critical Level < 5%)，触发熔断，保留最近 24 小时告警，阻断新写入
```

**两阶段原子销毁**：
淘汰执行过程严格遵循 `StorageCleaner` 的两阶段状态机：
1. `mark_segments_deleting(&[id])` $\to$ 更新 DB 为 `deleting`；
2. `rename()` 物理文件至 `.tombstone/` 隔离区；
3. `delete_record_segments(&[id])` $\to$ 在单个 SQLite 事务中彻底移除记录；
4. 异步 Unlink 批量物理释放磁盘空间。

---

### 5.3 API 接口规范 (`crates/api`)

全部接口遵循统一根信封格式：`{ "code": 0, "message": "success", "data": T, "timestamp": ms }`。

#### 1. 查询时间轴分布 (Timeline Query)
- **路径**：`GET /api/v1/cameras/{camera_id}/recordings/timeline`
- **入参**：
  - `startMs`: `i64` (13位 UTC 毫秒)
  - `endMs`: `i64` (13位 UTC 毫秒)
  - `streamType`: 可选 `'main' | 'sub'`，默认 `'main'`
- **出参**：
  ```json
  {
    "code": 0,
    "message": "success",
    "data": {
      "cameraId": "cam_01",
      "segments": [
        {
          "id": 1024,
          "startTimeMs": 1722153600000,
          "endTimeMs": 1722153610000,
          "durationMs": 10000,
          "hasMotion": true,
          "hasAlarm": false,
          "alarmIds": []
        },
        {
          "id": 1025,
          "startTimeMs": 1722153610000,
          "endTimeMs": 1722153620000,
          "durationMs": 10000,
          "hasMotion": true,
          "hasAlarm": true,
          "alarmIds": ["alm_8848"]
        }
      ]
    },
    "timestamp": 1722154000000
  }
  ```

#### 2. 点播流式传输切片 (Segment Streaming)
- **路径**：`GET /api/v1/recordings/{segment_id}/stream`
- **协议契约**：
  - 必须支持标准 **HTTP 206 Partial Content**（`Range: bytes=bytes_start-bytes_end`）；
  - 路径校验必须通过 `resolve_and_verify_evidence_path()` 经过安全规范化（`canonicalize`），严格杜绝路径穿越；
  - 响应头包含：`Content-Type: video/mp4`、`Accept-Ranges: bytes`、`Content-Range`。浏览器原生 HTML5 `<video>`、MSE 或 Video.js 即可直接实现秒开与拖动定位。

#### 3. 跨切片事件导出 (Export Clip)
- **路径**：`POST /api/v1/recordings/export`
- **功能**：指定 `startMs` 与 `endMs`，后端在后台将命中的多段 10 秒 MP4 切片无缝拼接为单个连续的 MP4 导出文件，供用户一键下载取证。

---

### 5.4 Web 前端控制台可视化设计 (`web`)

前端在设备监控与告警中心深度集成录像回放体验：

1. **24 小时可缩放时间轴（Timeline Scrubber）**：
   - 交互基于 HTML5 Canvas 或 SVG，支持高帧率流畅平移与鼠标滚轮时间轴缩放（从 24 小时视图缩放到 10 分钟精细视图）；
   - **分层色块标注**：
     - 底色条：无录像区间（灰色）；
     - 普通切片：深灰色/深蓝色条块；
     - 动检命中：琥珀黄色高亮标记；
     - AI 违规告警：亮红色粗条标记；
2. **多切片平滑无缝衔接**：
   - 使用轻量前端播放控制器，通过预加载下一个切片的元数据，在当前 10 秒切片播放至 9.5 秒时自动缓冲下一切片，实现无黑屏、无卡顿的连续播放；
3. **严格遵循设计规范**：
   - 遵照全局规则，界面**严禁使用 Emoji**，全部状态指示器采用 Lucide 矢量图标；所有时间标注与提示文本全部接入 i18n 国际化。

---

## 6. 边缘嵌入式与硬件环境约束 (Edge & System Constraints)

在 RK3588、华为昇腾 Atlas 200I、RK3568 等嵌入式计算盒上引入录像，必须严格防范以下物理陷阱：

### 6.1 eMMC 闪存磨损与写放大抑制
- **风险**：消费级 32GB/64GB eMMC 颗粒的 P/E 寿命通常仅为 1000~3000 次。若 8 路 1080P 视频以 4Mbps 持续全天写入，每天写入量可达约 350GB，eMMC 会在短短几个月内产生坏道并进入只读熔断；
- **硬核防范对策**：
  1. **默认模式约束**：在未检测到外挂 NVMe SSD 或外挂机械硬盘时，系统默认配置为**“事件触发录像（Mode B）”**，禁止盲目开启多路 24/7 全量高清录像；
  2. **Page-aligned 聚合写缓存**：MP4 写入器在用户态维持 64KB~128KB 页面对齐的内存缓冲块，批量送入系统调用，严禁单个网络小包反复触发 `write()`；
  3. **实时磨损监控**：深度联动 `detect_emmc_health()`，从 `/sys/block/mmcblk*/device/life_time` 读取 Type A/B 磨损比例，寿命损耗达到 80% 时在运维总线发出警告。

### 6.2 断网重连与时序突变隔离 (SourceReset)
- **风险**：网络抖动或摄像头重启后，RTSP 的 PTS 时间戳可能会突变为 0，或产生数小时的跨度跃迁；
- **对策**：
  - `RecordingCoordinator` 监听分发总线的 `StreamItem::SourceReset { epoch }`；
  - 一旦发生重连，无论当前切片录制了多少秒，**立即强制闭合当前切片**，新切片重新从新物理连接的第一个 IDR 帧开始建立单调时基，杜绝生成时间戳倒流、导致播放器直接 Crash 的畸形文件。

### 6.3 异常掉电安全性
- **风险**：边缘设备常被暴力拔电源。如果采用传统 MP4 结构且在关机瞬间未能将末尾 `moov` 写入磁盘，整个几百兆的文件将彻底变为不可播放的废品；
- **对策**：
  - 采用 **Fragmented MP4 (fMP4)** 标准格式，每个 GOP 即为一个独立的 Movie Fragment (`moof` + `mdat`)；
  - 即使设备在任何一个时刻断电，之前已经落盘的 Fragments 完全合法且 100% 可播，最多仅丢失断电前最后 1~2 秒的未刷盘数据。

---

## 7. 分阶段实施路线图 (Implementation Roadmap)

| 阶段 | 目标 | 核心工作项 | 验证准则 |
| :--- | :--- | :--- | :--- |
| **Phase 1** | **纯 Rust 流式 MP4 封装原型** | 1. 在 `crates/media` 中引入/实现轻量纯 Rust MP4 容器分段器；<br>2. 支持 Annex-B 转 AVCC/HVCC；<br>3. 实现首包 I 帧对齐分割逻辑。 | 单元测试模拟 Annex-B H.264/H.265 数据包输入，产出合法 MP4，通过 `ffprobe` / QuickTime 验证无报错。 |
| **Phase 2** | **分发器挂接与协调器实现** | 1. 扩展 `ConsumerKind::Recording`；<br>2. 实现 `RecordingCoordinator`，挂载至 `StreamHub`；<br>3. 建立专用阻塞线程池执行异步写盘；<br>4. 支持全天录像与事件触发录像模式。 | 单路 1080P 25fps 持续录像 1 小时，CPU 占用增量 $<1\%$，Tokio Worker 延时无劣化。 |
| **Phase 3** | **数据库持久化与存储淘汰** | 1. `crates/db` 新增 `record_segments` 实体与 Migration；<br>2. 切片生成后落库；<br>3. 扩展 `StorageCleaner`，将录像切片纳入两阶段原子淘汰。 | 人工灌满磁盘触发高低水位，系统平滑将过期无告警切片移入 `.tombstone/` 并物理删除，DB 记录同步清空。 |
| **Phase 4** | **API 服务与 Web 时间轴回放** | 1. `crates/api` 开放 Timeline 元数据与 HTTP 206 Range 点播端点；<br>2. 前端 `web` 实现 24 小时动态时间轴滑块；<br>3. 告警事件点与时间轴联动高亮。 | 在 Chrome / Safari / Edge 中拖动时间轴进度条，视频可在 200ms 内快速完成跳转并流畅播放。 |

---

## 8. 总结

引入纯 Rust 实现的边缘录像与回放引擎，补齐了 Heimdall 在**事后动态取证**与**日常巡检回溯**上的最后一块拼图。

依托现有已高度优化的 `StreamHub` 接入分发总线与 `StorageCleaner` 存储防护引擎，Heimdall 能够以**“0 二次转码开销、0 外部工具依赖、高鲁棒性存储保护”**的卓越架构，交付媲美甚至超越传统专业 NVR 的边缘多媒体体验。
