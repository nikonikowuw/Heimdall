# 子任务 PRD：任务 API 启停编排与状态同步

## 1. 目标

将 Task API 改造成任务配置和运行时控制的唯一入口：保存配置后调用运行时协调层，按照 `desiredEnabled` 执行启停，并把实际运行状态、错误原因和算法实例状态持久化。

HTTP handler 只负责参数提取、仓储调用、运行时服务调用和 DTO 映射，不直接创建 decoder、worker 或访问平台硬件 API。

## 2. 功能要求

- `GET/PUT /api/v1/tasks/{cameraId}` 支持算法 ID、分析帧率、算法参数和实际状态回显。
- `desiredEnabled = true` 时查询摄像头配置，调用统一 start 流程；启动成功后同步 Task/AlgorithmInstance 为 Running。
- `desiredEnabled = false` 时调用统一 stop 流程，确认资源释放后同步为 Stopped。
- 删除任务前先停止运行时，再在数据库内删除任务及其主算法实例；不存在运行时也必须具备幂等删除行为。
- 运行时启动失败时保留用户的 desired intent，实际状态写为 Error，并返回可诊断的 `statusMessage`。
- 重复 PUT 相同配置必须幂等；改变算法、FPS 或参数时必须停止旧配置或走明确的重配置流程，不能并行留下两套 worker。
- 任务保存与主 AlgorithmInstance 同步使用子任务一提供的统一仓储接口，前端不再需要第二次调用 `instanceApi` 保存配置。

## 3. 验收标准

1. API 集成测试覆盖创建、启用、停用、重复启用、更新配置和删除任务。
2. 启用请求成功后能查询到 Running 状态，且 pipeline 指标可观察；停用/删除后查询不到活动 pump。
3. 启动失败时 HTTP 响应、Task 数据和 AlgorithmInstance 数据都反映 Error，不出现 desired/actual 假激活。
4. 删除流程不会留下孤儿 AlgorithmInstance 或主流/子流订阅。
5. API 测试使用可注入的 mock runtime，不依赖真实 RTSP 摄像头和平台硬件。
6. `cargo test -p api`、相关 workspace 测试和 clippy 通过。

## 4. 依赖

- 前置依赖：`task-algo-binding-contract`、`task-runtime-lifecycle-coordinator`。
- 后续由 `task-cold-start-task-recovery` 复用同一套 start 流程完成冷启动恢复。
- 与告警落库、检测框元数据两个并行任务共享运行时事件和任务状态契约。
