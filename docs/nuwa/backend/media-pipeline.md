# 媒体管线

分析码流选择权完全由用户决定（支持 `main` 高清分析、`sub` 低能耗、`auto` 智能自适应）。用户指定主码流时，系统常驻硬件解码主码流，将解码后的原生帧直接下发至算法包，由算法包自行负责硬件预处理（RGA/VPC/AIPP 缩放、裁切、色彩转换）至其模型所需输入尺寸与格式，快照直接从常驻解码的高保真帧环形缓冲中零解码读回；指定子码流时走双流能效分工，主码流裸 NALU 进入有界 `RingBuffer`，告警时按需解码关键帧；指定自动适配时，若无子码流或探测不可达则自适应降级主流。无论单流还是双流，底层均由 `StreamHub` 统一复用底层物理连接。主码流分析态与压缩包证据环互斥：`main`（含 `auto` 探活失败降级主流）下分析泵已独占消费并常驻解码该路码流，不得再挂主码流裸 NALU 消费者，证据只能取自推理原生帧或常驻解码环；未命中同刻帧即直接失败，不退化为主码流 GOP 追帧；失败后的处置属结算侧回退链（内存候选 → 当帧快照），只有全部取证手段都失败才不落库（无图不成行，见 [数据库规范](./database-guidelines.md)），并记 WARN 供计数。

## 三条路径

| 路径                      | 允许的处理                                                                       |
| ------------------------- | -------------------------------------------------------------------------------- |
| `infer_fast_path`         | 硬解输出 → 设备预处理 → NPU；原生 buffer 传递，不做 CPU 像素复制、色彩转换或软解 |
| `snapshot_readback_path`  | 告警/人工抓拍时按需触发，优先在设备侧完成裁剪与硬件 JPEG 编码（MPP/VT/DVPP），仅压缩后 bitstream 执行 Device→Host 回读写盘；硬件不可用时逐级平滑降级至 CPU 路径 |
| `debug_cpu_fallback_path` | 仅物理无硬件单元时 CPU 保底，显式日志标明回退                                    |

压缩网络输入仍有一次 Host→Device DMA，不能宣称“全链路零拷贝”。设备侧零拷贝仅指解码输出到推理输入。

**已知且受控的例外——算法包内部的档位化 ROI 回读**：算法包在设备侧（RGA/VPC）裁切出定长档位画布后回读其 RGB 像素，用于单帧对齐与特征提取（典型：人脸 best-shot 的 112×112 对齐图、注册提取）。它不属于上述任何一条路径，因此必须同时满足：

1. 回读量按档位封顶且与源分辨率无关（RK3568 人脸包：画布档位 ≤ 384×384，单帧 ≤ 442KB；人脸占画面过大的极端取景会退化为整帧档位，此时回读量随分辨率增长，属已知偏差）；
2. 只在低频采样帧触发，严禁逐帧回读（best-shot 受质量门限 + 采样间隔 `MIN_FUSION_FRAME_INTERVAL` 约束，失败走指数退避）；
3. RGA/VPC 与 DMA-BUF 操作在专用 OS Worker 线程内执行，不得跑在 Tokio worker 上；
4. 回读仅服务于特征提取/证据生成，**不得**反向成为常驻推理的输入来源（`infer_fast_path` 仍必须设备侧直通）。

新增任何回读都必须先在本节登记并给出量级与频率上界，否则视为违反三路径边界。

| 平台          | 解码 / 载体                              | 预处理 → 推理                  |
| ------------- | ---------------------------------------- | ------------------------------ |
| Rockchip      | MPP / DMA-BUF fd                         | RGA → RKNN                     |
| Ascend        | DVPP / device memory                     | VPC/AIPP → ACL                 |
| Apple Silicon | VideoToolbox / CVPixelBuffer + IOSurface | Metal 等设备路径 → Core ML/ANE |
| 调试 CPU      | 软件解码 / Host 内存                     | CPU 预处理 → CPU 后端          |

不能把 CPU vImage 像素处理当作设备预处理；算法包的硬件细节见 [SDK 规范](./algo-sdk-guidelines.md#硬件预处理与会话)。

## 特写取景预裁剪（Pre-crop ROI）

`precrop` 是一条**画幅**规则（决定算法看到什么），不是告警规则：它不参与 `rules.rs` 的触发判定，也不并入运动门控掩码（只画取景框的任务不得静默变成「只算框内运动」）。语义与提取规则见 `types::DetectionRule::extract_precrop_roi`：取规则集中首个有有效面积的 precrop 规则的外接矩形（多边形致 AABB），宽或高不足 `0.01` 视为未配置，生效范围为**摄像头级**且同一时刻只生效一个。

**不变式：裁切与坐标还原是同一个决策。** 分析泵在抽帧后解析「本帧实际生效的取景矩形」，与检测结果一同交给管线做局部→全景还原（`resolve_infer_frame` → `process_detections_*` 的 `roi` 入参）；`roi = None` 表示当帧未裁切，检测结果已是全景坐标，管线**不得**做任何缩放。禁止在管线侧重读共享取景状态：一旦裁切失败而映射照旧生效，全景坐标会被二次缩放，告警与证据几何全部错位（实机表现为检测框被压进取景框）。硬件对齐（偶数、16 字节 Stride、最小边）会改变实际矩形，因此还原必须使用裁切函数回传的**生效矩形**，而非配置值。

**平台矩阵（宿主只做它该做的那一份）**：

| 平台                          | 取景裁切实现                                                                                  |
| ----------------------------- | --------------------------------------------------------------------------------------------- |
| macOS（开发/演示）            | `CVPixelBuffer` / Host 内存按平面逐行 CPU 裁切，像素格式与量化范围原样保持（非 IOSurface 缓冲） |
| Linux DMA-BUF / 昇腾设备内存  | 宿主**不做** CPU 像素拷贝；取景矩形降级为全景分析并记限流 WARN（10s 至多一条，附被抑制条数）  |

设备侧零拷贝取景裁切必须由算法包经 algo-sdk（RGA/VPC/AIPP）能力完成，宿主不得在常驻推理主路径上以 memcpy 冒充硬件路径；实现时须满足「快照硬件编码与设备侧裁剪」中的 Scratchpad 约束（禁止逐帧动态分配 DMA-BUF），且不得与低频快照编码器争抢同一块设备侧缓冲。取景裁切只作用于送模输入帧：证据与快照仍取自常驻解码的全幅帧，时标不因裁切而改变。

## 帧与所有权

[FrameRef / StrideInfo](../../../crates/types/src/frame.rs) 是共享契约，不在 spec 复制结构体。

- `FrameRef` 跨线程转移所有权，RAII 持有设备句柄/池租约；不得用像素 Vec 替代硬件帧。
- 有效宽高、分配宽高、各平面 stride/offset 分开使用；不能假设 `stride == width`。
- MPP 常见横向 16/64、纵向 16 对齐（1080 可能分配为 1088）；DVPP 宽 16、高 2、行跨度 128 等约束以具体接口为准。
- 时间基准见 [全局约定](../guides/conventions.md#时间)；插件 ABI 纳秒转换仅在 [适配边界](./algo-sdk-guidelines.md#帧契约) 进行。
- 允许的 CPU readback 前后执行 DMA-BUF cache sync START/END；常驻推理路径**禁止全尺寸回读**，运动门控只允许回读设备侧降采样后的定长缩略图（Rockchip 经 RGA 出 $320 \times 180$ Y 平面 57.6KB/帧，见 [运动门控设计](../designs/motion-detection-gating-engine.md#4-跨平台-y-平面提取策略)）。

## 跨流证据时标轴

- **PTS 轴语义**：`EncodedPacket::pts_ms` 的原点由**该路物理流自己的** RTSP `PLAY` 应答（`RTP-Info`）决定。主码流与子码流各有一条独立时标轴，同一真实时刻在两路上的数值相差一个常量偏移；**跨流直接比较 PTS（包括 `abs_diff <= 容差`）恒为近似成立，无法发现错配**，这是子码流推理取主码流证据时“证据帧与推理帧对不上”的根因。
- **换算真源**：[StreamClockAnchor](../../../crates/media/src/clock.rs) 在接入层按视频帧观测 `lag = wall_arrival - pts_ms`，以固定窗口低分位数（p10，约 2 秒收敛）估计时延地板；`lag` 噪声单边（调度与网络只会让包更晚到达），均值/EMA 会系统性高估。跨流换算为 `main_axis_pts = detection_pts + (lag_analysis - lag_main)`，等价于对齐到共同墙上时间。
- **重连作废**：每次 `PLAY` 都重新协商 `RTP-Info`，接入层必须 `reset()` 锚点；未重新收敛前 `lag_ms()` 返回 `None`。
- **类型约束**：[EvidenceTarget](../../../crates/pipeline/src/snapshot.rs) 分开承载检测轴与主码流证据轴字段，禁止调用方用单个 `i64` 表达跨轴目标。`main_axis_pts_ms` 为 `None`（未标定或锚点未注入）时**必须**完全跳过主码流证据环，退回检测流自身的帧——宁可取低分辨率但时间正确的证据，也细不假定偏移为 0 产出错帧。
- **同轴退化**：主码流分析模式下分析流与主码流是同一条物理连接（主/子 URL 相同，`StreamHub` 按规范化 URL 去重），注入同一锚点实例，换算恒等（偏移 0），无需标定。
- **容差语义**：两条轴对齐后 `MAX_TARGET_FRAME_DIFF_MS` 才第一次真正表达“允许几帧偏差”；`last_on_demand_frame` 缓存位于主码流轴，复用比较必须同样是换算后的时标。

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
  - **解耦主视频与伴生音频架构（Companion Audio Player）**：安防摄像头物理上大量存在无麦克风、未启用 RTSP 音频或编码为 G.711A/U（非 AAC）的情况。若在视频主播放器中声明 `hasAudio: true`，浏览器 MSE 会为 `<video>` 同时挂载音频与视频 `SourceBuffer`；在缺少 AAC 音频流时，音频轨道永久饥饿会导致浏览器主媒体时钟死锁挂起（`waiting`/`stalled`），造成视频黑屏假死。因此系统推行以下准则：
    - **主视频通道绝对纯净**：无论是 WebCodecs（Canvas）还是 MSE FLV（`<video>`）模式，主视频播放器永远以纯视频形态工作（`hasAudio: false, hasVideo: true`，视频元素 `muted = true`），彻底杜绝音频轨道缺失或不同步拖死视觉画面；
    - **伴生独立音频通道按需启停**：预览音频遵循“默认静音、按需拉取”。前端开启声音时，由独立的伴生 `<audio>` 播放器通过 `?audio=true&video=false`（FLV Header 设置标准 `TypeFlags = 0x04`，音频-only）拉取独立 AAC FLV 流。若摄像头无音频仅伴生通道静默，主视频秒开率与流畅度受 0 影响；
    - **静音/开启切换零抖动（Zero-Flicker Audio Toggling）**：用户在界面开关声音时，仅在后台启动或销毁独立伴生音频播放器，主视频通道严禁重启、重连或重新握手，实现 0 闪烁、0 断流的工业级监控体验；
  - **视觉推理与高清证据环硬件隔离**：`AnalysisPump` 与 `MainStreamRingBuffer` 仅面向纯 Annex-B 视频流；在送往 VPU 硬件视频解码器或推入 RingBuffer 前，必须严格过滤 `StreamTag::Audio` 与非视频数据，杜绝非视频 NALU 冲撞硬件解码内核引发 crash。
- **HTTP-FLV 应用层合并写**：在 FLV tag 边界执行 `flv_merge` 缓冲（默认 10ms 周期 / 64KB 上限），降低高频小包的事件循环调度开销；时间戳计算严格单调非递减并设置 `u32` 边界守卫。
- **WebCodecs 断点标记**：二进制帧协议支持 `WEBCODECS_FLAG_DISCONTINUITY` (0x01) 标志，配合前端在 Replay 或源流重置后触发 `VideoDecoder.reset()`。
- **运维观测**：通过 `GET /api/v1/system/media/streams/{stream_key}` 获取流与消费者的实时健康快照 `StreamHealthSnapshot`。
- HTTP-FLV 支持 AVC 与 Enhanced FLV HEVC（`hvc1`）；端点见 [API](./api-guidelines.md#路由)，浏览器能力需实测。音频-only 请求使用 `video=false`，只输出 AAC FLV Tags。
- 帧/压缩包队列、GOP 丢弃及 500ms 解码停机超时见 [并发模型](./concurrency-guidelines.md)，不阻塞下游反压硬解。

## 重配、门控与健康

- DVPP/MPP 重配遵循尺寸校验和 drain-before-switch：旧通道在途帧归零后才销毁并重建池。
- [DVPP](../../../crates/media/src/decoders/dvpp.rs) 尺寸白名单为宽 `128..=3840`、高 `128..=2160`，偶数对齐；5000ms 冷却，60000ms 内 3 次变更熔断降级。
- 门控顺序为抽帧 → 运动/ROI 过滤 → NPU，不能不加预算把全帧率送入推理；门控小图必须由设备侧降采样产出（Rockchip DMA-BUF 走 RGA 缩略图），`threshold` 与 `contour_area` 的口径是**评估栅格像素**而非源分辨率像素：硬件缩略图链路下是定长缩略图（$320 \times 180$），无缩略图链路（`Host` / `CVPixelBuffer`）时就是源帧可见尺寸，因此标定必须按载体分别做（见 [运动门控设计 §1.3](../designs/motion-detection-gating-engine.md#1-设计原则)）。
- **门控评估不得跑在 Tokio Worker 上**：RGA/VPC 降采样与 DMA-BUF 回读都是平台 FFI（单次阻塞可达百毫秒级），必须由**每路一个的专用 OS Worker 线程**承接，硬件上下文在线程内常驻；请求通道有界（容量 1），调用方带超时，Worker 卡死或崩断时本路保守放行且计入 `frames_gate_bypassed`，绝不反压阻塞解码。掩模/防区位图在评估栅格上惰性光栅化，亚像素规则不得凭空消失，防区覆盖像素数低于 `contour_area` 的死区必须 `WARN` 一次。
- 门控无法判定帧载体时必须保守放行，但必须按原因类别首次告警（或初始化失败/连续失败熔断时 `ERROR`）并计入 `frames_gate_bypassed` 与 `MotionGate::bypassed_frames()`，严禁静默失效。
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
- **Scratchpad 单画板机制**：RGA 等设备侧裁剪输出采用单块最大分辨率常驻预分配 DMA-BUF（受 RGA 硬件 dst 端口 `act_w <= 2048, act_h <= 2048, vir_h <= 2048` 限制，统一预分配 1080P 约 3.1MB，超限特写自动平滑走 CPU 保底），严禁根据目标 BBox 动态分配，杜绝 CMA 连续内存碎片化与系统崩溃。
- **色彩空间防发灰**：硬件 JPEG 编码必须显式声明 BT.601 Full Range（如 `MPP_FRAME_RANGE_JPEG`），杜绝因未映射 Limited Range 导致暗部泛白与对比度下降。
- **MPP 配置顺序**：MPP JPEGE 必须在 `mpp_enc_cfg_init` 后先执行 `MPP_ENC_GET_CFG`，再设置质量、NV12 色彩范围及真实帧的宽高/stride，最后执行 `MPP_ENC_SET_CFG`；编码器构造阶段不得用零尺寸提交 `SET_CFG`。

## 录像流分发与切片封装规范

系统支持轻量级纯流复制录像能力，详细架构见 [视频录像与回放引擎设计](../designs/video-recording-and-playback-engine.md)：

- **独立消费者隔离**：录像作为 `StreamHub` 的独立消费者（`ConsumerKind::Recording`），享有专用有界 `ConsumerMailbox`。磁盘 Flush/Sync 抖动导致的慢写由私有队列丢帧自愈，严禁反压阻塞物理 RTSP 接入与 AI 分析管线。
- **纯流直封装与零转码（Direct Remuxing）**：录像仅对原始 Annex-B `EncodedPacket`（H.264/H.265/AAC）执行解复用并封装为 MP4/fMP4 容器，**0 CPU 软编、0 VPU 硬编**，仅产生极微小的 I/O 与打包开销，常驻推理主路径（`infer_fast_path`）受 0 影响。
- **配置契约与按需开启**：录像功能默认关闭（`RecordingMode::Disabled`），向前 100% 兼容。支持细粒度按需配置：
  - `EventOnly`（边缘推荐）：日常仅在内存 `PreCaptureRingBuffer` 维持 5~15 秒前置压缩流滑动窗口（单路仅消耗 3~7MB RAM，磁盘 0 写入、0 闪存磨损），规则或动检告警时触发落盘；
  - `Continuous`：全天 24/7 定长切片写入（面向外挂 HDD/SSD 场景）；
  - `Hybrid`：子码流全天连续 + 主码流高清告警。
- **I 帧强制对齐切片**：MP4 切片分割（默认 10 秒）必须以 `is_keyframe == true` 的关键帧为首包，禁止机械定时截断，确保所有切片独立无损可播。
- **重连与时序自愈**：接收到 `StreamItem::SourceReset` 时，立即闭合当前切片，新切片自新会话首个 IDR 关键帧重新建立单调时基。
- **单二进制自包含**：封装器采用纯 Rust 实现，禁止外挂调用 `ffmpeg` 或其他外部 CLI 子进程。

验证 stride/offset、URL 特殊字符脱敏、GOP 丢帧恢复、重配 drain、时间戳回跳、健康防抖、温控恢复与停机超时。
