# 子任务 PRD：算法实例定向热替换与 Worker 所有权

## 1. 目标

修复现有算法激活流程的全局误替换和 Worker 句柄失效问题，使算法版本激活只影响绑定该算法的任务，并保证旧推理线程和新推理线程有明确、可等待的所有权关系。

## 2. 当前问题

- `reload_algorithm_on_pumps` 会把同一个 worker handle 广播给所有摄像头 pump。
- 算法路由只把 `InferenceWorker::handle()` 传入 pump，临时 `InferenceWorker` 离开路由后会被 Drop，pump 内的句柄可能指向已关闭线程。
- 当前 `replace_worker` 只能替换 handle，无法回收被替换的旧 worker。

## 3. 功能要求

- 算法激活后按 `algorithm_id` 查询受影响的任务/实例，只对绑定该算法的 runtime entry 执行替换。
- runtime 或 pump 必须持有 Worker owner，而不是只持有裸 handle；替换操作必须同时转移新 owner 并停止旧 owner。
- 旧 worker 的停止不能在持有 Tokio 锁时执行阻塞等待；需要明确专用线程、`spawn_blocking` 或等价的回收策略。
- 热替换失败时保留旧 worker 和旧模型继续运行，并把状态写为 Degraded 或 Error；不能先丢旧 worker 再发现新 worker 不可用。
- 替换过程中保持现有 StreamHub session、decoder、RingBuffer 和 pump 数据流，不影响不相关摄像头。
- 如果底层限制无法支持无损替换，提供受控的停泵、重建 worker、恢复泵回退路径，并明确状态转换。

## 4. 验收标准

1. 两个摄像头绑定不同算法时，激活其中一个算法版本只替换目标摄像头。
2. 热替换后新 worker 仍存活，推理指标继续增长；旧 worker 在有界时间内退出且无线程泄漏。
3. 新模型加载或实例化失败时，旧 worker 仍可继续推理，任务状态和日志可诊断。
4. 热替换期间不新增主/子流订阅，不丢失运行时资源所有权。
5. 覆盖 worker owner 生命周期、定向替换、失败回滚和并发激活测试。

## 5. 依赖

- 依赖 `task-algo-binding-contract` 和 `task-runtime-lifecycle-coordinator`。
- 可与 `task-cold-start-task-recovery` 并行开发，但必须共享同一套算法选择和 runtime entry 契约。
