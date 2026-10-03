# 数据库规范

实现入口：[db](../../../../crates/db/src/lib.rs)、[迁移](../../../../crates/db/src/migration/mod.rs)、[Repository](../../../../crates/db/src/repository)。

## 连接与迁移

连接池由 `db` 构建，默认 1～4 个连接，关闭高频 SQL 日志。连接初始化必须保证：

| PRAGMA         | 值           | 目的           |
| -------------- | ----------- | ------------ |
| `journal_mode` | `WAL`       | 降低读写互斥       |
| `synchronous`  | `NORMAL`    | 减少 fsync 写放大 |
| `busy_timeout` | `5000`      | 写锁等待 5000ms  |
| `foreign_keys` | `ON`        | 保证外键与级联约束    |
| `optimize`     | `0x10012`   | 引导并维护查询计划统计信息 |

- Refinery 文件：`src/migration/migrations/V{version}__{snake_case_description}.sql`。
- 版本递增，已合并迁移只增不改；修复或回滚用新的前向迁移。
- SQL 通过 `embed_migrations!` 内嵌，启动先迁移再初始化 SeaORM；失败立即退出。
- **`PRAGMA optimize` 必须两处各一份**：`connection.rs::init_db`（sea-orm 连接池）与 `migration/mod.rs::run_migrations`（rusqlite 独立连接）各持有自己的 pragma 序列，互不共享。迁移处**必须放在迁移执行之后**：全新库在建表前 optimize 无事可做，统计信息不会生成。
- **`optimize` 的 bitmask 必须含 `0x10`，否则冷启动无界 ANALYZE**。bundled SQLite 3.46 的语义：`0x10000` = 连本连接从未查询过的表也纳入检查；`0x2` = 对可能受益的表运行 ANALYZE（默认位）；**`0x10` = 以有界 `analysis_limit` 执行**（内部取 `SQLITE_DEFAULT_OPTIMIZE_LIMIT` = 2000 行）。缺 `0x10` 时 `nLimit = 0`，ANALYZE 不受行数限制，会完整扫描库内每个索引。实测（50 万告警 + 20 万抓拍 + 20 万识别）：`0x10002` = 302 ms，`0x10012` = 40 ms；2 M 行时分别为 2912 ms 与 1915 ms。因此一律写 **`0x10012`**。
- **`0x10000` 与 `0x2` 的精确范围**：`0x2` = 对可能受益的表运行 ANALYZE（默认位），**与 skip-scan 无关**；跳扫由优化器依据 `sqlite_stat1` 自行决策，**没有任何 pragma bit 用于开关它**。`0x10000` = 让本连接从未查询过的表也进入检查，它**不是无条件必需**：因条件 4b（表上存在缺 stat1 的索引）已能覆盖绝大多数首次引导场景；`0x10000` 真正不可替代的是**无索引表**（4b/4c 均无法成立）。实测（bundled 3.46）确认：有索引且无 stat1 的表，单靠 `0x2` 即被分析；无索引表则必须靠 `0x10000`。
- **默认 mask 包含 `0x10`，风险只在显式传值时出现**：裸 `PRAGMA optimize` 的默认 mask 为 `0xfffe`（含 `0x10`），因此「显式写 `0x10002`」看似与默认等价，实际把有界 ANALYZE 关掉了——这是最容易被误判的写法。一律写 **`0x10012`**。
- **统计信息缺失会让计划退化，而不是报错**。

## 表与查询

- 表名单数 snake_case；主键 `id`，事件保留唯一 `event_id: TEXT` 用于幂等。
- 绝对时间用 `INTEGER/i64` UTC 毫秒（见 [全局约定](../../guides/conventions.md#时间)），布尔用 0/1；图片/视频存文件，库内只保存相对路径。
- Alarms、Captures、Recognitions 分开管理；告警支持待处理/已核验状态流转。
- **证据图来源可追溯**：`capture_records` / `recognition_records` 必须记录 `image_source`
  （`peak_candidate` / `targeted`）与 `image_stream`（`main` / `sub`），并附 `image_pts_ms`。
  枚举取值定义在 [types/evidence.rs](../../../../crates/types/src/evidence.rs)，库内以 snake_case 字符串存储，
  空串表示「未标注」（迁移前遗留行），读取侧一律归一为 `null`，不得用默认值猜测语义。
- `image_pts_ms` **只能在与检测轴同轴时写入**；跨轴取证帧（如主码流回溯）写 0 表示不可比，
  否则会把两条不同时钟轴的时标混在一个列里。判定统一由
  [`SnapshotResult::comparable_frame_pts_ms`](../../../../crates/pipeline/src/snapshot.rs) 给出，不在 API 层重算。
- **抓拍行必须与证据图成对（无图不成行）**：`capture_records` 的行本身就是证据产物（人工复核、识别裁剪、存储统计的输入），快照生成失败时跳过落库并记 WARN，**不得**写入空 `image_rel_path` 的行——不可复核的行会污染证据表并掩盖证据缺失率。告警表相反：告警事实由规则引擎独立判定，证据失败时行必须保留并标注证据状态（`evidenceStatus`，待实现，见 [契约](../../pipeline/backend/detection-alarm-contract.md)）。取证失败的可观测性由日志计数承担，不靠造无图行。
- **抓拍特写列成对语义**：`capture_records.crop_image_rel_path` 是人脸特写（有脸时），
  `capture_records.body_crop_image_rel_path` 是人体特写（抓拍记录恒产出，人工复查看衣着的主体证据）。
  两列均 `NOT NULL DEFAULT ''`，空串表示「本次未产出」（背身/低头没有人脸特写）、迁移前遗留行同样为空串，
  读取侧一律归一为「不可用」，不得用近似图或默认值充填（迁移 `V18`）。
- **淘汰必须登记记录的全部物理文件**：`EvictionStore` 用
  [`EvidenceRecordFiles`](../../../../crates/pipeline/src/storage_cleaner/mod.rs) 承载一条记录的整套相对路径，
  抓拍最多三图（全景 + 人脸特写 + 人体特写），由仓储层过滤空串后传入。漏登记任一图 = 留下永不回收的孤儿文件，
  破坏「图在案在，图销案销」；`find_all_active_image_paths` 的孤儿对账同样以该集合为准。
- 融合模板元数据（`fused_count` / `template_quality`）可空存储；一次性握手信号（如 `template_mature`）
  不落库：把「是否成熟」这种事件写成列，只会得到无法解释的 NULL。
- **[规划设计] 录像切片实体 (RecordSegments)**：独立于抓拍单张图管理，表名为 `record_segments`。记录 `camera_id`、`stream_type`、`start_time_ms`、`end_time_ms`、`duration_ms`、`file_path`（必须为相对路径）、`has_motion`、`has_alarm`、`alarm_ids` 及 `status`。必须建立 `(camera_id, start_time_ms, end_time_ms)` 与 `(status, has_alarm, start_time_ms)` 复合索引，满足时间轴毫秒级范围检索与高效淘汰。
- 查询封装在 Repository，`api` / `pipeline` 不直接使用 SeaORM DSL；列表必须有 `limit`。
- 时间范围与摄像头过滤建立对应复合索引，例如 `(camera_id, timestamp)`；分页遵循 [API 契约](../../api/backend/api-guidelines.md#分页)。
- **复合索引首列未被约束时，计划与统计信息强相关**：SQLite 对这类查询只能走跳扫（skip-scan），而跳扫有**硬性前置条件**——优化器要求该索引 `hasStat1`（`sqlite3.c` 的 `hasStat1!=0` 断言）且首列基数够大，**没有任何统计信息时跳扫根本不会被考虑**（不是「代价估算后判负」）。结果是静默退化为全表扫 + `USE TEMP B-TREE FOR ORDER BY`：不报错，只表现为耗时随数据量上涨。实测（50 万行告警，`alarm_records` 仅有 `(camera_id, occurred_at DESC)`）：默认态「全部通道 + 今天」86.52 ms `SCAN`；补齐 `(occurred_at DESC, id DESC)` 后 0.05 ms `SEARCH`。**因此为「不约束首列的常态查询」单独建索引，比依赖统计信息更根本**——实测把 `sqlite_stat1` 删掉后，该计划仍保持 `SEARCH`（不再随 stats 有无而变）。
- **排序索引必须把 `id` 纳入列定义**：仅建 `(occurred_at)` 时，`ORDER BY occurred_at DESC, id DESC` 仍需 `TEMP B-TREE`；列定义必须完整复刻排序子句（见上一条「排序一律以 `id` 兜底」）。
- **删列前先查见证依赖**：`ALTER TABLE ... DROP COLUMN`（SQLite 3.35+）会**重写整表并重建既有索引**（100 万行实测约 1.15 s），需在迁移窗口内完成；既有索引会存活，但必须显式断言（见 `crates/db/tests/migration_tests.rs` 的 V24 用例）。同时确认无 view / trigger 引用该列。
- **绑定的时间类型决定比较语义**：SQLx 对 `NaiveDateTime` 编码为空格分隔（`%F %T%.f`），对 `DateTime<Tz>` 编码为 RFC3339；而时间列由 ORM `Set(Utc::now())` 写入时是 RFC3339。字符串序下 `' ' < 'T'`，混用会在**非零点边界**静默丢行（零点边界恰好同果，因此最易漏测）。时间范围参数一律用 `DateTimeUtc`。
- **回归测试必须能区分修复前后两种实现**：一个「只断言总数」的测试很容易被相互抵消的行掩盖。反例：为验证 `count_since` 改用 `occurred_at`，若只构造「帧今日/入库昨日」与「帧昨日/入库今日」各一条，断言总数 `== 2` 对**两种实现都成立**（一边漏计、一边多计，精确抵消），测试看似通过却毫无保护。必须断言**命中行的身份集合**（如返回的 `event_id` 列表），或至少让两侧的计数不相等。
- **`count_*` 聚合的时间列必须与该实体列表的时间轴一致**：否则同一页面上「统计卡片」与「列表筛选」会给出矛盾数字。证据时间轴优先于入库时间轴（见 [全局约定](../../guides/conventions.md#时间)的「时间戳对应源帧」）；`created_at` 只是内部簿记列，不应用于业务统计（如 `AlarmRepo::count_since` 用 `occurred_at` 而非 `created_at`）。
- **「今日」类聚合的零点口径必须与消费端一致**：时间轴对齐（同一列）**不等于**边界对齐。后端若用 **UTC 零点**而前端筛选用**设备/浏览器本地零点**，在 UTC+8 下每天 00:00–08:00 这 8 小时内「今日告警」卡片与列表会给出不同数字。跨层统计的日界应以设备时区（`TimeStatus.timezone` / `timezone_offset`）为准，或显式接受 UTC 口径并在两端同时使用——不得默认两者等价。
- **无生产者的预留字段不得进入 DTO / 筛选器 / 渲染层**：新增 DB 列时必须同时具备写入来源。仅有 SQL `DEFAULT` 而无任何写入路径的列会让用户看到一个恒定值（如曾经的 `severity` 列：唯一写入点是硬编码常量，前端渲染永远显示 `warning`、筛选永远返回空集）。删列比留一个无来源的列便宜。删列时**必须同步清理该列在 i18n 语言包里的键**——`grep` 只匹配标识符（`severity`）会漏掉标题化的键名（`allSeverities`），孤儿键不会被 i18n parity 测试拦住（它只校验三语之间的一致性，不校验是否被引用）。
- **排序一律以 `id` 兜底**：`occurred_at`/`captured_at`/`recognized_at`/`created_at` 均非唯一，同一毫秒的批量写入会造出大量并列行；只按时间列排时 SQLite 不保证稳定顺序，`limit`+`offset` 翻页会重复或漏行，淘汰扫描（`find_oldest_batch`）更会在同 `limit` 重查时反复拿到已删除的批次。排序必须写成 `order_by_*(时间列).order_by_*(Column::Id)`，同向追加。
- 关键字过滤（`q`）是 `LIKE '%...%'`，**无法命中索引**，其代价由 64 字符上限与保留期约束（见 [API 契约](../../api/backend/api-guidelines.md#证据与告警列表的-q)）。通道名称匹配需要 `LEFT JOIN cameras`，而 `cameras.name` 非唯一列也无索引：该 join 只在客户端传 `q` 时拼入，不得无条件预置，否则会给无搜索的常态列表平白增加一次全表扫描。
- 查询共用工具收敛在 [repository/query.rs](../../../../crates/db/src/repository/query.rs)（`escape_like` / `keyword_pattern`）；新增仓储不得再抄一份转义逻辑。人员列表的 `keyword` 同样经此转义（`100%` 不得变成前缀通配符），并同样以 `id` 作次键保证翻页稳定。

### 孤儿文件对账的调度约束

`StorageCleaner::reconcile_orphans` 会递归遍历证据根目录并与活跃路径集合求差，**不能在任意时刻调度**：

- 它只能运行在**证据生产者已停但数据库已就绪**的窗口（当前为冷启动摄像机管线启动前）。与在线写入并发运行时，刚写入磁盘、尚未完成入库的图会被判为无主文件并移入墓碑。
- 目录遍历、逐条 `is_file()` 与 `quarantine_file`（含跨目录 rename）全部是阻塞文件系统调用，必须整段包在 `tokio::task::spawn_blocking` 内，不得在 Tokio worker 上直接展开。
- 扫描结果拆分返回：孤儿部分在 blocking 任务内完成隔离并回传墓碑路径，由异步侧再交给 `UnlinkDispatcher`；缺失记录（Ghost Record）只计数与告警，不做自愈删除。
- 扫描失败必须降级为警告并让服务继续启动，不得阻断启动。

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
- 存储保护规则见 [全局约定](../../guides/conventions.md#存储保护)。
- SQLite 回滚不等于文件恢复；清理验证必须覆盖文件删除失败、DB 失败及中断后的恢复行为。
- 保留天数/容量必须有限；配置 `auto_vacuum = INCREMENTAL` 并定期 `incremental_vacuum` 回收删除页面。
- [StorageCleaner](../../../../crates/pipeline/src/storage_cleaner/mod.rs) 与 Pipeline 使用同一证据目录，在应用启动时注入，不在 Handler 临时创建。
- **[规划设计] 录像分级级联淘汰**：录像切片作为一级实体接入 `StorageCleaner` 的 `EvictionStore`。存储水位警戒时，严格遵循四级淘汰阶梯：无告警过期切片 $\to$ 水位超限早期无告警切片 $\to$ 过期告警关联切片。执行过程严格遵循两阶段提交（`deleting` 标记 $\to$ `.tombstone/` 原子移动 $\to$ 单事务 DB 清除 $\to$ 异步物理 Unlink），杜绝产生孤儿切片文件。

## 验证

使用独立临时 SQLite 文件验证 WAL、迁移、查询上限、幂等与批量写入；证据清理同时断言文件和记录状态。
格式、lint 和测试门禁见 [质量规范](../../guides/quality-guidelines.md)。
