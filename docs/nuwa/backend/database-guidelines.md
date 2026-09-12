# 数据库规范

实现入口：[db](../../../crates/db/src/lib.rs)、[迁移](../../../crates/db/src/migration/mod.rs)、[Repository](../../../crates/db/src/repository/)。

## 连接与迁移

连接池由 `db` 构建，默认 1～4 个连接，关闭高频 SQL 日志。连接初始化必须保证：

| PRAGMA         | 值       | 目的               |
| -------------- | -------- | ------------------ |
| `journal_mode` | `WAL`    | 降低读写互斥       |
| `synchronous`  | `NORMAL` | 减少 fsync 写放大  |
| `busy_timeout` | `5000`   | 写锁等待 5000ms    |
| `foreign_keys` | `ON`     | 保证外键与级联约束 |

- Refinery 文件：`src/migration/migrations/V{version}__{snake_case_description}.sql`。
- 版本递增，已合并迁移只增不改；修复或回滚用新的前向迁移。
- SQL 通过 `embed_migrations!` 内嵌，启动先迁移再初始化 SeaORM；失败立即退出。

## 表与查询

- 表名单数 snake_case；主键 `id`，事件保留唯一 `event_id: TEXT` 用于幂等。
- 绝对时间用 `INTEGER/i64` UTC 毫秒（见 [全局约定](../guides/conventions.md#时间)），布尔用 0/1；图片/视频存文件，库内只保存相对路径。
- Alarms、Captures、Recognitions 分开管理；告警支持待处理/已核验状态流转。
- 查询封装在 Repository，`api` / `pipeline` 不直接使用 SeaORM DSL；列表必须有 `limit`。
- 时间范围与摄像头过滤建立对应复合索引，例如 `(camera_id, timestamp)`；分页遵循 [API 契约](./api-guidelines.md#分页)。

### 查询模式：杜绝 N+1

遍历一批实体时，先**批量收集 ID**，再用 `WHERE id IN (...)` 一次加载关联数据，最后在内存中按外键分组。禁止在循环内逐条查询。

```rust
// 正确：批量加载后内存分组
let task_ids: Vec<i64> = tasks.iter().map(|t| t.id).collect();
let instances = InstEntity::find()
    .filter(InstColumn::TaskId.is_in(task_ids.iter().copied()))
    .all(db)
    .await?;
let map: HashMap<i64, Vec<InstModel>> = instances
    .into_iter()
    .fold(HashMap::new(), |mut m, inst| {
        m.entry(inst.task_id).or_default().push(inst);
        m
    });

// 错误：循环内逐条查询
for task in &tasks {
    let insts = InstEntity::find()
        .filter(InstColumn::TaskId.eq(task.id))  // N 次查询
        .all(db).await?;
}
```

批量写入同理：先收集变更集，在单个事务内用 `INSERT` / `UPDATE` / `DELETE ... IN (...)` 一次性提交，不在循环中逐条执行。`save_task_with_instances` 已遵循此模式。

## 任务与算法实例原子同步

- 摄像头分析任务支持挂载多个不同的算法实例（1:N 绑定架构），并通过 `task_id` 显式外键关联 `analysis_tasks`。同一任务下禁止重复挂载相同算法（`UNIQUE(task_id, algorithm_id)`）。
- 任务与算法实例集合的配置修改、状态同步与级联删除，统一收敛在 `TaskRepo::save_task_with_instances`、`add_instance_to_task`、`update_instance_and_sync_task`、`update_status` 和 `delete_task_with_instances`（以及向后兼容单算法桥接函数）内部的单一 SQLite 事务（`db.transaction`），禁止在 API 或业务层脱离事务进行散装写入，杜绝产生孤儿实例或半更新记录。
- 多算法实例状态聚合：任务的综合运行状态 (`actual_status`) 严格遵循优先级判定策略聚合（`Error` > `Reconnecting` > `Starting` > `Degraded` > `Running` > `Stopped`），精准反映所挂载算法实例的整体健康状态。
- 数据层入库强校验：`analysis_fps >= 0`、`algo_params_json` 必须为有效 JSON Object。提供 `algorithm_id` 时必须检验其在 `algorithms` 表的存在性，不存在时立即回滚并返回 `DbError::NotFound`，严禁写入无效算法。
- `TaskStatus` 采用显式 `#[repr(i32)]` 固定持久化数值：`Stopped(0)`、`Starting(1)`、`Running(2)`、`Degraded(3)`、`Reconnecting(4)`、`Error(5)`，禁止依赖隐式 enum cast。

## 写入与存储保护

- 高频写入经过有界通道攒批提交（例如 32 条或 1000ms），满载丢旧并计数告警。
- 存储保护规则见 [全局约定](../guides/conventions.md#存储保护)。
- SQLite 回滚不等于文件恢复；清理验证必须覆盖文件删除失败、DB 失败及中断后的恢复行为。
- 保留天数/容量必须有限；配置 `auto_vacuum = INCREMENTAL` 并定期 `incremental_vacuum` 回收删除页面。
- [StorageCleaner](../../../crates/pipeline/src/storage_cleaner/mod.rs) 与 Pipeline 使用同一证据目录，在应用启动时注入，不在 Handler 临时创建。

## 验证

使用独立临时 SQLite 文件验证 WAL、迁移、查询上限、幂等与批量写入；证据清理同时断言文件和记录状态。
格式、lint 和测试门禁见 [质量规范](./quality-guidelines.md)。
