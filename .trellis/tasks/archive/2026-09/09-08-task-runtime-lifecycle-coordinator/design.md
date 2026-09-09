# Technical Design: 分析任务运行时协调与双流资源生命周期

## 1. 范围与边界

### 1.1 模块边界
- 本任务隶属于 `crates/pipeline` crate，新增 `coordinator` 模块并在 `crates/pipeline/src/lib.rs` 暴露相关契约。
- 联动 `crates/media`（`StreamHub`、`CameraStreamSession`、`VideoDecoder`）、`crates/infer`（`AlgoRegistry`、`AlgoPackage`、`InferenceWorker`）与 `crates/types`（`AnalysisTask`、`Camera`、`StreamKey` 等）。
- **非本任务范围**：HTTP 路由、SQLite 数据库读写、冷启动自动扫描启动、前端 Web 界面。

```
crates/api (后续子任务)
       │
       ▼
crates/pipeline::coordinator::TaskRuntimeCoordinator
       ├── media::StreamHub (双流会话、订阅计数、AI保活)
       ├── media::decoder::create_decoder (硬件/模拟视频解码器)
       ├── infer::AlgoRegistry -> AlgoInstance -> InferenceWorker (常驻推理工作线程)
       └── pipeline::PipelineManager (SubStreamAnalysisPump、RingBuffer、分析事件分发)
```

---

## 2. 核心数据结构与契约

### 2.1 启动参数 `StartCameraPipelineParams`
```rust
#[derive(Debug, Clone)]
pub struct StartCameraPipelineParams {
    pub camera_id: String,
    pub main_rtsp_url: String,
    pub main_codec: CodecType,
    pub sub_rtsp_url: String,
    pub sub_codec: CodecType,
    pub transport_policy: TransportPolicy,
    pub algorithm_id: String,
    pub algo_params: serde_json::Value,
    pub target_fps: u32,
    pub motion_gate_enabled: bool,
}
```

### 2.2 活跃运行时条目 `ActiveRuntimeEntry`
```rust
pub struct ActiveRuntimeEntry {
    pub camera_id: String,
    pub generation: u64,
    pub main_stream_key: String,
    pub main_attach_handle: tokio::task::JoinHandle<()>,
    pub sub_stream_key: String,
    pub sub_session: Arc<CameraStreamSession>,
    pub target_fps: u32,
    pub algorithm_id: String,
    pub algo_params: serde_json::Value,
}
```

### 2.3 分析事件出口 `PipelineAnalysisEvent` (解耦下游任务)
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceStatus {
    Ready,
    Failed,
}

#[derive(Debug, Clone)]
pub struct PipelineAlarmEvent {
    pub event_id: String,
    pub camera_id: String,
    pub alarm: TriggeredAlarm,
    pub snapshot: Option<SnapshotResult>,
    pub evidence_status: EvidenceStatus,
    pub evidence_error: Option<String>,
    pub timestamp: i64,
}

#[derive(Debug, Clone)]
pub struct PipelineTrackEvent {
    pub camera_id: String,
    pub timestamp: i64,
    pub tracks: Vec<TrackedObject>,
}

#[derive(Debug, Clone)]
pub enum PipelineAnalysisEvent {
    Alarm(Box<PipelineAlarmEvent>),
    Tracks(PipelineTrackEvent),
}
```
`PipelineManager` 暴露：
1. 实时广播通道：`analysis_events: tokio::sync::broadcast::Sender<PipelineAnalysisEvent>`，供实时流推流与 Web 控制台订阅；
2. 待持久化告警补偿队列：`pending_alarm_events: Arc<Mutex<VecDeque<PipelineAlarmEvent>>>`（上限 1024）与 `drain_pending_alarm_events`，确保下游落库 Worker 在无订阅者或广播通道 Lagged 溢出时依然可靠获取告警事实并计数丢弃。

---

## 3. 运行时生命周期与原子状态机

### 3.1 启动点火序列 (Step-by-Step Ignition)
```
1. 校验输入参数 (URL 非空有效、FPS > 0、算法ID有效)
   └─ 失败: 直接返回 ValidationError
2. 幂等性检查:
   └─ 若已有运行条目且配置一致 -> 幂等成功直接返回
   └─ 若已有运行条目但配置变更 -> 先优雅停止旧条目
3. 从 StreamHub 订阅主码流:
   let main_rx = stream_hub.subscribe(main_key, main_url, policy).await?
4. 启动主码流 RingBuffer Attach 协程:
   let main_attach = pipeline_mgr.attach_main_stream(cam_id, main_rx)
5. 获取子码流 Session 并开启 AI 保活引用:
   let sub_session = stream_hub.get_or_create_session(sub_key, sub_url, policy).await
   sub_session.acquire_ai_task()
   stream_hub.set_ai_enabled(sub_key, sub_url, policy, true).await
6. 创建子码流解码器 (wrapped in Option):
   let decoder = media::create_decoder(cam_id, sub_codec)
7. 获取算法包并在 spawn_blocking 中创建实例与 Worker:
   let pkg = algo_registry.get(algo_id)
   let algo_inst = spawn_blocking(|| pkg.create_instance(...)).await?
   let worker = infer::InferenceWorker::new(algo_inst)
8. 启动驱动泵并托管 Worker 与 Decoder:
   pipeline_mgr.start_analysis_pump_with_worker(cam_id, sub_session, decoder, worker, config).await
9. 注册 ActiveRuntimeEntry 并递增 generation
```

### 3.2 失败回滚机制 (Strict Reverse Rollback)
如果在步骤 3~8 中任意一步发生错误（例如解码器构建失败、算法包未找到、FFI 初始化异常），必须严格反向回滚：
1. 若 pump 已挂载 -> `pipeline_mgr.stop_analysis_pump(cam_id).await`
2. 若 worker 已初始化 -> `tokio::task::spawn_blocking(worker.shutdown())`
3. 若 decoder 未转移给 pump -> `tokio::task::spawn_blocking(drop(decoder))` 异步安全析构
4. 若 sub_session 已标记 -> `stream_hub.set_ai_enabled(..., false).await`
5. 若 main_attach 协程已启动 -> `main_attach.abort()`, `let _ = main_attach.await`
6. 若主流已 subscribe -> `stream_hub.unsubscribe(main_key).await`
7. 标记 `ai_active = false` 并回收闲置管线上下文 `remove_pipeline_context_if_idle(cam_id).await`。
保证启动失败后系统回归 0 泄漏初始态。

### 3.3 停止序列 (Ordered Graceful Shutdown)
1. 从 `runtimes` 映射表中移除 `ActiveRuntimeEntry`；
2. 停止子码流分析驱动泵 (`pipeline_mgr.stop_analysis_pump(cam_id).await`)，驱动泵会取消解码/推理协程、释放 `sub_session.release_ai_task()` 并在 `spawn_blocking` 中安全关闭 worker 与 decoder；
3. 调用 `stream_hub.set_ai_enabled(..., false).await`，若 AI 引用归零且无观众则自动进入 StreamHub 冷却挂起；
4. 调用 `main_attach_handle.abort()` 并带超时 `await` 确保后台写入任务退出；
5. 调用 `stream_hub.unsubscribe(main_stream_key).await`，精确释放主流订阅计数；
6. 标记 `ai_active = false` 并释放 idle decoder 与回收闲置管线上下文。

---

## 4. 并发与错误处理准则

1. **非阻塞 Tokio 规范**：
   - 算法包实例创建 `create_instance` 涉及 C ABI / dlopen / 模型载入，严禁在 Tokio worker 中直接执行，统一包裹在 `tokio::task::spawn_blocking` 中。
   - 解码器创建与 Worker 关停均设有超时控制，关停不得死锁。
2. **锁粒度控制**：
   - `runtimes` 使用 `tokio::sync::RwLock<HashMap<String, ActiveRuntimeEntry>>`。
   - 启动与停止针对单个摄像头加锁或细粒度互斥，避免全局锁阻塞多路摄像头的并发编排。
3. **主流订阅对称性**：
   - 每一次 `stream_hub.subscribe(&main_key, ...)` 成功，必须保证对应一次 `stream_hub.unsubscribe(&main_key)`。杜绝仅依靠 Receiver 丢弃导致底层 `active_viewers` 计数永久泄漏。

---

## 5. 验证策略

- **单元测试与集成测试**：
  1. 完整单路启动与停止生命周期测试（MockDecoder + MockInferBackend + 模拟 StreamHub 会话）；
  2. 启动中途注入故障回滚测试（主流正常、算法包不存在时回滚，验证 StreamHub 订阅计数归零，无孤儿协程）；
  3. 重复调用 `start_pipeline` 幂等性测试（不重复创建 pump、worker 和订阅）；
  4. 验证编码包发布后，主流 RingBuffer 持续收到数据，pump `frames_decoded` 和 `frames_inferred` 持续增加；
  5. 验证分析事件出口接收到 Track 与 Alarm 事件。
