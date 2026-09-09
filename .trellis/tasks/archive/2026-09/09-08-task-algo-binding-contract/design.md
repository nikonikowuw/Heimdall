# 技术设计：任务算法绑定数据契约与实例一致性

- 任务：`.trellis/tasks/09-08-task-algo-binding-contract`
- 状态：design
- 范围：后端核心数据契约（`crates/types`、`crates/db`、`crates/api`）

---

## 1. 架构总览与目录边界

本任务属于纯 Rust 后端数据契约与持久化层改造，为后续任务启动调度、状态同步及前端回显提供唯一事实源。

```
Heimdall/
├── crates/types/
│   └── src/task.rs                  # TaskStatus 显式 repr(i32) 与辅助方法，AnalysisTask 增补算法字段
├── crates/db/
│   ├── src/migration/migrations/
│   │   └── V5__bind_algorithm_to_analysis_tasks.sql # analysis_tasks 表增补字段与索引
│   ├── src/entity/task.rs           # SeaORM Entity 结构同步
│   ├── src/repository/task.rs       # TaskRepo 增补校验、原子同步算法实例与状态同步接口
│   └── tests/task_repo_tests.rs     # 事务原子性、输入校验与实例同步集成测试
└── crates/api/
    └── src/routes/task.rs           # TaskConfigDto / TaskSummaryDto 增补算法字段、DTO 校验与调用重构
```

### 目录与职责划分
| 模块/目录 | 职责范围 | 不允许的操作 |
|---|---|---|
| `crates/types` | 核心领域类型、`TaskStatus` 枚举定义与显式数值映射 | 不引入 db/api 依赖，不包含 I/O 操作 |
| `crates/db` | SQLite 迁移 (V5)、Entity、事务级 Task ↔ AlgorithmInstance 原子同步仓储 | 不直接处理 HTTP 错误，不调用硬件或解码器 |
| `crates/api` | DTO 定义、参数校验（FPS、JSON Object）、Handler 映射 | 不直接编写 SQL/SeaORM DSL，调用 TaskRepo 完成原子操作 |

---

## 2. 数据流与契约设计

### 2.1 `TaskStatus` 显式数值映射与状态机规范

固定 `TaskStatus` 为 `#[repr(i32)]`，禁止依赖隐式 cast：

| 枚举变体 | `i32` 数值 | 字符串 (JSON) | 语义说明 |
|---|---|---|---|
| `Stopped` | `0` | `"stopped"` | 任务未启用或已停止，资源已释放 |
| `Starting` | `1` | `"starting"` | 正在拉取流、加载模型或创建解码器/Worker |
| `Running` | `2` | `"running"` | 分析泵正常驱动，常驻推理进行中 |
| `Degraded` | `3` | `"degraded"` | 降级运行（如模型热替换失败回退、帧率下调） |
| `Reconnecting` | `4` | `"reconnecting"` | 码流中断或重连恢复中 |
| `Error` | `5` | `"error"` | 启动失败或运行异常，保留期望状态供自愈排查 |

提供转换函数：
- `TaskStatus::as_i32(self) -> i32`
- `TaskStatus::from_i32(val: i32) -> Option<Self>`
- `impl TryFrom<i32> for TaskStatus`
- `impl From<TaskStatus> for i32`
- `impl std::fmt::Display for TaskStatus`

### 2.2 数据库 Schema 迁移 (`V5__bind_algorithm_to_analysis_tasks.sql`)

在 `analysis_tasks` 表中增补 4 个字段：
```sql
ALTER TABLE analysis_tasks ADD COLUMN algorithm_id TEXT NOT NULL DEFAULT '';
ALTER TABLE analysis_tasks ADD COLUMN analysis_fps INTEGER NOT NULL DEFAULT 0;
ALTER TABLE analysis_tasks ADD COLUMN algo_params_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE analysis_tasks ADD COLUMN status_message TEXT NOT NULL DEFAULT '';

CREATE INDEX IF NOT EXISTS idx_analysis_tasks_algorithm_id ON analysis_tasks(algorithm_id);
```

- 兼容性：`DEFAULT` 约束确保旧数据迁移后字段自动填充，无破坏性变更。
- `analysis_fps = 0` 代表不限帧率（由上游子码流物理帧率驱动）。

### 2.3 `Task` 与 `AlgorithmInstance` 单一事实源与原子同步

本期系统规范：“一个摄像头任务对应一个主算法实例”。
在 `TaskRepo` 中提供单一入口原子同步函数：
```rust
pub async fn save_task_and_sync_instance(
    db: &DatabaseConnection,
    params: SaveTaskParams,
) -> Result<Model, DbError>;
```

**执行流程（单一事务 `db.transaction` 内）**：
1. **输入合法性校验**：
   - `params.analysis_fps >= 0`，否则返回 `DbError::Validation`。
   - `params.algo_params_json` 解析必须为 `serde_json::Value::Object`，否则返回 `DbError::Validation`。
   - 若 `params.algorithm_id` 非空，查询 `algorithms` 表验证算法是否存在。若不存在，返回 `DbError::NotFound`（杜绝静默写入无效算法）。
2. **Upsert `analysis_tasks`**：
   - 存在则更新 `name`、`desired_enabled`、`algorithm_id`、`analysis_fps`、`algo_params_json`、`rules_json`、`motion_gate_json`、`updated_at`；
   - 不存在则插入，初始 `actual_status = 0 (Stopped)`、`status_message = ""`。
3. **同步关联 `algorithm_instances`**：
   - 查询当前 `camera_id` 下已有的算法实例：
     - 若已存在实例，更新其 `algorithm_id`、`analysis_fps`、`params_json`、`rules_json`、`motion_gate_json`、`enabled = params.desired_enabled`、`updated_at`；
     - 若不存在且 `algorithm_id` 非空，插入新实例（生成 UUIDv4 `instance_id`），`enabled = params.desired_enabled`，`actual_status = 0`，`status_message = ""`。
4. **事务提交保证**：
   - 任何一步出错（如无效算法、JSON 解析失败、DB 约束违反），整个事务回滚，杜绝留下孤儿记录或半更新状态。

配套同步方法：
- `update_status(db, camera_id, actual_status, status_message)`：事务内同时更新 `analysis_tasks` 与对应 `algorithm_instances` 的 `actual_status` 及 `status_message`。
- `delete_task_and_instance(db, camera_id)`：事务内级联删除 `analysis_tasks` 与 `algorithm_instances` 中该 `camera_id` 的记录。

### 2.4 API DTO 契约规范

#### `TaskConfigDto` (camelCase)
```json
{
  "cameraId": "CAM-01",
  "name": "西门入口布防",
  "desiredEnabled": true,
  "algorithmId": "general_detection",
  "analysisFps": 15,
  "algoParams": { "threshold": 0.5 },
  "actualStatus": 2,
  "statusMessage": "Running",
  "rules": [],
  "motionGate": { "enabled": true, "threshold": 25, "contourArea": 100, "keepaliveIntervalMs": 2000 }
}
```

#### `TaskSummaryDto` (camelCase)
```json
{
  "id": 1,
  "cameraId": "CAM-01",
  "name": "西门入口布防",
  "desiredEnabled": true,
  "actualStatus": 2,
  "statusMessage": "Running",
  "algorithmId": "general_detection",
  "analysisFps": 15,
  "algoParams": { "threshold": 0.5 },
  "rulesCount": 0,
  "motionGateEnabled": true,
  "rules": [],
  "motionGate": { "enabled": true, "threshold": 25, "contourArea": 100, "keepaliveIntervalMs": 2000 },
  "createdAt": 1741500000000,
  "updatedAt": 1741500000000
}
```

---

## 3. 错误处理与防御边界

1. **`DbError::Validation`**：
   - 增加专用的验证错误分支，返回明确的不合规原因（如 `"analysis_fps 必须大于等于 0"`, `"algo_params 必须为 JSON Object"`）。
2. **API 错误映射**：
   - 在 `crates/api/src/routes/task.rs` 中，捕获输入错误并返回 HTTP 400 Bad Request，错误码 `10001` (INVALID_PARAM)。
3. **未指定 `algorithm_id` 的边界**：
   - 允许 `algorithm_id` 为空字符串或 `None`（前端未选定算法）。
   - 空 `algorithm_id` 时直接落库任务，保留未解析状态；此时不创建或同步算法实例，留待运行时调度器进行平台算法回退。

---

## 4. 验证策略

1. **数据库迁移测试**：
   - 测试包含已有数据的 SQLite 库执行 V5 迁移，断言旧记录默认值正确读取。
2. **仓储层单元测试 (`tests/task_repo_tests.rs`)**：
   - 正常保存任务 + 自动同步实例验证。
   - 重复保存任务（更新参数/规则）同步更新已有实例验证。
   - 尝试绑定不存在的算法 ID，断言事务回滚且无脏数据残留。
   - 状态更新接口断言任务与实例双侧状态同步。
   - 删除任务断言级联清理实例。
3. **API 序列化与校验测试**：
   - 校验 DTO 的 camelCase 序列化与反序列化。
   - 验证负数帧率、非法 JSON 参数被 Handler 拦截拒绝。
