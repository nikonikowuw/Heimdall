# 事件驱动 NVR 录像能力

## Goal

为 Heimdall 添加按需事件录像能力：当告警（Alarm）或识别结果（Recognition）触发时，自动将事件前后可配置秒数的主码流高清画面封装为 fMP4 视频片段并持久化到磁盘，供用户在浏览器内直接回放或导出标准 MP4。

核心用户价值：事后复查视觉证据从单帧快照升级为完整视频片段，还原事件全过程。

## Background

- 现有系统在告警时仅生成全景图 + 特写图（JPEG 快照），缺乏视频级证据。
- 主码流已通过 `StreamHub` dispatcher 分发，`MainStreamRingBuffer` 保留 ~3.5 秒裸 NALU 用于快照解码，但不支持录像。
- 项目契约要求：单二进制自包含（禁外挂 ffmpeg）、磁盘写入有界、文件 IO 不在 Tokio worker 上执行。
- Spec 草案 `media-pipeline.md § 录像流分发与切片封装规范` 已有初步规划（状态：规划设计，尚未实现）。

## Requirements

### R1: 事件触发录像

- 告警或识别事件触发时，将事件前 N 秒（pre-capture）和事件后 M 秒（post-capture）的主码流压缩流封装为一个 fMP4 文件落盘。
- pre-capture 通过内存 `PreCaptureRingBuffer` 滑动窗口实现，持续缓存主码流 NALU。
- post-capture 为事件触发后继续实时写入，倒计时结束后关闭文件。

### R2: 按通道可配置

- 录像为按需功能，默认关闭（`RecordingMode::Disabled`）。
- 每个通道（摄像头）独立配置：是否启用、pre_capture_seconds、post_capture_seconds。
- 全局提供默认值（pre=10s, post=10s），通道未配置时继承全局默认。
- 时长允许范围：5~30 秒。

### R3: 事件密集合并

- post-capture 倒计时期间如有新事件触发，延长倒计时至新事件时间 + post_capture_seconds。
- 单个 fMP4 文件可关联多个事件（多对多）。

### R4: fMP4 存储格式

- 内部存储格式为 Fragmented MP4，每个 GOP 一个 fragment。
- 单 video track（H.264/H.265），V1 不含音频。
- 封装器预留 Track 抽象，支持未来添加 AudioTrack。
- fMP4 支持断电恢复：已写入的 fragment 可独立播放。

### R5: 自研极薄 fMP4 Writer

- 纯 Rust 实现，不依赖外部 crate 的 muxer 能力。
- 仅支持写入，不支持读/编辑/解析。
- 预估核心代码量 500~800 行。

### R6: 客户端导出

- 用户导出时，前端使用 `mp4box.js` 在浏览器内将 fMP4 remux 为标准 MP4 下载。
- 宿主不参与格式转换，零 CPU 负载。

### R7: 浏览器内直接回放

- 后端提供 `GET /api/v1/recordings/:id/file` 接口，支持 HTTP Range。
- 前端在告警详情内嵌 `<video>` 标签直接播放，自动 seek 到事件 `offset_ms` 位置。

### R8: 独立消费者隔离

- 录像消费者通过 `ConsumerKind::Recording` 订阅 `StreamHub` dispatcher 的主码流。
- 每个启用录像的通道拥有独立的 `RecordingWorker` 专用 OS 线程。
- 码流包和事件触发信号通过 bounded channel 送入线程。
- 磁盘慢写仅阻塞自身线程，不反压上游。

### R9: EventBus 事件总线

- Pipeline 产生事件时通过 EventBus 广播 `EventSignal`。
- RecordingWorker 作为 EventBus 订阅者接收触发信号。
- 架构面向未来告警上报（Webhook / MQTT / 邮件）等可插拔副作用扩展。

### R10: 存储淘汰

- 录像文件独立 TTL 配置，默认保留 7 天，超龄主动清理。
- 磁盘水位告急时，录像文件在淘汰阶梯中排第 2 级（普通抓拍之后，告警证据之前）。
- 淘汰遵循现有"图在案在，图销案销"原则：单事务内同步删除物理文件与 DB 记录。

### R11: 断连处理

- Idle 状态收到 `SourceReset`：清空 PreCaptureBuffer，从新 keyframe 重新积累。
- Recording 状态收到 `SourceReset`：立即闭合当前文件（标记 truncated），回到 Idle。
- 一个 fMP4 文件内时基必须单调连续，绝不跨越 SourceReset 边界。

### R12: 磁盘目录结构

- `{data_dir}/recordings/{camera_id}/{YYYY-MM-DD}/{HHMMSS}_{recording_id}.mp4`
- 按日期分层，TTL 淘汰时可整目录清理。

### R13: 数据库模型

- 新增 `recordings` 表：id, camera_id, file_path, start_time, end_time, duration_ms, file_size, codec, status, created_at。
- 新增 `recording_events` 关联表：recording_id, event_type, event_id, event_time, offset_ms。
- 多对多关系，支持一个录像关联多个事件。

### R14: UI 交互

- 录像配置放在通道详情页，独立分组卡片。
- 开关 Toggle 控制启用/禁用，启用后展开 pre/post 秒数配置。
- 告警详情页新增"事件录像"tab，内嵌播放器 + 导出按钮。

## Out of Scope (V1)

- 音频录制（封装器预留扩展口但不实现）
- 连续录像模式（Continuous）和混合模式（Hybrid）
- 告警上报能力（EventBus 预留但不实现具体上报器）
- 录像文件加密与水印
- 多路录像的全局内存预算上限管理

## Acceptance Criteria

- [ ] AC1: 启用录像的通道在告警触发时，自动生成包含事件前后指定秒数画面的 fMP4 文件
- [ ] AC2: fMP4 文件可通过浏览器 `<video>` 标签直接播放，支持 seek
- [ ] AC3: 事件密集时（post 未结束又来新事件），录像正确延长并关联多个事件
- [ ] AC4: 前端可通过 mp4box.js 将 fMP4 导出为标准 MP4
- [ ] AC5: SourceReset 时正确闭合文件，不产生时基不连续的损坏文件
- [ ] AC6: 磁盘水位告急时，录像文件按优先级被正确淘汰
- [ ] AC7: TTL 过期的录像被主动清理（文件 + DB 记录原子删除）
- [ ] AC8: 录像写盘不在 Tokio worker 线程上执行
- [ ] AC9: 录像消费者慢写不反压 RTSP 接入或 AI 分析管线
- [ ] AC10: 未启用录像的通道无任何额外内存或线程开销
