# 并发模型规范

> Argus 混合了异步 IO 与阻塞的平台 SDK 调用。**用错模型会直接把 tokio runtime 卡死** —— 这是本项目最容易犯且后果最严重的错误。

> ⚠️ **状态：立项约定（尚未经代码验证）**
> 首批管线代码落地后需回填真实线程模型与通道容量，并删除本提示。

---

## 核心分界线

| 用 tokio async | 用专用 OS 线程 |
|---------------|---------------|
| Axum HTTP handler | 视频解码（MPP / VideoToolbox / DVPP） |
| WebSocket 推送 | NPU 推理（RKNN / AscendCL / Core ML） |
| SQLite 访问（SeaORM 异步接口） | 硬件预处理（RGA / AIPP） |
| 定时任务、配置重载 | 任何 C++ FFI 调用 |
| 控制面逻辑 | 任何单次超过 1ms 的 CPU 密集计算 |

**判断规则**：这个调用会不会阻塞当前线程超过 1 毫秒？会 → 不能出现在 async 任务里。

平台 SDK 全是同步阻塞接口，单次调用几十到几百毫秒。tokio 的工作线程数默认等于 CPU 核数，在 RK3568 上就是 4 个。一个阻塞调用就占掉 25% 的 runtime 处理能力，几个并发下去整个 HTTP 服务就没响应了。

---

## 纯 Rust-Native 全链路线程拓扑

```
[Tokio Runtime]  ── Axum HTTP/WS、SQLite WAL 批量提交、控制面调度
      │
      ├── [异步 RTSP 拉流 Task] (retina 纯异步解复用，轻量极低 CPU)
      │         │
      │         ▼ crossbeam 有界通道
      ├── [专用解码线程池] 每路一个专用 OS 线程执行硬件解码 (MPP / VideoToolbox / DVPP)
      │         │ 产出 FrameRef（零拷贝平台句柄，满足 Send + 'static）
      │         ▼ crossbeam 有界环形通道（满则丢弃最旧帧）
      ├── [门控与规则 Worker] 纯 Rust SIMD 帧差检测 + 降采样小图快速过滤
      │         │ 过滤 90% 静止帧；有效帧才送入 NPU
      │         ▼
      ├── [固定 NPU 推理 Worker] 绑定模型上下文（数量 = NPU core 数，调用 ort/coreml-rs/native 后端）
      │         │ 同步阻塞执行硬件推理，产出 RawOutput
      │         ▼
      └── [后处理与跟踪 Worker] 纯 Rust 统一 NMS + ROI/Mask/Line 判定 + ByteTrack 跟踪
                │
                ├──> [SQLite 批量落盘] (通过有界通道移交 Tokio 异步合并提交)
                └──> [WebSocket 广播] (通过 tokio::sync::broadcast 实时广播前端)
```

约定：

- **线程数量在启动时确定并固定**，运行中不动态创建。`std::thread::spawn` 只允许出现在启动装配代码里。
- **阻塞硬件与推理调用必须在专用固定线程中执行**，严禁进入 Tokio 工作线程，见后文「async 与阻塞的边界写法」。
- 推理 worker 数量 = NPU 可并行核数（RK3576 有 3 个 NPU core，RK3568 有 1 个）。开多了只会互相排队并额外占内存。
- 每路摄像头独立解码线程，一路挂掉重连不影响其它路。

---

## 通道选型

| 场景 | 用什么 | 容量策略 |
|------|--------|---------|
| 帧数据传递 | `crossbeam_channel::bounded` | **必须有界**，满了丢最旧帧并计数 |
| async ↔ 阻塞线程 | `tokio::sync::mpsc` (bounded) | 有界，用 `blocking_send` / `blocking_recv` 跨界 |
| 事件广播给多个 WS 客户端 | `tokio::sync::broadcast` | 有界，慢客户端自然掉队（`Lagged`）而非拖垮系统 |
| 共享只读配置 | `Arc<Config>` + `arc_swap` 热替换 | — |

**硬性规则**：帧路径上**禁止 `unbounded_channel`**。消费者一慢内存就无上界增长，几分钟内 OOM。见 [../guides/edge-constraints-guide.md](../guides/edge-constraints-guide.md)。

**丢帧优于积压**：帧队列满时丢弃最旧的帧，而不是阻塞解码线程。视频分析场景里，一秒前的帧已经没有价值。

### 丢旧帧通道的标准范式

Rust 通道默认在满时会阻塞或报错，严禁在解码线程中调用阻塞的 `send()` 反压硬件解码。丢弃最旧帧的标准写法：

```rust
// 解码线程向推理队列推送：满则弹出最旧帧并尝试重推
fn push_frame_drop_oldest(
    tx: &crossbeam_channel::Sender<FrameRef>,
    rx: &crossbeam_channel::Receiver<FrameRef>,
    frame: FrameRef,
    dropped_counter: &AtomicU64,
) {
    if let Err(crossbeam_channel::TrySendError::Full(overflow)) = tx.try_send(frame) {
        // 队列已满：弹出最旧一帧以让出空间
        let _ = rx.try_recv();
        dropped_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let _ = tx.try_send(overflow);
    }
}
```

在异步推理工作线程（`InferenceWorkerHandle`）中，采用单槽（容量 = 1）`Mutex<Option<InferenceJob>>` + `Notify` 模式：
新帧进入时通过 `slot.replace(new_job)` 原子弹出被插队的旧任务，旧任务的 oneshot reply 立即通知 `Err(InferError::Execution { "drop-oldest" })`，原子累加 `dropped_count`，常驻工作线程仅执行最新帧。

帧数据 `FrameRef` 必须满足 `Send + 'static`（见 [media-pipeline.md](./media-pipeline.md)），生命周期由句柄的 RAII `Drop` 归还给缓冲池。

---

## 视频流 GOP 语义感知与防花屏修剪 (GOP-Aware Leaky Queue)

在实时低延迟视频管线中，单纯按先进先出丢弃单包会导致解码器丢失参考帧，引发严重的马赛克与绿屏。丢帧机制必须具备 **GOP 拓扑感知**：
1. **GOP 尾部修剪 (GOP Tail Pruning)**：当队列饱和丢弃某个 P 帧时，自动置位 `PruningTail` 状态，后续属于同一 GOP 的残缺 P/B 帧坚决丢弃，绝不送入解码器破坏参考帧链；
2. **关键帧瞬间跳跃 (Instant Leap to IDR)**：当新的关键帧（IDR / SPS / PPS）到达时，重置为正常状态；若此时下游积压，瞬间排空积压旧帧，将延迟彻底清零；
3. **参数集常驻保护**：SPS/PPS/VPS 关键参数集绝对不丢。

---

## 专用推理常驻线程 (InferenceWorker) 铁律

1. **OS 线程绑定与单线程运行时**：每个 `InferenceWorker` 在独立的专用 OS 线程中运行，启动独立的单线程运行时 (`Builder::new_current_thread()`)。模型实例与硬件 NPU 上下文在线程内常驻，杜绝跨线程漂移。
2. **`block_in_place` 运行时风味防御**：`tokio::task::block_in_place` 只能在 `MultiThread` 运行时中执行，在 `current_thread` 运行时调用会直接 Panic。库层阻塞包装需先检查：
   ```rust
   let is_multi_thread = tokio::runtime::Handle::try_current()
       .map(|h| h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread)
       .unwrap_or(false);
   ```
3. **Panic 双重异常隔离**：同步构建期使用 `std::panic::catch_unwind` 隔离，异步 Future 执行期使用 `futures::FutureExt::catch_unwind` 隔离，底层 FFI 异常转换为 `InferError`，保证专用工作线程健康存活。
4. **停机超时隔离 (Graceful Join Timeout)**：退出时通过 `recv_timeout` 等待工作线程回收。若超过上限（如 2500ms）底层硬件调用挂起，执行超时隔离放弃，杜绝挂死守护进程。

---

## async 与阻塞的边界写法

```rust
// ❌ 在 async 任务里直接调阻塞 FFI：卡死整个 runtime worker
async fn handle(frame: FrameRef) -> Result<Vec<Detection>> {
    backend.infer(frame)   // 阻塞 80ms
}

// ❌ spawn_blocking 也不对：推理不能每次临时借线程，
//    模型和 NPU 上下文必须绑定在固定线程上常驻
tokio::task::spawn_blocking(move || backend.infer(frame)).await?

// ✅ 固定的推理线程 + 通道
//    启动时建线程，模型常驻，async 侧只发请求收结果
let (tx, rx) = tokio::sync::mpsc::channel(8);
std::thread::spawn(move || {
    let mut backend = RknnBackend::load(&spec)?;   // 模型加载一次，常驻
    while let Some(req) = rx.blocking_recv() {
        let out = backend.infer(req.frame);
        let _ = req.reply.send(out);               // oneshot 回传
    }
});
```

**关键点**：`spawn_blocking` 适合一次性阻塞任务，**不适合 Argus 的推理路径**。NPU 上下文和已加载的模型必须绑定在固定线程上（部分 SDK 还有线程亲和要求），每次借用新线程会导致重复初始化甚至崩溃。

---

## 共享状态

| 数据 | 用什么 |
|------|--------|
| 配置（读多写极少） | `Arc<ArcSwap<Config>>`，热重载时整体替换 |
| 运行时统计计数器 | `AtomicU64`，不要用 `Mutex<u64>` |
| 摄像头运行状态表 | `Arc<RwLock<HashMap<..>>>`，锁内只做查表不做 IO |
| 帧数据 | **不共享**，通过通道转移所有权 |

**规则**：

- **持锁期间禁止做 IO、FFI 调用或 `.await`**。锁的临界区只允许内存操作。
- 在 async 上下文里需要锁时用 `tokio::sync::Mutex`；在阻塞线程里用 `std::sync::Mutex`。**不要混用**。
- 优先用通道转移所有权，而不是共享 + 加锁。

---

## 优雅退出

设备会被断电或重启，退出路径必须可靠：

- 用 `tokio::sync::watch` 或 `CancellationToken` 广播关停信号。
- 每个解码/推理线程在循环里检查关停信号，收到后**释放平台资源再退出**（DMA-BUF fd、CVPixelBuffer、NPU context 都必须显式释放）。
- 主线程 `join` 所有工作线程，设置超时上限；超时则强制退出并记 `error!`。
- **不要在 `Drop` 里做阻塞清理** —— 见 [../guides/edge-constraints-guide.md](../guides/edge-constraints-guide.md)。

---

## 禁止事项

- ❌ async 任务里调用阻塞 FFI
- ❌ 帧路径上用 `unbounded_channel`
- ❌ 帧路径发送使用阻塞式 `send`（导致解码线程被下游反压卡死）
- ❌ 运行时动态 `std::thread::spawn`
- ❌ 持锁跨 `.await`
- ❌ 用 `spawn_blocking` 跑需要常驻上下文的推理
- ❌ 无退避的重连/重试循环（会把摄像头和 NPU 打爆）

---

## 待验证事项

- [ ] 各平台 SDK 的线程亲和性要求（RKNN context 是否可跨线程使用）
- [ ] 帧队列容量的实际取值（需真机测解码与推理的速率差）
- [ ] 是否需要 `rayon` 做 CPU 侧后处理（NMS）的并行化
