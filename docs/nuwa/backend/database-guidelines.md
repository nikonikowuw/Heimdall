# 数据库规范

实现入口：[db](../../../crates/db/src/lib.rs)、[迁移](../../../crates/db/src/migration/mod.rs)、[Repository](../../../crates/db/src/repository/)。

## 连接与迁移

连接池由 `db` 构建，默认 1～4 个连接，关闭高频 SQL 日志。连接初始化必须保证：

| PRAGMA         | 值        | 目的           |
| -------------- | -------- | ------------ |
| `journal_mode` | `WAL`    | 降低读写互斥       |
| `synchronous`  | `NORMAL` | 减少 fsync 写放大 |
| `busy_timeout` | `5000`   | 写锁等待 5000ms  |
| `foreign_keys` | `ON`     | 保证外键与级联约束    |

- Refinery 文件：`src/migration/migrations/V{version}__{snake_case_description}.sql`。
- 版本递增，已合并迁移只增不改；修复或回滚用新的前向迁移。
- SQL 通过 `embed_migrations!` 内嵌，启动先迁移再初始化 SeaORM；失败立即退出。

## 表与查询

- 表名单数 snake_case；主键 `id`，事件保留唯一 `event_id: TEXT` 用于幂等。
- 绝对时间用 `INTEGER/i64` UTC 毫秒（见 [全局约定](../guides/conventions.md#时间)），布尔用 0/1；图片/视频存文件，库内只保存相对路径。
- Alarms、Captures、Recognitions 分开管理；告警支持待处理/已核验状态流转。
- **证据图来源可追溯**：`capture_records` / `recognition_records` 必须记录 `image_source`
  （`peak_candidate` / `targeted`）与 `image_stream`（`main` / `sub`），并附 `image_pts_ms`。
  枚举取值定义在 [types/evidence.rs](../../../crates/types/src/evidence.rs)，库内以 snake_case 字符串存储，
  空串表示「未标注」（迁移前遗留行），读取侧一律归一为 `null`，不得用默认值猜测语义。
- `image_pts_ms` **只能在与检测轴同轴时写入**；跨轴取证帧（如主码流回溯）写 0 表示不可比，
  否则会把两条不同时钟轴的时标混在一个列里。判定统一由
  [`SnapshotResult::comparable_frame_pts_ms`](../../../crates/pipeline/src/snapshot.rs) 给出，不在 API 层重算。
- **抓拍行必须与证据图成对（无图不成行）**：`capture_records` 的行本身就是证据产物（人工复核、识别裁剪、存储统计的输入），快照生成失败时跳过落库并记 WARN，**不得**写入空 `image_rel_path` 的行——不可复核的行会污染证据表并掩盖证据缺失率。告警表相反：告警事实由规则引擎独立判定，证据失败时行必须保留并标注证据状态（`evidenceStatus`，待实现，见 [契约](./detection-alarm-contract.md)）。取证失败的可观测性由日志计数承担，不靠造无图行。
- **抓拍特写列成对语义**：`capture_records.crop_image_rel_path` 是人脸特写（有脸时），
  `capture_records.body_crop_image_rel_path` 是人体特写（抓拍记录恒产出，人工复查看衣着的主体证据）。
  两列均 `NOT NULL DEFAULT ''`，空串表示「本次未产出」（背身/低头没有人脸特写）、迁移前遗留行同样为空串，
  读取侧一律归一为「不可用」，不得用近似图或默认值充填（迁移 `V18`）。
- **淘汰必须登记记录的全部物理文件**：`EvictionStore` 用
  [`EvidenceRecordFiles`](../../../crates/pipeline/src/storage_cleaner/mod.rs) 承载一条记录的整套相对路径，
  抓拍最多三图（全景 + 人脸特写 + 人体特写），由仓储层过滤空串后传入。漏登记任一图 = 留下永不回收的孤儿文件，
  破坏「图在案在，图销案销」；`find_all_active_image_paths` 的孤儿对账同样以该集合为准。
- 融合模板元数据（`fused_count` / `template_quality`）可空存储；一次性握手信号（如 `template_mature`）
  不落库：把「是否成熟」这种事件写成列，只会得到无法解释的 NULL。
- **[规划设计] 录像切片实体 (RecordSegments)**：独立于抓拍单张图管理，表名为 `record_segments`。记录 `camera_id`、`stream_type`、`start_time_ms`、`end_time_ms`、`duration_ms`、`file_path`（必须为相对路径）、`has_motion`、`has_alarm`、`alarm_ids` 及 `status`。必须建立 `(camera_id, start_time_ms, end_time_ms)` 与 `(status, has_alarm, start_time_ms)` 复合索引，满足时间轴毫秒级范围检索与高效淘汰。详见 [视频录像与回放引擎设计](../designs/video-recording-and-playback-engine.md)。
- 查询封装在 Repository，`api` / `pipeline` 不直接使用 SeaORM DSL；列表必须有 `limit`。
- 时间范围与摄像头过滤建立对应复合索引，例如 `(camera_id, timestamp)`；分页遵循 [API 契约](./api-guidelines.md#分页)。
- **排序一律以 `id` 兜底**：`occurred_at`/`captured_at`/`recognized_at`/`created_at` 均非唯一，同一毫秒的批量写入会造出大量并列行；只按时间列排时 SQLite 不保证稳定顺序，`limit`+`offset` 翻页会重复或漏行，淘汰扫描（`find_oldest_batch`）更会在同 `limit` 重查时反复拿到已删除的批次。排序必须写成 `order_by_*(时间列).order_by_*(Column::Id)`，同向追加。
- 关键字过滤（`q`）是 `LIKE '%...%'`，**无法命中索引**，其代价由 64 字符上限与保留期约束（见 [API 契约](./api-guidelines.md#证据与告警列表的-q)）。通道名称匹配需要 `LEFT JOIN cameras`，而 `cameras.name` 非唯一列也无索引：该 join 只在客户端传 `q` 时拼入，不得无条件预置，否则会给无搜索的常态列表平白增加一次全表扫描。
- 查询共用工具收敛在 [repository/query.rs](../../../crates/db/src/repository/query.rs)（`escape_like` / `keyword_pattern`）；新增仓储不得再抄一份转义逻辑。

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
- **[规划设计] 录像分级级联淘汰**：录像切片作为一级实体接入 `StorageCleaner` 的 `EvictionStore`。存储水位警戒时，严格遵循四级淘汰阶梯：无告警过期切片 $\to$ 水位超限早期无告警切片 $\to$ 过期告警关联切片。执行过程严格遵循两阶段提交（`deleting` 标记 $\to$ `.tombstone/` 原子移动 $\to$ 单事务 DB 清除 $\to$ 异步物理 Unlink），杜绝产生孤儿切片文件。详见 [视频录像与回放引擎设计](../designs/video-recording-and-playback-engine.md)。

## 验证

使用独立临时 SQLite 文件验证 WAL、迁移、查询上限、幂等与批量写入；证据清理同时断言文件和记录状态。
格式、lint 和测试门禁见 [质量规范](./quality-guidelines.md)。
