# 子任务 PRD：分析任务运行时协调与双流资源生命周期

## 1. 目标

实现按摄像头管理的分析运行时协调层，统一装配主码流 RingBuffer、子码流 StreamHub 会话、硬件 decoder、独立 InferenceWorker 和 `SubStreamAnalysisPump`，并保证启动、停止、失败回滚和重复启用的资源安全。

本子任务不负责 HTTP 路由、数据库持久化、前端页面和冷启动扫描。

## 2. 启动契约

运行时协调层接收已解析的摄像头信息和任务算法配置，至少完成：

1. 校验主/子码流 URL、codec、transport policy、analysis FPS 和算法配置。
2. 获取主码流 session，建立订阅并保存 attach task handle，将 NALU 持续写入 RingBuffer。
3. 获取子码流 session，开启 AI 保活，创建与任务绑定的 decoder。
4. 从 `AlgoRegistry` 获取算法包，创建独立 `AlgoInstance` 和常驻 `InferenceWorker`。
5. 启动 `start_analysis_pump_with_worker`，并暴露可查询的 pump 状态和指标。

## 3. 生命周期要求

- 每个摄像头只能存在一个 active runtime entry；启用操作必须幂等。
- entry 必须持有主流 attach `JoinHandle`、主流订阅 key、子流 session、worker owner 和生命周期 generation。
- 停止顺序固定为停止 pump、关闭子流 AI 保活、取消并等待主流 attach、退订主流、释放 worker/decoder。
- 启动任一步失败时按反向顺序回滚，不能遗留 pump、worker、订阅计数或后台任务。
- 主流 `subscribe` 与 `unsubscribe` 必须一一对应；不能依赖 Receiver Drop 自动退订。
- 不在 Tokio worker 中执行不可控的硬件初始化、阻塞 FFI 或长时间 worker 回收；必要时使用专用线程或 `spawn_blocking`。
- 运行时内部通道必须有界，实时路径采用丢旧帧策略，不得因推理反压网络解码。

## 4. 测试与验收

1. 使用 mock decoder、mock inference backend 和可控广播 session，验证启动后 `is_analysis_pump_running` 为 true。
2. 发布编码包后，`frames_decoded` 和 `frames_inferred` 持续增长，主流 RingBuffer 能收到数据。
3. 重复启用不会创建第二个 pump、第二个 worker 或额外的主流订阅。
4. 禁用、删除和启动失败后，pump、attach task、worker 和 StreamHub 引用均被回收。
5. 主流 attach task 可以被明确取消并等待，不产生后台孤儿任务。
6. `cargo test -p pipeline`、媒体生命周期测试和 clippy 通过。

## 5. 依赖与下游

- 依赖 `task-algo-binding-contract` 的任务算法配置和状态契约。
- 被 `task-api-pipeline-orchestration`、`task-cold-start-task-recovery` 和 `task-targeted-algorithm-hot-reload` 依赖。
- 应在本子任务中确定后续告警持久化和检测框任务共同使用的分析事件出口，但不实现它们的数据库或前端功能。
