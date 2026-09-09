# 实现计划：任务算法绑定数据契约与实例一致性

## API 契约跳过说明

本任务属于纯后端数据模型与仓储改造（`crates/types`、`crates/db`、`crates/api`），DTO 已在 `design.md` 中完整定义并在后续子任务 `09-08-task-frontend-task-algorithm-status` 中与前端对接，因此跳过独立的 `api.md`。

---

## 前置条件

- 依赖分支：dev / 当前工作区干净
- 编译环境：Rust 1.75+ / cargo test 全绿通过

---

## 实现步骤

### Step 1: `crates/types` 类型扩展与状态映射
- [ ] 检查并修改 `crates/types/src/task.rs`：
  - 为 `TaskStatus` 声明 `#[repr(i32)]`，显式指定 `Stopped = 0`、`Starting = 1`、`Running = 2`、`Degraded = 3`、`Reconnecting = 4`、`Error = 5`。
  - 实现 `TaskStatus::as_i32()`、`TaskStatus::from_i32()`、`TryFrom<i32>`、`From<TaskStatus>` 和 `Display`。
  - 为 `AnalysisTask` 增补 `algorithm_id: String`、`analysis_fps: u32`、`algo_params: serde_json::Value`。
- [ ] 修复因 `AnalysisTask` 结构变更引发的现有单测（如 `crates/pipeline/src/manager.rs`）。
- [ ] 验证：`cargo test -p types -p pipeline`

### Step 2: 数据库迁移 V5 (`crates/db`)
- [ ] 新建 `crates/db/src/migration/migrations/V5__bind_algorithm_to_analysis_tasks.sql`：
  - `ALTER TABLE analysis_tasks ADD COLUMN algorithm_id TEXT NOT NULL DEFAULT '';`
  - `ALTER TABLE analysis_tasks ADD COLUMN analysis_fps INTEGER NOT NULL DEFAULT 0;`
  - `ALTER TABLE analysis_tasks ADD COLUMN algo_params_json TEXT NOT NULL DEFAULT '{}';`
  - `ALTER TABLE analysis_tasks ADD COLUMN status_message TEXT NOT NULL DEFAULT '';`
  - `CREATE INDEX IF NOT EXISTS idx_analysis_tasks_algorithm_id ON analysis_tasks(algorithm_id);`
- [ ] 更新 `crates/db/src/entity/task.rs` 中的 `Model` 结构体字段。
- [ ] 验证：`cargo test -p db --lib`（测试数据库迁移与 schema 初始化）

### Step 3: `DbError` 校验扩展与 `TaskRepo` 增强
- [ ] 在 `crates/db/src/error.rs` 中增加 `Validation(String)` 错误分支。
- [ ] 在 `crates/db/src/repository/task.rs` 中：
  - 定义 `SaveTaskParams` 结构。
  - 实现校验逻辑：`analysis_fps >= 0`、`algo_params_json` 必须是有效 JSON Object。
  - 实现核心事务方法 `save_task_and_sync_instance`：
    - 若 `algorithm_id` 非空，验证该算法在 `algorithms` 表是否存在。
    - Upsert `analysis_tasks`。
    - 原子同步对应 `camera_id` 的主 `algorithm_instances` 记录。
  - 实现 `update_status` 事务同步方法（同时更新任务与算法实例状态及消息）。
  - 实现 `delete_task_and_instance` 事务级联清理方法。
  - 保留并更新原有兼容接口 `save_or_update`、`find_by_camera_id`、`list_all` 等。

### Step 4: 数据库仓储单元测试与集成测试
- [ ] 新建 `crates/db/tests/task_repo_tests.rs`：
  - 测试旧数据兼容与 V5 迁移后读取。
  - 测试 `save_task_and_sync_instance` 成功创建任务并生成算法实例。
  - 测试再次调用更新任务参数并同步更新算法实例。
  - 测试非法 `analysis_fps`（负数）与非法 `algo_params_json`（非对象）被拒绝。
  - 测试绑定不存在的算法 ID 时事务回滚且无脏数据。
  - 测试 `update_status` 双侧同步。
  - 测试 `delete_task_and_instance` 双侧清理。
- [ ] 验证：`cargo test -p db --test task_repo_tests`

### Step 5: `crates/api` DTO 与路由适配
- [ ] 修改 `crates/api/src/routes/task.rs`：
  - `TaskConfigDto` 增补 `algorithm_id`、`analysis_fps`、`algo_params`、`actual_status`、`status_message`。
  - `TaskSummaryDto` 增补算法与状态字段。
  - `update_task` handler 接入校验：
    - 检查 `analysis_fps >= 0`
    - 检查 `algo_params.is_object()`
    - 调用 `TaskRepo::save_task_and_sync_instance` 进行原子保存
  - `delete_task` handler 调用 `TaskRepo::delete_task_and_instance`。
  - `get_task` 和 `list_tasks` 准确回显算法与状态字段。
- [ ] 编写或更新 API 单元测试，验证 DTO 序列化字段为 camelCase。
- [ ] 验证：`cargo test -p api`

### Step 6: 门禁检查与回归验证
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo test --workspace`

---

## 回滚策略

本任务涉及数据库迁移与仓储层重构，若发生不可恢复的问题：
1. `git checkout -- crates/types crates/db crates/api`
2. 删除 `crates/db/src/migration/migrations/V5__bind_algorithm_to_analysis_tasks.sql`
3. 重新运行 `cargo test --workspace`
