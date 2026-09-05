# Design: 子码流驱动泵与专用常驻推理线程架构

## Architecture Overview

本设计连接系统媒体层与规则引擎层，建立工业级双流驱动闭环。

```
                       Sub-Stream (RTSP H.264/H.265)
                                    │
                                    ▼
                         CameraStreamSession
                      (broadcast_tx: EncodedPacket)
                                    │
               ┌────────────────────┴────────────────────┐
               │                                         │
               ▼                                         ▼
   SubStreamAnalysisPump (Async Task)          Web Live Preview (FLV/WebCodecs)
        │
        ├─► AnalysisFpsGovernor (Target FPS 节流采样)
        │
        ▼ 连续数据包送入
   VideoDecoder (Dedicated OS / Actor Decoder: MPP / DVPP / VideoToolbox)
        │
        ▼ 产出零拷贝 FrameRef (DMA-BUF / CVPixelBuffer)
   ┌────┴────────────────────────────────────────────────┐
   │ 1. 刷新 sub_stream_fallback (保底大图源)             │
   │ 2. 若当前帧命中采样节流 -> 送入 Drop-Oldest Channel  │
   └──────────────────────┬──────────────────────────────┘
                          │ FrameRef 所有权转移 (cap=1)
                          ▼
               InferenceWorker (Dedicated OS Thread)
               - 线程常驻 (Pinning)
               - 持有 Box<dyn InferenceBackend>
               - 同步执行 backend.detect(&frame)
               - 异常隔离 (panic::catch_unwind)
                          │
                          ▼ Result<Vec<Detection>, InferError>
               PipelineManager::process_detections
               - 局部 ROI 仿射映射
               - ByteTrack 目标关联
               - 空间规则评估 (ROI 入侵 / Line 绊线)
               - 告警触发 -> 信号量配额拉取 Main Stream RingBuffer
               - 高清大图 + 特写抠图落盘 (图在案在) + WebSocket 实时广播
```

---

## Detailed Components

### 1. `InferenceWorker` 与 `InferenceWorkerHandle` (`crates/infer/src/worker.rs`)

#### 消息定义与队列
```rust
struct InferenceRequest {
    frame: FrameRef,
    reply: oneshot::Sender<Result<Vec<Detection>, InferError>>,
}

pub struct InferenceWorkerHandle {
    sender: crossbeam_channel::Sender<InferenceRequest>,
    receiver_for_drop: crossbeam_channel::Receiver<InferenceRequest>,
    dropped_frames: Arc<AtomicU64>,
    worker_thread: Option<std::thread::JoinHandle<()>>,
    shutdown_flag: Arc<AtomicBool>,
}
```

#### Drop-Oldest 无锁队列范式
```rust
impl InferenceWorkerHandle {
    pub async fn submit(&self, frame: FrameRef) -> Result<Vec<Detection>, InferError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        let req = InferenceRequest { frame, reply: reply_tx };

        // 尝试非阻塞推入有界队列（容量 = 1）
        if let Err(crossbeam_channel::TrySendError::Full(overflow)) = self.sender.try_send(req) {
            // 队列已满：弹出最旧的一个请求并丢弃，腾出位置
            if let Ok(stale) = self.receiver_for_drop.try_recv() {
                self.dropped_frames.fetch_add(1, Ordering::Relaxed);
                // 告知被丢弃任务超时/超载
                let _ = stale.reply.send(Err(InferError::Execution {
                    reason: "推理负载过高，跳过过时帧 (drop-oldest)".to_string(),
                }));
            }
            // 尝试重推新帧
            let _ = self.sender.try_send(overflow);
        }

        reply_rx.await.map_err(|_| InferError::Execution {
            reason: "推理工作线程意外终止或通道断开".to_string(),
        })?
    }
}
```

#### 工作线程主循环
- 在 `std::thread::Builder::new().name("infer-worker".into()).spawn(move || { ... })` 中运行；
- 循环从 `crossbeam_channel::Receiver` 接收请求；
- 使用 `std::panic::catch_unwind` 包装 `backend.detect(&req.frame)`；
- 收到关闭信号（或通道关闭）时跳出循环，退出前清理资源。

---

### 2. `AnalysisFpsGovernor` 抽帧节流器 (`crates/pipeline/src/pump.rs`)

- **功能**：限制进入推理链路的帧率（例如摄像头输出 25fps，但模型推理只需要 10fps 或 5fps）；
- **策略**：
  - 解码必须保持全量，否则 P 帧丢失会导致解码花屏；
  - 节流器维护上一采样帧的时间戳 `last_sampled_pts`，当 `pts - last_sampled_pts >= 1000 / target_fps` 时判定为采样帧；
  - 采样帧推入 `InferenceWorkerHandle`，非采样帧只用于更新 `sub_stream_fallback` 并立即释放。

---

### 3. `SubStreamAnalysisPump` 运行生命周期 (`crates/pipeline/src/pump.rs`)

```rust
pub struct SubStreamAnalysisPump {
    camera_id: String,
    cancel_token: CancellationToken,
    task_handle: Option<tokio::task::JoinHandle<()>>,
}

impl SubStreamAnalysisPump {
    pub fn start(
        camera_id: String,
        mut packet_rx: broadcast::Receiver<Arc<EncodedPacket>>,
        mut decoder: Box<dyn VideoDecoder + Send>,
        worker: Arc<InferenceWorkerHandle>,
        pipeline_mgr: Arc<PipelineManager>,
        target_fps: u32,
    ) -> Self { ... }
}
```

#### 驱动泵主循环 (Tokio async task)
1. 从 `packet_rx.recv()` 接收 `Arc<EncodedPacket>`；
2. 喂入 `decoder.decode(&pkt.data, pkt.pts_ms)`；
3. 若成功解码出 `FrameRef`：
   - 立即调用 `pipeline_mgr.update_sub_stream_frame(&camera_id, frame.clone()).await`；
   - 检查 `governor.should_sample(frame.timestamp)`；
   - 若命中采样，调用 `worker.submit(frame)` 异步获取检测结果；
   - 结果到达后，调用 `pipeline_mgr.process_detections(&camera_id, detections, frame.timestamp).await`；
4. 收到 `cancel_token` 信号时退出循环并 flush 解码器。

---

### 4. `PipelineManager` 挂载与联动 (`crates/pipeline/src/manager.rs`)

- 内部字段增加：`pumps: Arc<TokioRwLock<HashMap<String, SubStreamAnalysisPump>>>`；
- API：
  - `pub async fn start_analysis_pump(&self, camera_id: &str, session: &CameraStreamSession, backend: Arc<dyn InferenceBackend>, target_fps: u32)`；
  - `pub async fn stop_analysis_pump(&self, camera_id: &str)`；
  - `pub async fn stop_all_pumps(&self)`。
