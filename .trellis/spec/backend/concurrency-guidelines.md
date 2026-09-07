# 并发模型规范 (Concurrency Guidelines)

> 混合异步 I/O 与同步阻塞硬件 SDK 调用。
> 核心铁律：**任何超过 1ms 的阻塞操作绝对禁止进入 Tokio 异步工作线程，模型与 NPU 上下文必须常驻专用 OS 线程**。

---

## 1. 核心边界与职责分工

| 适用 Tokio 异步运行时 (`async/await`) | 必须使用独立专用 OS 线程 (`std::thread`) |
| ------------------------------------- | --------------------------------------- |
| Axum HTTP 请求与鉴权拦截 | 视频硬件解码（MPP / VideoToolbox / DVPP） |
| WebSocket 广播分发与心跳管理 | NPU 硬件推理（RKNN / AscendCL / Core ML） |
| SQLite 异步读写（SeaORM 异步接口） | 硬件 2D 图像加速预处理（RGA / VPC） |
| 定时看门狗与后台轻量巡检 | 任何平台 C/C++ FFI 驱动调用 |
| 异步 RTSP 解复用与网络包读取 (Retina) | 单次执行耗时超过 1ms 的 CPU 密集计算 |

- **Tokio 线程污染防御**：Tokio Worker 数量默认等于 CPU 物理核数（边缘平台通常仅 4 核）。一个 50ms 的硬件 FFI 阻塞调用将直接霸占 25% 的异步调度能力，导致整个 HTTP/WS 服务假死。

---

## 2. 全链路线程拓扑与常驻 Worker 契约

```text
[Tokio Runtime]  ── Axum HTTP/WS、SQLite 批量合并提交、状态机调度
      │
      ├── [异步 RTSP 拉流 Task] (Retina 纯异步 I/O，产出 EncodedPacket)
      │         │ crossbeam 有界通道
      │         ▼
      ├── [专用解码线程池] 每路摄像头绑定 1 个专属 OS 线程执行硬件解码 (产出 FrameRef)
      │         │ 有界丢旧帧队列 (crossbeam bounded / drop-oldest)
      │         ▼
      ├── [规则与门控 Worker] CPU SIMD 降采样小图帧差检测，过滤 90% 静止背景
      │         │ 有效帧送入 NPU 推理队列
      │         ▼
      └── [固定 NPU 推理 Worker] 绑定模型实例与硬件上下文（数量 == 物理 NPU core 数）
                │ 同步阻塞执行推理，产出检测结果
                ├──> [SQLite 批量写入通道] (移交 Tokio 异步合并落盘)
                └──> [WebSocket 广播通道] (tokio::sync::broadcast 实时广播)
```

- **启动期固定拓扑**：所有 OS 线程数量在应用启动装配时确定，**严禁在运行时动态 `std::thread::spawn`**；
- **推理线程固定绑定**：严禁使用 `tokio::task::spawn_blocking` 执行常驻模型推理。模型上下文与硬件 Session 必须在专用 OS 线程内常驻（具备硬件亲和性，避免重复初始化崩溃）。

---

## 3. 通道选型与丢旧帧防爆契约

| 传输场景 | 推荐选型 | 策略约束 |
| --------- | --------- | --------- |
| **帧数据传递** | `crossbeam_channel::bounded` | **必须有界**（如容量 1~4），满载时弹出丢弃最旧帧并计数告警 |
| **异步 ↔ 同步跨界** | `tokio::sync::mpsc` (bounded) | 有界，通过 `blocking_send` / `blocking_recv` 跨界 |
| **事件广播至多客户端** | `tokio::sync::broadcast` | 有界，慢客户端触发 `Lagged` 自动跳帧，绝不阻塞主推流 |
| **全局只读配置共享** | `Arc<ArcSwap<Config>>` | 读无锁无争用，热重载时原子替换 |

- **帧队列绝对禁止 `unbounded_channel`**：消费者稍慢会导致未压缩帧在内存中无界堆积，数秒内物理 OOM；
- **丢旧帧标准范式 (Drop-Oldest)**：

  ```rust
  if let Err(crossbeam_channel::TrySendError::Full(overflow)) = tx.try_send(frame) {
      let _ = rx.try_recv(); // 弹出并丢弃最旧一帧
      dropped_counter.fetch_add(1, Ordering::Relaxed);
      let _ = tx.try_send(overflow);
  }
  ```

- **GOP 拓扑感知修剪**：一旦队列饱和丢弃了某个 P 帧，后续同一 GOP 内的残缺 P/B 帧必须直接丢弃（避免解码马赛克）；直到新的 IDR 关键帧到达时重置状态并排空积压，实现延迟清零。

---

## 4. 共享状态与锁规约

- **持锁期间禁止做 IO、FFI 调用或 `.await`**：锁内只能执行纯内存字段操作，计算完立即释放；
- **锁选型严格区隔**：异步 async 上下文中使用 `tokio::sync::Mutex`；阻塞 OS 线程中使用 `parking_lot::Mutex`。**严禁在 async 跨 `.await` 处持有 std/parking_lot 同步锁**（会引发运行时死锁）；
- **计数器无锁化**：运行时统计指标（帧序号、丢帧数、字节数）必须使用 `AtomicU64`，禁止为简单计数器加 Mutex。

---

## 5. 优雅退出与停机超时隔离

- 通过 `tokio::sync::watch` 或 `CancellationToken` 广播停止信号；
- 循环在检测到停止信号后，显式释放底层硬件句柄（DMA-BUF、CVPixelBuffer、NPU context）；
- 主线程回收工作线程必须设置超时上限（如 2500ms），若底层硬件 FFI 挂死超时，记录 `error!` 并放弃阻塞 `thread.join()`，执行线程隔离。

---

## 6. 禁止事项 (Iron Rules)

- ❌ 在 async 任务中直接调用阻塞的硬件 FFI 或解码/推理接口
- ❌ 在帧流转通道中使用 `unbounded_channel`
- ❌ 在常驻推理路径中使用 `spawn_blocking`（导致模型反复初始化与上下文丢失）
- ❌ 运行时动态频繁创建 `std::thread::spawn`
- ❌ 持锁跨越 `.await` 挂起断点
- ❌ 在解码线程中使用阻塞式 `send`（导致被下游反压反向卡死解码器）
- ❌ 停机时无超时死等 `thread.join()`
