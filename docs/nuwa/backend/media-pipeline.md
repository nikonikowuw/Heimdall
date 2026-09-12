# 媒体管线

分析码流选择权完全由用户决定（支持 `main` 高清分析、`sub` 低能耗、`auto` 智能自适应）。用户指定主码流时，系统常驻硬件解码主码流，将解码后的原生帧直接下发至算法包，由算法包自行负责硬件预处理（RGA/VPC/AIPP 缩放、裁切、色彩转换）至其模型所需输入尺寸与格式，快照直接从常驻解码的高保真帧环形缓冲中零解码读回；指定子码流时走双流能效分工，主码流裸 NALU 进入有界 `RingBuffer`，告警时按需解码关键帧；指定自动适配时，若无子码流或探测不可达则自适应降级主流。无论单流还是双流，底层均由 `StreamHub` 统一复用底层物理连接。

## 三条路径

| 路径                      | 允许的处理                                                                       |
| ------------------------- | -------------------------------------------------------------------------------- |
| `infer_fast_path`         | 硬解输出 → 设备预处理 → NPU；原生 buffer 传递，不做 CPU 像素复制、色彩转换或软解 |
| `snapshot_readback_path`  | 告警/人工抓拍时按需触发，优先在设备侧完成裁剪与硬件 JPEG 编码（MPP/VT/DVPP），仅压缩后 bitstream 执行 Device→Host 回读写盘；硬件不可用时逐级平滑降级至 CPU 路径 |
| `debug_cpu_fallback_path` | 仅物理无硬件单元时 CPU 保底，显式日志标明回退                                    |

压缩网络输入仍有一次 Host→Device DMA，不能宣称“全链路零拷贝”。设备侧零拷贝仅指解码输出到推理输入。

| 平台          | 解码 / 载体                              | 预处理 → 推理                  |
| ------------- | ---------------------------------------- | ------------------------------ |
| Rockchip      | MPP / DMA-BUF fd                         | RGA → RKNN                     |
| Ascend        | DVPP / device memory                     | VPC/AIPP → ACL                 |
| Apple Silicon | VideoToolbox / CVPixelBuffer + IOSurface | Metal 等设备路径 → Core ML/ANE |
| 调试 CPU      | 软件解码 / Host 内存                     | CPU 预处理 → CPU 后端          |

不能把 CPU vImage 像素处理当作设备预处理；算法包的硬件细节见 [SDK 规范](./algo-sdk-guidelines.md#硬件预处理与会话)。

## 帧与所有权

[FrameRef / StrideInfo](../../../crates/types/src/frame.rs) 是共享契约，不在 spec 复制结构体。

- `FrameRef` 跨线程转移所有权，RAII 持有设备句柄/池租约；不得用像素 Vec 替代硬件帧。
- 有效宽高、分配宽高、各平面 stride/offset 分开使用；不能假设 `stride == width`。
- MPP 常见横向 16/64、纵向 16 对齐（1080 可能分配为 1088）；DVPP 宽 16、高 2、行跨度 128 等约束以具体接口为准。
- 时间基准见 [全局约定](../guides/conventions.md#时间)；插件 ABI 纳秒转换仅在 [适配边界](./algo-sdk-guidelines.md#帧契约) 进行。
- 允许的 CPU readback 前后执行 DMA-BUF cache sync START/END；常驻推理不得为 CPU 门控额外读回像素。

## 接入与分发

- [Retina 接入](../../../crates/media/src/rtsp.rs) 使用 SIMPLE/Annex B 格式，`VideoFrame::into_data()` → `Bytes/EncodedPacket`，避免重复复制压缩包。
- 通过单调时钟与帧增量映射时间戳，检测回跳和静默超时。
- 缓存最新 H.264 SPS/PPS/IDR、H.265 VPS/SPS/PPS/IRAP，客户端接入时先发参数集与关键帧。
- 强密码 URL 复用现有解析器，日志统一 `mask_rtsp_url()`。
- **多消费者隔离分发架构**：`StreamHub` 采用 `PacketDispatcher` + 每消费者独立有界 `ConsumerMailbox`（`VecDeque + Notify`），杜绝全局单一覆盖窗口导致慢读客户端拖慢正常客户端或 AI 推理：
  - **显式消息状态机**：通道传递 `StreamItem`（`Packet`、`Replay`、`SourceReset`）；邮箱满载溢出时自动清空残缺 P/B 帧并排入单一完整 `GopSnapshot`，消费者无需等待下一个自然关键帧即可实现立即无损对齐；
  - **源流 Epoch 重置**：物理 RTSP 重连时发布 `SourceReset { epoch }`，原子递增 epoch 并清空所有旧缓存与参考链，杜绝旧会话 PTS 与脏参考帧串流；
  - **RAII 资源租约与僵尸驱逐**：订阅返回携带 RAII 租约的 `StreamSubscription`，Drop 时自动注销并归还全局消费者预算；基于 `last_progress_mono_ms` 监控无进展半开连接并幂等驱逐；
  - **准入控制**：全局与单流消费者数量受 `PreviewDistributionConfig`（单流 16、全局 128、GOP 保护 8MB）强约束，溢出时返回 `TooManyConsumers` (HTTP 429)。
- **音频架构与硬件隔离红线**：
  - 采集层完整支持多轨 RTSP（AAC 音频帧封装为 `StreamTag::Audio` 进入分发总线）；
  - 预览层遵循“默认关闭、`?audio=true` 按需开启”：客户端未传参数时过滤音频以节省下行带宽；传递 `audio=true` 时在 FLV Header 设置标准 `TypeFlags = 0x05`（音频 bit 2 + 视频 bit 0），动态合成 AAC Sequence Header 与 FLV Audio Tags。浏览器不支持 H.265 MSE 时，视频仍走 WebCodecs，音频通过 `?audio=true&video=false` 的独立 AAC-only FLV 通道播放；
  - **视觉推理与高清证据环硬件隔离**：`AnalysisPump` 与 `MainStreamRingBuffer` 仅面向纯 Annex-B 视频流；在送往 VPU 硬件视频解码器或推入 RingBuffer 前，必须严格过滤 `StreamTag::Audio` 与非视频数据，杜绝非视频 NALU 冲撞硬件解码内核引发 crash。
- **HTTP-FLV 应用层合并写**：在 FLV tag 边界执行 `flv_merge` 缓冲（默认 10ms 周期 / 64KB 上限），降低高频小包的事件循环调度开销；时间戳计算严格单调非递减并设置 `u32` 边界守卫。
- **WebCodecs 断点标记**：二进制帧协议支持 `WEBCODECS_FLAG_DISCONTINUITY` (0x01) 标志，配合前端在 Replay 或源流重置后触发 `VideoDecoder.reset()`。
- **运维观测**：通过 `GET /api/v1/system/media/streams/{stream_key}` 获取流与消费者的实时健康快照 `StreamHealthSnapshot`。
- HTTP-FLV 支持 AVC 与 Enhanced FLV HEVC（`hvc1`）；端点见 [API](./api-guidelines.md#路由)，浏览器能力需实测。音频-only 请求使用 `video=false`，只输出 AAC FLV Tags。
- 帧/压缩包队列、GOP 丢弃及 500ms 解码停机超时见 [并发模型](./concurrency-guidelines.md)，不阻塞下游反压硬解。

## 重配、门控与健康

- DVPP/MPP 重配遵循尺寸校验和 drain-before-switch：旧通道在途帧归零后才销毁并重建池。
- [DVPP](../../../crates/media/src/decoders/dvpp.rs) 尺寸白名单为宽 `128..=3840`、高 `128..=2160`，偶数对齐；5000ms 冷却，60000ms 内 3 次变更熔断降级。
- 门控顺序为抽帧 → 运动/ROI 过滤 → NPU，不能不加预算把全帧率送入推理；CPU 小图门控只用于符合上述路径边界的场景。
- [StreamHub](../../../crates/media/src/stream_hub.rs) 健康必须依据真实 NALU 到达，当前探活窗口 4000ms；Degraded 容错后再判 Failed，不以协程存活代替通流。
- 健康状态保留 10000ms 容错与连续 3 次失败确认，具体探测入口见 [camera_probe.rs](../../../crates/api/src/camera_probe.rs)。

## 温控

[ThermalPolicy](../../../crates/pipeline/src/thermal.rs) 统一管理目标平台阈值、跳帧比例、迟滞和冷却观察期，不复制一套平台参数。

| 状态         | 动作                                        |
| ------------ | ------------------------------------------- |
| Normal       | 全速并允许准入                              |
| Warning      | 50% 跳帧并预警                              |
| Critical     | 75% 削峰，阻断新任务                        |
| Emergency    | 切断非关键辅流、暂停高负载推理              |
| Conservative | 传感器失败时 50% 限流、阻断准入并报维护告警 |

## 快照硬件编码与设备侧裁剪

低频证据路径（`snapshot_readback_path`）全面推行设备侧全链路硬件加速：

- **统一抽象**：通过 `media::DeviceSnapEncoder` 封装全景大图编码与设备侧裁剪特写编码，业务层统一面向 Trait 编程，底层屏蔽各平台差异。
- **单实例串行调度**：快照编码器常驻单实例（由 `SnapEncoder` 互斥锁保护排队），严禁按摄像头或并发请求创建多上下文，避免硬件单通道争抢与 CMA 内存耗尽；失败时逐级降级至 CPU。
- **几何与对齐规范**：裁剪参数统一经由 `compute_crop_roi` 纯数学算法计算，严格保障 NV12 起点与宽高偶数对齐、硬件 16 字节 Stride 对齐、硬件下限防溢出（$\ge 16\times 16$）以及向左上平移的防越界补偿。
- **Scratchpad 单画板机制**：RGA 等设备侧裁剪输出采用单块最大分辨率（如 1080P/4K）常驻预分配 DMA-BUF，严禁根据目标 BBox 动态分配，杜绝 CMA 连续内存碎片化与系统崩溃。
- **色彩空间防发灰**：硬件 JPEG 编码必须显式声明 BT.601 Full Range（如 `MPP_FRAME_RANGE_JPEG`），杜绝因未映射 Limited Range 导致暗部泛白与对比度下降。

验证 stride/offset、URL 特殊字符脱敏、GOP 丢帧恢复、重配 drain、时间戳回跳、健康防抖、温控恢复与停机超时。
