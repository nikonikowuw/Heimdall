# Implementation Plan: 子码流驱动泵与专用常驻推理线程架构

## Phase 1: 专用常驻推理线程 (`crates/infer/src/worker.rs`)
- [x] 定义 `InferenceRequest` 与 `InferenceWorkerHandle`
- [x] 实现容量为 1 的无锁丢旧帧队列（Drop-Oldest Pattern），原子累加丢帧计数
- [x] 使用 `std::thread::Builder` 启动专有 OS 线程，线程内常驻 `Box<dyn InferenceBackend>` 或 `Arc<dyn InferenceBackend>`
- [x] 封装 `std::panic::catch_unwind` 隔离底层 panic
- [x] 在 `crates/infer/src/lib.rs` 导出 `InferenceWorker` 与 `InferenceWorkerHandle`
- [x] 编写单元测试：验证正常提交推理、多并发调用下的 Drop-Oldest 丢旧帧行为、线程生命周期优雅回收

## Phase 2: 子码流驱动泵与抽帧节流器 (`crates/pipeline/src/pump.rs`)
- [x] 实现 `AnalysisFpsGovernor`（基于 PTS 的低开销时间戳步长抽帧判定）
- [x] 实现 `SubStreamAnalysisPump`：
  - 订阅 `CameraStreamSession.broadcast_tx`
  - 循环驱动 `VideoDecoder` 解码
  - 持续刷新 `CameraPipelineContext::sub_stream_fallback`
  - 抽帧送入 `InferenceWorkerHandle`
  - 推理结果送入 `PipelineManager::process_detections`
  - 基于 `CancellationToken` 的安全退出
- [x] 在 `crates/pipeline/src/lib.rs` 导出 `SubStreamAnalysisPump` 与 `AnalysisFpsGovernor`
- [x] 编写单元测试：验证抽帧节流器逻辑、驱动泵在 MockDecoder 下的数据流转

## Phase 3: 管线管理器整合与生命周期管理 (`crates/pipeline/src/manager.rs`)
- [x] 在 `PipelineManager` 中新增 `pumps: Arc<TokioRwLock<HashMap<String, SubStreamAnalysisPump>>>`
- [x] 实现 `start_analysis_pump`、`stop_analysis_pump`、`is_analysis_pump_running`、`get_analysis_pump_metrics`
- [x] 在任务停止或管线销毁时自动级联停掉对应的驱动泵
- [x] 编写管线级集成测试：驱动泵全流程贯通（模拟 RTSP 包 -> 解码 -> 推理 -> 规则触发 -> RingBuffer 靶向抓拍落盘）

## Phase 4: 全量质量门禁与 Spec 闭环
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --all-targets -- -D warnings`
- [x] `cargo test --workspace`
- [x] 验证端到端零拷贝语义与 RAII 资源回收
- [x] 验证 macOS CoreML 真实算法包（C ABI 插件 `libgeneral_detection.dylib`、`license_plate_recognition`）前向推理自测与驱动泵全链路 E2E 验证（Apple Silicon ANE/GPU）

