# 子任务 PRD：任务算法绑定数据契约与实例一致性

## 1. 目标

补齐 Task、Rust 类型、数据库实体、任务 API 与 AlgorithmInstance 之间的配置和运行状态契约，为后续运行时启动提供唯一、可恢复的任务配置来源。

本子任务只负责数据模型、迁移、仓储和输入校验，不负责拉流、解码器、推理 worker 或冷启动调度。

## 2. 范围

- `analysis_tasks` 增加 `algorithm_id`、`analysis_fps`、`algo_params_json`，必要时增加 `status_message`。
- `crates/types` 增加任务算法配置字段，并固定 `TaskStatus` 的持久化数值映射。
- `TaskConfigDto`、`TaskSummaryDto` 和数据库 entity/repository 支持新字段。
- 以“一个摄像头任务对应一个主算法实例”作为本期约束；多算法并发另开任务。
- 任务保存时提供 Task 与 AlgorithmInstance 的原子同步仓储接口，避免前端双写产生不一致。

## 3. 关键约束

- `algorithm_id` 未提供时保留未解析状态，由运行时层执行 `general_detection` 或平台检测算法回退；数据层不得静默写入不存在的算法。
- `analysis_fps` 必须校验为非负值；`0` 的含义统一为不限帧率，默认值由 API 明确规定。
- `algo_params` 必须是 JSON object 或空对象，不接受数组、字符串等不可作为算法配置的值。
- `TaskStatus` 使用显式 `#[repr(i32)]` 或等价转换函数，禁止业务代码继续依赖隐式 enum cast。
- 兼容现有数据库：旧任务读取时使用默认算法参数和状态消息，迁移不得破坏已有任务。

## 4. 验收标准

1. 新数据库迁移后，旧数据和新任务均可正常读写算法字段。
2. Task DTO 使用 `camelCase` 返回 `algorithmId`、`analysisFps`、`algoParams`、`actualStatus` 和 `statusMessage`。
3. Task 与主 AlgorithmInstance 的创建、更新、启停状态同步具备单元测试；失败时不能留下半更新记录。
4. 状态码与 `TaskStatus`、`AlgorithmInstance.actual_status`、前端展示约定一致，并覆盖 Stopped、Starting、Running、Degraded、Reconnecting、Error。
5. `cargo test -p db`、相关 API 类型测试和 `cargo clippy --all-targets -- -D warnings` 通过。

## 5. 依赖与不包含内容

- 被 `task-runtime-lifecycle-coordinator`、`task-api-pipeline-orchestration`、`task-cold-start-task-recovery` 和 `task-frontend-task-algorithm-status` 依赖。
- 不实现 StreamHub 订阅、pump 启停、算法实例创建、热替换和前端交互。
