# 并发模型

硬件 SDK/FFI 及超过约 1ms 的 CPU 工作不直接运行在 Tokio Worker 中。

## 执行归属

| Tokio 异步任务                                       | 固定专用 OS Worker                       |
| ---------------------------------------------------- | ---------------------------------------- |
| HTTP/WS、Retina 网络 IO、轻量定时器、SeaORM 异步调用 | 解码、RGA/VPC 预处理、NPU 推理、重型计算 |

线程数量在启动装配时确定，不按帧创建线程；模型、会话及硬件上下文在所属 Worker 常驻。
常驻推理不使用逐次 `spawn_blocking`；对外 async 方法通过有界通道调度同步硬件工作。
实现参考 [InferenceWorker](../../../crates/infer/src/worker.rs) 和 [解码器](../../../crates/media/src/decoder.rs)。

## 通道

| 用途           | 选型与满载行为                                                                  |
| -------------- | ------------------------------------------------------------------------------- |
| 帧             | `crossbeam_channel::bounded`，通常 1～4；非阻塞投递、优先丢旧并计数             |
| 异步/同步控制  | 有界 `tokio::sync::mpsc`；阻塞接收只在 OS Worker，不能阻塞 Tokio 或硬件帧生产者 |
| 多客户端广播   | 有界 `tokio::sync::broadcast`；`Lagged` 跳过旧消息                              |
| 只读配置热替换 | `Arc<ArcSwap<Config>>`，按已有实现选用                                          |

- 帧路径禁止无界队列和阻塞 `send`。丢旧后重试 `try_send`，重试仍满/断开时也必须释放所有权。
- 压缩包队列丢失参考帧后，丢弃同 GOP 残缺 P/B 帧，等新 IDR 后恢复；解码帧队列不套用 GOP 规则。
- 所有事件缓冲、批次和缓存同样需容量上限，策略与指标写进配置或接口说明。

## 共享状态

- 锁内仅做短时内存操作，不跨 IO、FFI 或 `.await`；异步上下文与阻塞线程按需选 Tokio/parking_lot 锁。
- 简单计数器用 `AtomicU64`；帧所有权跨线程转移，不复制像素。
- 同一硬件实例不并发调用；插件状态与 Pipeline 全局状态隔离，见 [算法 SDK](./algo-sdk-guidelines.md#状态与生命周期)。

## 停机

1. 通过 `watch` 或 `CancellationToken` 广播停止，唤醒阻塞队列/池等待者。
2. Worker 停止接单，完成必要 flush，再释放帧、池租约和硬件上下文。
3. 按明确超时等待完成后再 `join`；解码默认 500ms，见 `DEFAULT_THREAD_SHUTDOWN_TIMEOUT`。
4. 硬件挂死超时记录错误并隔离，禁止无期限 `join`；仍在使用的句柄不能由外壳提前释放。

验证队列满/断开、GOP 恢复、取消唤醒、错误释放、停机超时以及重复启停。
