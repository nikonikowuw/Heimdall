# 子任务 PRD：冷启动任务恢复与算法选择

## 1. 目标

在算法包注册完成后，恢复数据库中持久化启用且摄像头健康的分析任务，消除重启后 `desiredEnabled = true` 但没有实际 pump 的假激活状态。

## 2. 功能要求

- 在 `reconcile_and_seed_algorithms` 完成后扫描所有任务和摄像头配置。
- 仅恢复 `desired_enabled = true` 且摄像头探活状态正常、主/子码流配置完整的任务。
- 复用统一运行时 start 流程，禁止在 `app/main.rs` 中复制一套硬件装配逻辑。
- 冷启动恢复必须有并发上限和单任务错误隔离；一台摄像头失败不能阻止其他任务恢复或 HTTP 服务启动。
- 未指定 `algorithm_id` 时执行确定的回退：优先 `general_detection`，否则选择当前平台已注册且类型为 detection 的算法；没有可用算法时写入 Error。
- 恢复过程要写入 Starting、Running、Degraded 或 Error 状态及状态消息，不能只依赖内存日志。
- 对于算法包目录缺失、版本不兼容、子码流不可用或 decoder 创建失败，保留 desired intent 并记录可重试的错误。

## 3. 验收标准

1. 测试数据库中存在多个任务时，只有符合条件的任务被恢复。
2. 恢复成功的任务 pump 运行且指标持续增长；不健康摄像头不会被错误点火。
3. 未指定算法时能按回退规则启动；无候选算法时任务状态明确为 Error。
4. 单任务失败不会阻塞其他恢复任务和 API 服务启动。
5. 恢复流程具备重复执行幂等性，不会产生重复 pump、worker 或 StreamHub 订阅。
6. 覆盖 app/reconcile 的集成测试和必要的错误降级测试。

## 4. 依赖

- 依赖 `task-algo-binding-contract` 的字段和状态定义。
- 依赖 `task-runtime-lifecycle-coordinator` 的统一运行时入口。
- 建议在 `task-api-pipeline-orchestration` 完成后接入，确保冷启动和 HTTP 启动语义一致。
