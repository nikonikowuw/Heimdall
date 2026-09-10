# 执行计划与实施清单：任务 API 启停编排与状态同步 (implement.md)

## 1. 实施阶段拆解

### 阶段一：协调器服务抽象与 AppState 依赖注入 (crates/pipeline & crates/api)
- [x] 在 `crates/pipeline/src/coordinator.rs` 中抽象并导出 `TaskRuntimeService` Trait：
  - `start_camera_pipeline`
  - `stop_camera_pipeline`
  - `stop_all`
  - `get_runtime_info`
  - `list_runtime_infos`
  - `has_active_runtime`
- [x] 为 `TaskRuntimeCoordinator` 实现 `TaskRuntimeService`；
- [x] 在 `crates/pipeline/src/lib.rs` 中导出 `TaskRuntimeService`；
- [x] 扩展 `crates/api/src/state.rs`：
  - `AppState` 结构体新增 `pub task_coordinator: Arc<dyn pipeline::TaskRuntimeService>`；
  - `AppState::new_with_limit` 默认装配由底层 `pipeline`、`stream_hub` 和 `algo_registry` 构建的 `TaskRuntimeCoordinator`；
  - 新增 `AppState::with_task_coordinator(mut self, coordinator: Arc<dyn pipeline::TaskRuntimeService>) -> Self`；
- [x] 调整 `crates/app/src/main.rs` 确保装配正常，运行 `cargo check -p api -p app` 确保编译无误。

### 阶段二：Mock 运行时服务与测试设施 (crates/api/tests & test helpers)
- [x] 在 `crates/api/src/routes/task.rs` 测试模块中实现 `MockTaskRuntimeService`：
  - 线程安全地记录调用历史（调用的 `camera_id`、参数、调用频次）；
  - 支持配置指定摄像机启动成功、失败或返回特定 `CoordinatorError`；
  - 支持模拟当前活跃运行时查询与概要指标。

### 阶段三：Task API 编排逻辑重构 (crates/api/src/routes/task.rs)
- [x] 重构 `PUT /{camera_id}` (`update_task`)：
  - 入参范围校验（FPS 0..=60，algoParams 必须为合法对象，序列化长度上限防御）；
  - 默认算法兜底解析（若未显式传参，匹配激活算法）；
  - 事务保存期望配置到数据库 (`TaskRepo::save_task_and_sync_instance`)；
  - 同步更新 PipelineManager 几何规则；
  - 若 `desired_enabled == true`：
    - 查询并校验摄像头存在性（外键完整性保障，不存在返回 404）；
    - 组装 `StartCameraPipelineParams`（支持推导或回退子码流 RTSP 地址、Codec 映射）；
    - 处理幂等与配置变更；
    - 调用 `state.task_coordinator.start_camera_pipeline`；
    - 成功时更新数据库为 `Running(2)`，更新 `set_ai_active(true)`；
    - 失败时更新数据库为 `Error(5)`，记录 `status_message`，更新 `set_ai_active(false)`；
  - 若 `desired_enabled == false`：
    - 调用 `state.task_coordinator.stop_camera_pipeline`；
    - 更新 `set_ai_active(false)`；
    - 更新数据库为 `Stopped(0)`。
  - 返回完整 `TaskConfigDto`（包含准确的 `actualStatus` 和 `statusMessage`）。
- [x] 重构 `DELETE /{camera_id}` (`delete_task`)：
  - **顺序调整**：先调用 `state.task_coordinator.stop_camera_pipeline(&camera_id).await`；
  - 清理管线状态 (`stop_task`、`set_camera_rules`、`set_ai_active`)；
  - 调用 `TaskRepo::delete_task_and_instance(&state.db, &camera_id).await`；
  - 若删除行数为 0 返回 404，否则返回 200。
- [x] 优化 `GET /{camera_id}` (`get_task`) 与 `GET /` (`list_tasks`)：
  - 返回状态与 DTO 对齐。

### 阶段四：完整测试覆盖与集成验收
- [x] 编写测试用例验证核心行为：
  1. `test_task_create_and_enable_pipeline_success`：正常启用与状态流转；
  2. `test_task_enable_pipeline_failure_preserves_desired_records_error`：启动失败保留 desired 且持久化 Error 状态；
  3. `test_task_disable_stops_pipeline_and_sets_stopped`：停用流程平稳关闭；
  4. `test_task_idempotent_enable`：相同配置重复 PUT 只启动一次；改变 FPS 后验证先停旧配置再启动新配置；
  5. `test_task_disable_stops_pipeline_and_sets_stopped`：验证停止失败时返回 Error 且保留运行时，停止成功后才为 Stopped；
  6. `test_task_delete_stops_pipeline_before_db_removal`：先停流与运行时，再删除数据库记录。
- [x] 运行门禁验证并排查异常。

---

## 2. 验证命令清单 (Verification Commands)

```bash
# 1. 格式化检查
cargo fmt --all -- --check

# 2. 静态检查
cargo clippy -p pipeline -p api -p app --all-targets -- -D warnings

# 3. 目标包单元与集成测试
cargo test -p api

# 4. 全工作区完整回归测试
cargo test --workspace
```

---

## 3. 回滚与安全考量 (Rollback & Safety)

1. **防死锁与原子性**：
   - 数据库操作严格收敛于 `TaskRepo` 内部事务，API Handler 内不跨异步持有数据库锁；
   - 协调器内部已有 Camera 级串行锁保护，API Handler 仅作为薄客户端调用；
2. **防假激活与状态自洽**：
   - 任何非成功的媒体或算法加载错误都会立刻将 `actual_status` 刷回 `Error(5)`，前端界面能立即展示异常状态，绝不在数据库中留下假 Running 记录；
3. **回滚边界**：
   - 本次改动仅限 `crates/pipeline` 与 `crates/api`，不修改数据库 schema（前序任务已完成迁移）；若遇到严重回归，可通过 Git 快速回退 API 层的路由与状态绑定，底层协调器独立不受影响。
