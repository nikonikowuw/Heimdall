# 边缘按需解码与双流高清证据抓拍 (Subtask 2)

## Goal

在 Pipeline 内部实现按需启动的硬件加速解码器（优先子码流 640×360 推理），配合主码流 NALU 内存环形队列（Ring Buffer），在告警瞬间精准抽取主码流全高清单帧（1080P/4K）并异步编码落盘。

## Requirements

1. **硬件解码器按需调度 (On-Demand Decoding Worker)**：
   - 硬件解码器（macOS VideoToolbox / Linux MPP）收敛在 `pipeline` 内部独立专属工作线程中；
   - 仅在对应摄像头的 AI 任务处于启用状态（`desired_enabled == true`）时拉起；
   - 任务关闭时立刻释放硬件解码会话与显存资源；推流端点继续使用 `StreamHub` 的原始包直通分发，互不反压。
2. **推理流优先子码流 (Sub-stream First)**：
   - 优先绑定摄像头子码流（通常为 640×360 或 720×480）喂给解码器；
   - 解码产物直接封装为原生 `FrameRef`（macOS 产出 `CVPixelBuffer`，Linux 产出 `DMA-BUF`），显存零拷贝流转至 C ABI 算法包；
   - 若摄像头未配置子码流，系统自动平滑回退至主码流。
3. **主码流内存环形缓存 (Ring Buffer)**：
   - 主码流拉流线程仅接收原始压缩数据包（H.264 / H.265 NALU），以 `Arc<EncodedPacket>` 形式压入内存 Ring Buffer；
   - 仅保留最近 2~3 秒的完整 GOP 链，内存开销控制在 10~20MB 以内，CPU/VPU 占用接近 0。
4. **靶向精准高清抽帧与异步 JPEG 编码**：
   - 告警触发瞬间（时标 $T$），发送精准抽帧指令至主码流管道；
   - 从 Ring Buffer 中定位包含 $T$ 的前置 I 帧及后续 P 帧，硬件瞬时快进解码至该时间戳的一帧高分辨率原图（1080P/4K）；
   - 异步投递至专有 I/O 线程池，调用硬件加速/SIMD 图像库（macOS ImageIO / Linux TurboJPEG）快速压缩为 JPEG；
   - 同时按目标 BBox 裁剪出一张高清特写图（Crop Image）；
   - 若主码流断线或不可用，自动降级抓取触发当帧的子码流图像，保证 100% 不漏图。

## Acceptance Criteria

- [ ] 仅当 AI 分析任务开启时，系统才拉起 VideoToolbox/MPP 硬件解码器；关闭任务后显存和解码实例立即释放。
- [ ] 4 路摄像头接入时，由于子码流推理与主码流零解码，整机 CPU 占用率稳定低于 12%，内存开销低于 200MB。
- [ ] 触发告警时，能在 50ms 内从主码流 Ring Buffer 解码出 1080P/4K 高清大图与特写抠图并落盘，图片清晰无花屏、无绿边。
