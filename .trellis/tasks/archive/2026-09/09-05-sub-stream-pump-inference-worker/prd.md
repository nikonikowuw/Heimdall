# PRD: 子码流驱动泵与专用常驻推理线程架构 (Sub-Stream Analysis Pump & Dedicated Inference Worker)

## Goal

构建工业级**子码流驱动泵（`SubStreamAnalysisPump`）**与**专用常驻推理线程（`InferenceWorker`）**架构，彻底打通系统内部“子码流拉流 -> 硬件解码 -> 抽帧节流 -> 零拷贝丢旧帧通道 -> 专用常驻推理线程 -> 跟踪与规则判定 -> 告警高清抓拍”的主干全链路数据流，消除媒体解码层与业务分析层之间的断层。

---

## Background & Problem Statement

当前系统的架构实现中存在关键断裂点：
1. **主干数据流未闭环**：
   - 媒体层已实现 RTSP 解复用（`StreamHub`）与硬件解码器（`MPP` / `DVPP` / `VideoToolbox` / `MockDecoder`），解码产出 `FrameRef`；
   - 业务管线层（`PipelineManager`）实现了空间规则引擎（ROI/Line/Mask）、ByteTrack 多目标跟踪与主码流 RingBuffer 高清大图抓拍（`process_detections`）；
   - 但**没有任何后台循环将解码产出的 `FrameRef` 持续送入推理引擎**，`InferenceBackend::detect()` 在生产路径中从未被调用。
2. **推理模型缺乏专用 OS 线程驻留机制**：
   - NPU 驱动（如华为 Ascend ACL 的 `aclrtContext`）强绑定在 OS 线程的线程局部存储（TLS）中，Tokio 的 Work-Stealing 机制会导致任务在不同 OS 线程漂移，引发崩溃；
   - RKNN 上下文非重入安全；
   - 若在 async 代码中使用 `spawn_blocking`，每次临时借用线程池线程会导致模型重复初始化或上下文失效。
3. **缺乏超载丢旧帧背压防护**：
   - 视频分析场景具有强时效性（1 秒前的旧帧毫无分析价值）；
   - 推理满载时如果阻塞通道，会反压卡死硬件解码器；若使用无界通道则会导致内存迅速暴涨（OOM）。

---

## Detailed Requirements

### R1. 专用常驻推理线程池与句柄封装 (`crates/infer/src/worker.rs`)
- **R1.1 固定 OS 线程常驻 (Context Pinning)**：
  - 启动时通过 `std::thread::Builder::new().name("infer-worker-...").spawn(...)` 创建专用 OS 线程，模型与 NPU 会话常驻该线程；
  - 严禁在 Tokio Worker 线程或临时线程池中执行 `detect()`。
- **R1.2 异常隔离与安全屏障**：
  - 单次推理使用 `std::panic::catch_unwind` 捕获异常，防止 FFI 崩溃击穿主进程；
  - 推理失败返回强类型 `InferError`，记录错误计数器。
- **R1.3 丢旧帧背压防护 (Drop-Oldest Channel)**：
  - 请求通道容量严格设为 1（或固定小容量 1~2）；
  - 当工作线程正忙且新帧到达时，自动丢弃最旧一帧（Drop-Oldest），并原子递增 `dropped_frames_total` 计数器，绝不反压阻塞上游解码器。
- **R1.4 异步交互句柄 (`InferenceWorkerHandle`)**：
  - 对外暴露清晰的异步接口：`async fn submit(&self, frame: FrameRef) -> Result<Vec<Detection>, InferError>`；
  - 支持优雅关停：Handle 释放或显式发出退出信号时，工作线程退出循环并回收资源。

### R2. 子码流分析驱动泵 (`crates/pipeline/src/pump.rs`)
- **R2.1 视频包订阅与解码驱动**：
  - 绑定单路摄像头的分析任务，订阅 `StreamHub` 广播的子码流视频数据包（`Arc<EncodedPacket>`）；
  - 持有独立的专用硬件解码器实例（`Box<dyn VideoDecoder + Send>`）。
- **R2.2 抽帧节流器 (Analysis FPS Governor)**：
  - 支持配置分析帧率（如 5/10/15 FPS 或全帧率 25/30 FPS）；
  - 所有数据包连续喂入解码器，以维护 H.264/H.265 P/B 帧参考链完整；
  - 仅命中采样时间戳周期的解码帧才送入推理线程，非采样帧直接释放，节约 NPU 算力与总线带宽。
- **R2.3 保底降级快拍源动态刷新**：
  - 驱动泵每产出一帧解码 `FrameRef`，自动同步更新至 `CameraPipelineContext::sub_stream_fallback`；
  - 确保主码流硬解配额超限或解码延迟时，始终有当前最新的子码流帧作为高清快照的保底图源。
- **R2.4 结果联动业务管线**：
  - 获得 `Vec<Detection>` 后，自动调用 `PipelineManager::process_detections`；
  - 驱动坐标映射、航迹跟踪、几何规则匹配以及告警触发时的 RingBuffer 靶向高清抓拍。
- **R2.5 优雅生命周期管理**：
  - 基于 `tokio_util::sync::CancellationToken` 控制启动与停止；
  - 支持视频断流检测与自愈重连。

### R3. 管线管理器装配与控制接口 (`crates/pipeline/src/manager.rs`)
- **R3.1 驱动泵生命周期控制**：
  - `start_sub_stream_pump(&self, camera_id: &str, session: Arc<CameraStreamSession>, backend: Arc<dyn InferenceBackend>, target_fps: u32)`；
  - `stop_sub_stream_pump(&self, camera_id: &str)`；
  - `is_sub_stream_pump_running(&self, camera_id: &str) -> bool`。
- **R3.2 运行实例状态管理**：
  - `PipelineManager` 内部通过 `Arc<RwLock<HashMap<String, PumpHandle>>>` 统一追踪各路摄像头的驱动泵运行状态；
  - 摄像头任务停用或管线销毁时，自动发出关停信号并等待 Worker 回收。

### R4. 工业级性能与零拷贝契约
- **R4.1 严守帧生命周期与零拷贝**：
  - 解码器输出的 `FrameRef` 通过通道传递所有权，全流程禁止全量 CPU 内存拷贝；
  - 利用 `FrameHandle` 的 RAII 自动管理显存或 DMA-BUF 引用计数。
- **R4.2 锁分离与低延迟**：
  - 帧路径严禁持锁执行 IO、FFI 或 `.await`；
  - 驱动泵各阶段均具备耗时监控（解码耗时、推理排队耗时、推理计算耗时、后处理耗时）。

---

## Acceptance Criteria

- [ ] `InferenceWorker` 在专用常驻 OS 线程中稳定运行，模型实例单线程常驻不迁移；
- [ ] 推理队列满时成功执行 Drop-Oldest 丢弃旧帧策略，原子计数器正确累加，无内存泄漏与死锁；
- [ ] `SubStreamAnalysisPump` 成功订阅子码流包、驱动解码器并按配置的目标 FPS 抽帧送检；
- [ ] 解码产出的每帧实时刷新 `sub_stream_fallback` 保底快照帧；
- [ ] 推理检测结果成功驱动 `PipelineManager::process_detections`，完成轨迹跟踪与规则告警；
- [ ] 告警触发后成功从主码流 `RingBuffer` 完成单帧按需抓拍；
- [ ] 驱动泵启停生命周期测试通过，取消时无线程残留，无资源句柄泄露；
- [ ] Workspace 全量验证门禁通过（`cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test --workspace`）。
