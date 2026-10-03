# 告警查询链路物理层修复：统计信息、索引、count_since 时间轴与 severity 字段下线

## Goal

修复实时告警中心（`/alarms` 列表 + 计数、`/system/overview` 今日告警、存储淘汰扫描）查询链路的物理层欠账，使查询耗时与执行计划不再依赖"是否恰好 ANALYZE 过"这一运行时偶然状态；同时下线一个**从未被使用过**的字段 `severity`（严重等级），消除"用户看到恒定假等级、筛选项恒返回空集"的语义误导。

用户价值：
1. 告警页筛选/翻页响应时间从数百毫秒降至毫秒级，且不随数据量增长而突降；
2. 仪表盘"今日告警"与列表筛选在同一时间基准上，不再存在跨天分叉的可能；
3. 界面上不再出现一个永远显示 `warning`、永远筛不出 `critical` 的"严重等级"。

## Background

### 查询链路现状（已通过代码、现网 DB 与合成压力库核实）

- `alarm_records` **只有 1 个业务索引**：`idx_alarm_records_camera_time(camera_id, occurred_at DESC)`。首列是 `camera_id`，而告警页**默认态恰恰不约束 `camera_id`**（"全部通道 + 今天"，`web/src/features/alarms/AlarmsPage.tsx:176` 的 `getInitialTodayRange`）。
- 现网 DB（`Heimdall.db`）**不存在 `sqlite_stat1`** → 从未执行过 ANALYZE；全仓 `grep -rn "ANALYZE\|PRAGMA optimize" crates/` **零命中**。现状 `EXPLAIN QUERY PLAN` 为 `SCAN + USE TEMP B-TREE FOR ORDER BY`。
- bundled SQLite 为 **3.46.0**（`libsqlite3-sys 0.30.1`，全仓仅 1 个版本，rusqlite 与 sqlx 共用），已支持 `PRAGMA optimize` 的 bitmask 形式。
- 200 万行合成压力库对照（唯一变量为统计信息）：

  | 查询 | 无 stats（= 现网现状） | ANALYZE 后 |
  | --- | --- | --- |
  | 默认今日列表 | 209.61 ms（`SCAN`） | 1.96 ms（`SEARCH`） |
  | `count(occurred_at>=今日)` | 167.42 ms | 0.60 ms（`COVERING INDEX`） |
  | `count(status+今日)` | 228.90 ms | 8.11 ms |

- `idx_alarm_records_camera_time` 首列未被约束时无法走跳扫：跳扫有**硬性前置条件**——优化器要求该索引 `hasStat1`（`sqlite3.c` 的 `hasStat1!=0` 断言），**没有任何统计信息时跳扫根本不会被考虑**（不是「代价估算后判负」）→ 选全表扫。**同一二进制在不同通道数的现场表现不同**（通道越少越易退化）。
- `AlarmRepo::count_since`（`crates/db/src/repository/alarm.rs:291`）用 **`created_at`**，而「证据三支柱」的另一支 `CaptureRepo::count_since`（`crates/db/src/repository/capture.rs:225`）用 **`captured_at`**（事件时间）→ 告警侧是**偏离既有约定的离群项**，非刻意设计。
- `.trellis/spec/guides/conventions.md:10` 已确立约定：「时间戳对应源帧，不替换为回调到达时间」——支持事件时间轴。
- `created_at` 无任何索引；`/system/overview` 由前端 **每 2s 轮询**（`web/src/features/system/SystemOverview.tsx:153`），叠加 alarm + capture 两次全表扫。实测 `count_since(created_at)`：10 万行 10.84 ms / 50 万行 54.54 ms。
- `status` 在 UI 上与 `recognition_status` **完全对称**（`web/src/features/alarms/filters.ts` 中两张取值表并列声明），但只有 `recognition_records` 有 `idx_recognition_records_status`，`alarm_records` 无。
- `idx_capture_records_camera_crop ON capture_records(camera_id, crop_image_rel_path)` 是**死索引**：全仓无任何按 `crop_image_rel_path` 过滤的查询（`CropImageRelPath` 仅出现在 `select_only` 列投影中）。

### `severity` 字段：证据表明从未被使用

用户已确认「从未使用过，直接删除」。核实证据：

| 检查维度 | 结果 |
| --- | --- |
| 生产写入点 | **仅 1 处**：`crates/api/src/alarm_service.rs:104` 硬编码 `AlarmSeverity::Warning.as_str()` |
| `SET/UPDATE severity` SQL | 同上，无其他（含手写 SQL） |
| API 端点能否写入 | **不能**——`severity` 仅作为 query 参数出现在读取路径（`crates/api/src/routes/alarm.rs:72,89`） |
| 规则配置 `rules_json` 能否携带等级 | **不能**——`DetectionRule` 字段只有 `role` / `line_direction` / `points`（`crates/types/src/task.rs:48-52`） |
| 上游事件载荷 | **不含**——`crates/pipeline/src/` 中 `severity` 零命中 |
| 算法包输出契约 | **不包含**——C ABI `AV_RESULT_ALARM` 无等级字段；JSON 解析器只认 `bbox`/`confidence`/`quality_score`/`embedding`/`fused_count`/`template_quality` |
| git 全历史 `-S'"critical"'`（生产代码） | **空**——从未在生产代码中出现过 |
| git 全历史所有 `severity: Set(...)` diff | 生产代码只有 `Warning`；`"critical"` / `"high"` 仅出现在测试文件 |
| 唯一写入点的演变 | 自引入提交 `c4b67031` 起**一次未改** |
| 原始设计文档 | **不含该列**——`prd/prd-v1.0.md:361-379` 的 `alarm_records` 建表语句无 `severity`，确认它是 V2 迁移临时追加，非设计意图 |

因此该列的值来源是 **V2 迁移的 SQL `DEFAULT 'warning'`**：即使删掉第 ④ 步那一行，SQLite 也会补出同样的值（已实测验证两者的记录取值完全相同）。前端渲染的是**裸英文常量**（`AlarmTableRow.tsx:115` 的 `{alarm.severity || 'WARNING'}`），`"WARNING"` 在任何 i18n 文件中都不存在；`severity` 的 i18n key 只用在筛选下拉标签上。

### 绑定值格式的潜在隐患（R3.2 的依据）

SQLx 对 `NaiveDateTime` 与 `DateTime<Tz>` 采用**不同**的 SQLite 文本编码（见 `sqlx-sqlite-0.8.6/src/types/chrono.rs:73-77`）：

| 绑定类型 | 编码格式 | 示例 |
| --- | --- | --- |
| `NaiveDateTime` | `%F %T%.f`（空格分隔） | `2026-10-02 09:30:00` |
| `DateTime<Utc>` | RFC3339（`T` + 偏移） | `2026-10-02T09:30:00+00:00` |

而 `occurred_at` 由应用层 `Set(Utc::now())` 写入，落盘为 **RFC3339**。字符串比较下 `' ' < 'T'`，因此混合格式做范围比较会静默丢行。已用内存库实测（`occurred_at` 为 RFC3339，三条记录分属今日 05:00 / 今日 11:00 / 昨日 23:50）：

| 边界值 | `NaiveDateTime` 空格绑定 | `DateTimeUtc` RFC3339 绑定 |
| --- | --- | --- |
| 今日零点 | 命中 [1,2] ✓ | 命中 [1,2] ✓ |
| 今日 09:30 | 命中 [1,2] ✗ **多含 05:00** | 命中 [2] ✓ |

**当前未爆**：`overview.rs` 恰好只用「今日零点」这一种边界，两种编码在该值下结果相同。但这是一个**随时会触发**的隐患——任何未来新增的非零点下界（如「近 N 小时」「自某次部署以来」）都会静默丢行。R3.2 把类型收窄为 `DateTimeUtc` 即从根上消除该风险。

### 对上一轮口头分析的纠正（重要）

上一轮把 `reconcile_orphans` → `find_all_active_image_paths`（全表物化 200 万行 ≈ 628 ms ×3 表）描述为"**300s 周期固定成本**"是**错误的**。核实结果：

- `start_periodic_worker`（`crates/pipeline/src/storage_cleaner/mod.rs:974`）的循环体**只调用 `sweep_tombstones` 与 `clean_if_needed`**，二者均**不**调用 `reconcile_orphans`（`clean_if_needed` 内 `reconcile_orphans` 出现次数为 0）。
- 全仓 `reconcile_orphans` 调用点仅 3 处：`crates/app/src/main.rs:272`（**冷启动一次**）与 `storage_cleaner/mod.rs` 内的 2 处单测。

真实严重度为**一次性启动延迟**（≤50 万行约 150–200 ms），不构成需要本次修复的运维问题。

## Requirements

### R1 · 补齐并维护 SQLite 统计信息（P0）

- R1.1 在 `crates/db/src/connection.rs::init_db` 的 pragma 序列中增加 `PRAGMA optimize=0x10012;`。
  bitmask 语义（bundled 3.46.0）：`0x10000` = 让本连接未查询过的表也进入检查（**非无条件必需**：仅有未分析索引的表靠条件 4b 已覆盖，不可替代的是无索引表）；`0x10` = 以有界 `analysis_limit` 执行（内部取 `SQLITE_DEFAULT_OPTIMIZE_LIMIT` = 2000 行）；`0x2` = 对可能受益的表运行 ANALYZE（默认位）。
  **`0x10` 位不可省**：缺它时 `nLimit = 0`，ANALYZE 不受行数限制，冷启动会完整扫描库内每个索引（实测 50 万告警 + 20 万抓拍 + 20 万识别：`0x10002` = 302 ms，`0x10012` = 40 ms；2 M 行时分别为 2912 ms 与 1915 ms）。
  **`0x2` 与 skip-scan 无关**：该位只表示「运行 ANALYZE」；跳扫由优化器依据 `sqlite_stat1` 自行决策，没有任何 pragma bit 用于开关它。
- R1.2 `crates/db/src/migration/mod.rs::run_migrations` 的 rusqlite 连接同样补齐该 pragma，保证迁移后计划已就绪。
- R1.3 本次仅要求冷启动引导；周期化刷新见 Out of Scope。

### R2 · `alarm_records` 索引对齐（P0/P1）

新增迁移 `V23__alarm_query_indexes.sql`：

- R2.1 `CREATE INDEX IF NOT EXISTS idx_alarm_records_time ON alarm_records(occurred_at DESC, id DESC);`
  覆盖默认态（全部通道 + 时间窗）的排序流，消除 `USE TEMP B-TREE FOR ORDER BY`，并使计划不再依赖统计信息是否到位。
- R2.2 `CREATE INDEX IF NOT EXISTS idx_alarm_records_status_time ON alarm_records(status, occurred_at DESC, id DESC);`
  与 `recognition_records` 的状态筛选索引对称。
- R2.3 `DROP INDEX IF EXISTS idx_capture_records_camera_crop;` 下线死索引。

### R3 · `count_since` 时间轴对齐（P0，正确性）

- R3.1 `AlarmRepo::count_since` 的过滤列由 `Column::CreatedAt` 改为 `Column::OccurredAt`，与列表排序/筛选、`CaptureRepo::count_since` 及 `conventions.md:10` 的事件时间约定一致。
- R3.2 入参类型由 `chrono::NaiveDateTime` 收窄为 `DateTimeUtc`。**这不只是类型卫生**：`occurred_at` 以 RFC3339 存储，而 `NaiveDateTime` 被 sqlx 编码为空格分隔格式（`%F %T%.f`）。两者字符串序不同（`' ' < 'T'`），已实测确认在**非零点边界**下会静默丢行（见下方 Background 补充）。
- R3.3 调用点 `crates/api/src/routes/system/overview.rs` 的 `today_start` 相应调整为 UTC 时刻，且**零点口径必须用设备本地零点**——列表默认时间窗由前端按本地零点计算，两者不得分叉。

### R4 · `severity` 全链路下线（P1）

新增迁移 `V24__drop_alarm_severity.sql`，**独立于 V23**，仅含一条 `ALTER TABLE alarm_records DROP COLUMN severity;`。同步删除：

- R4.1 **DB 层**：`crates/db/src/entity/alarm.rs:25` 的 `pub severity: String` 字段；`AlarmFilter.severity`（`crates/db/src/repository/alarm.rs:28`）与其过滤分支（`:49-50`）。
- R4.2 **types 层**：`crates/types/src/alarm.rs` 的 `AlarmSeverity` 枚举（`:34-58`）及其 `from_str_loose`；`crates/types/src/lib.rs:15` 的 re-export；同文件内的 `test_alarm_severity_serialization_and_loose_parsing` 测试。
- R4.3 **api 层**：`AlarmDto.severity`（`crates/api/src/routes/alarm.rs:35`）与映射（`:58`）；`AlarmQuery.severity` 与 `AlarmCountQuery.severity`（`:72,89`）及其传递（`:144,172`）；`alarm_service.rs:104` 的写入与 `:136` 的 WS 广播载荷字段。
- R4.4 **web 层**：`types/index.ts` 的 `AlarmSeverity` 类型与 `AlarmRecord.severity`；`filters.ts` 的 `SEVERITY_FILTERS` / `SeverityFilter`；`AlarmsPage.tsx` 的筛选状态、请求参数、WS 匹配逻辑与 `SelectField`；`AlarmTableRow` / `AlarmCardItem` / `AlarmLightboxModal` 的等级列与 `isCritical` 分支；`AlarmsContent.tsx:152` 的表头列；`LivePage.tsx` 的 `LiveAlarmToast.severity`；`lib/api.ts` 两组 `severity` 参数；3 个语言包的 `columns.severity` / `filter.severityWarning` / `filter.severityCritical` / **`filter.allSeverities`**。
  > 评审修正：原清单漏列 `allSeverities`（筛选项被删但标签键未删，成孤儿键）。grep 只匹配标识符 `severity` 会漏掉标题化的 `severities`。
- R4.5 **测试同步**：`crates/pipeline/tests/rules_engine_tests.rs:152`、`crates/db/tests/evidence_repo_tests.rs:173`、`crates/api/tests/alarm_persistence_broadcast_tests.rs`（`:193,322,727,747,767`）的 severity 构造与断言。

### R5 · 规范回填（P1）

在 `.trellis/spec/db/backend/database-guidelines.md` 补入：

- R5.1 「复合索引首列未被约束时依赖跳扫 + 统计信息」这一物理层前提，以及统计信息必须由启动引导维护；并记录 `optimize` 的 bitmask 语义（含 `0x10` 位不可省的实测依据）。
- R5.2 `count_*` 聚合的目标时间列必须与该实体列表的时间轴一致（事件时间优先，除明确的入库审计场景）；并补「『今日』类聚合的零点口径必须与消费端一致」——时间轴对齐不等于边界对齐。
- R5.3 「无生产者的预留字段不得暴露给用户」——新增 DB 列时必须同时具备写入来源，否则不得进入 DTO / 筛选器 / 渲染层；删列时须同步清理 i18n 键（含标题化键名）。
- R5.4 「回归测试必须能区分修复前后两种实现」——只断言总数的测试会被相互抵消的行掩盖，必须断言命中行的身份集合（依据：`count_since` 的原测试对两种实现恒成立）。

## Acceptance Criteria

- [x] **AC1** 全新库在 `init_db` 后存在 `sqlite_stat1`，**且有数据时 `alarm_records` 获得统计条目**：`crates/db/src/migration/mod.rs::test_init_db_bootstraps_query_planner_statistics` 走真实链路（`run_migrations` → 写入 200 行 → `init_db`）断言两者。
  > 评审修正：存在性断言本身**会被空表骗过**——全新空库里无索引表 `sys_gb28181_config` 就能让 `sqlite_stat1` 出现，而三张证据表的统计条目为 0。因此测试额外钉死「业务表有统计条目」，并断言空库阶段该条目为 0。
- [x] **AC2** 默认态查询计划不含全表扫描与临时排序：`migration_tests.rs` 断言 `SEARCH` + 命中索引名 + 无 `TEMP B-TREE` + **无 `SCAN`**。
  > 评审修正：原断言只检查索引名与临时排序，`SCAN ... USING INDEX`（覆盖索引但全扫）同样能通过，弱于 AC2 文本。已收紧为显式要求 `SEARCH` 且排除 `SCAN`。
- [x] **AC3** 释放统计信息后计划不退化：`migration_tests.rs` 先在**无 `sqlite_stat1`** 的库上显式断言该前提成立再验证计划为 `SEARCH`；并补反向断言（事后 `ANALYZE` 仍保持 `SEARCH`）。
- [x] **AC4** `status` 筛选走索引：断言 `SEARCH idx_alarm_records_status_time` 且无 `TEMP B-TREE`/`SCAN`。
- [x] **AC5** 迁移幂等与历史登记：`crates/db/src/migration/mod.rs::test_refinery_records_all_versions_and_is_idempotent` 断言 `refinery_schema_history` 登记到最新版本、含 V23/V24，且重复执行不重复登记。
- [x] **AC6** `idx_capture_records_camera_crop` 已不存在：`migration_tests.rs` 断言 `sqlite_master` 中计数为 0。
- [x] **AC7** 时间轴对齐的行为验证：`crates/api/tests/alarm_persistence_broadcast_tests.rs::test_system_overview_today_alarms_uses_frame_time` 打真实端点 `/api/v1/system/overview`，断言 `todayAlarms` 按 `occurred_at` 统计。
  > 评审修正：原验证只在仓储层（`count_since`），未覆盖端点接线；且端点原先用 **UTC 零点**而列表用**本地零点**，UTC+8 下 00:00–08:00 两者矛盾。已改为本地零点口径并补端点级测试（已用变异测试证实能捕获回退）。
- [x] **AC8** `severity` 列物理消失且不丢行/不丢索引：`migration_tests.rs` 断言 `pragma_table_info` 无该列、行数不变、三个索引存活。
- [x] **AC9** `severity` 无残留引用：`crates/` 与 `web/src/` 的 shipped code 零命中；仅 `migration_tests.rs` 与新增的 AC10 契约测试按名引用（断言其不存在），属合理。
  > 评审修正：原 AC9 的 grep 模式漏掉了标题化键名 `allSeverities`（`severities` ≠ `severity`）。该孤儿键已从三语语言包删除。
- [x] **AC10** API 契约同步：`alarm_persistence_broadcast_tests.rs::test_alarm_api_omits_severity_and_ignores_severity_filter` 断言列表响应无 `severity` 键、`?severity=critical` 不筛选，count 接口同样忽略该参数。
- [x] **AC11** WS 载荷同步：`TOPIC_ALARM_TRIGGERED` 的 payload 不含 `severity`（原断言已移除，`alarm_service.rs` 不再写入）。
- [x] **AC12** Web 门禁全绿：`cd web && pnpm format && pnpm lint && pnpm typecheck && pnpm test && pnpm check:cycles && pnpm build`。
  实测：lint 无告警；617 tests / 78 files 全通过；模块图 277 模块 788 依赖无环；build 成功。
- [x] **AC13** Rust 门禁全绿：`cargo fmt --all -- --check`、`cargo clippy --all-targets -- -D warnings`、`cargo nextest run --workspace`。
  实测：**988 tests passed / 0 failed**，clippy 零告警。
- [x] **AC14** 量化对比已记录（结论写入任务 notes；不入库）。

## 二次/三次校验与修正（独立检查逐轮发现）

首轮修复后经两轮独立检查，每轮都查出真问题，均已修正：

**第二轮**
- **AC2/AC3 的首轮「收紧」是半假的**：`query_row` **只读 `EXPLAIN QUERY PLAN` 的第 1 行**，而临时排序是**单独的第 2 行**（`USE TEMP B-TREE FOR LAST TERM OF ORDER BY`）。因此 `!contains("TEMP B-TREE")` 曾为**死代码**——把 V23 索引改成只有 `(occurred_at DESC)`（漏写 `id` 次键）时断言仍会通过。已新增 `explain_plan` 辅助函数汇齐**全部**结果行，并用变异测试验证现在能拦下该退化。
- `design.md` / `implement.md` 仍残留旧的 `0x10002` 处方与「3.46 自动施加 analysis_limit」错误论断，已订正。
- `prd.md` / `V23` 注释 / `connection.rs` 中残留的「保守判负」框架已统一为准确描述（跳扫的硬前提是 `hasStat1`，不是代价估算）。
- **`0x10000` 位并非无条件必需**（已用 bundled 3.46 实测矩阵证实）：有索引且缺 stat1 的表单靠 `0x2` 即被 ANALYZE（条件 4b）；仅**无索引表**必须依赖它。这也解释了为何全新空库 `optimize` 后会出现 `sqlite_stat1` 但业务表统计条目为 0。

**第三轮**
- **AC7 测试 oracle 与实现算法不一致（DST 时区假失败）**：测试原先用「此刻偏移」回推本地零点，而实现是 DST 感知的；在欧洲/美洲夏令时切换日两者相差 60 分钟，会让测试**误报失败**。已把实现函数 `local_today_start` 公开（`routes::system` re-export）并让测试直接复用它，彻底消除双实现漂移。已用 `TZ=Asia/Shanghai / Europe/Berlin / America/New_York / UTC / Pacific/Auckland` 验证通过，并对 DST 三个分支（`Single`/`Ambiguous`/`None`）做了冻结时钟验证。
- **AC7 跨零点竞态**：若墙钟在构造与端点调用之间跨过本地零点，期望值不可比。已加守卫（检测到跨天则重算，并仅在未跨天时使用强断言 `assert_ne!(…, 1)`）。
- **AC11 补正向断言**：原仅「删掉旧断言」；现断言 `TOPIC_ALARM_TRIGGERED` 载荷**不含** `severity`（与 AC10 的 API 契约测试并列）。

## Out of Scope

- **`reconcile_orphans` / `find_all_active_image_paths` 的启动开销**：经纠正为一次性启动成本（≤50 万行约 150–200 ms），不在本次范围。若未来确需优化，**必须注意**：任何按时间窗收窄活跃路径集合的做法都会让仍被引用的老文件被判定为孤儿并被移入墓碑隔离区——这是数据丢失风险，不得作为性能优化顺手实施。
- **周期化 `PRAGMA optimize`**：需在 `EvictionStore` trait 上新增方法以维持 pipeline 不直接触碰 SQLite 的分层约束，属独立改进项。
- **新增 `severity` 的真实等级来源**：本次只做下线。若未来要引入告警分级，需跨算法包输出契约 → C ABI → pipeline 事件载荷 → DB 的完整设计，是独立功能任务。
- **前端 P3 疑点**：识别 Tab 下 `alarmApi.count` 被传入识别状态、以及批量状态变更广播的逐条 `send` 语义，均需运行时确认，不并入本任务。
- **`created_at` / `occurred_at` 存储格式混用**（RFC3339 vs `DEFAULT CURRENT_TIMESTAMP` 空格格式）：当前库内全为 RFC3339，尚未爆发；仅作规范备注记录，不做数据迁移。

## Risks

- **迁移总耗时**（实测 100 万行）：建 2 索引 + 删抓拍索引 ≈ 1.03 s；`DROP COLUMN`（触发整表重写 + 既有索引重建）≈ 0.93 s。合计约 2 s。迁移在服务启动前同步执行（`crates/app/src/main.rs:123`），期间服务不可用。缓解：拆成 V23/V24 两个迁移，失败可定位到具体版本；两顺序实测耗时几乎相同（1.96 s vs 1.94 s），故按语义拆分而非按性能。
- **统计信息引导的冷启动成本**（实测，真实 V1..V24 schema + bundled SQLite 3.46.0）：首次启动时库内无 `sqlite_stat1`，`optimize` 会为每张表补统计。

  | mask | 50 万告警 + 20 万抓拍 + 20 万识别 | 200 万告警 + 50 万 + 50 万 |
  | --- | --- | --- |
  | `0x10002`（缺 `0x10` 位） | 302 ms | 2912 ms |
  | **`0x10012`（已采纳）** | **40 ms** | **1915 ms** |

  采纳 `0x10012` 后首次引导为百毫秒级；二次启动（`stat1` 已存在）实测 1–2 ms。缺 `0x10` 位会让 ANALYZE 无界执行，这是本任务在评审中修正的缺陷之一（详见 R1.1）。
- **`count_since` 回归测试曾对两种实现恒成立**（评审发现）：原测试只断言总数 `== 2`，而「帧今日/入库昨日」与「帧昨日/入库今日」各一条时，两种实现精确抵消（一边漏计、一边多计）。已重写为断言命中行身份集合 + 与列表结果一致，并用变异测试验证（把 `occurred_at` 改回 `created_at` 会失败）。
- **`DROP COLUMN` 会重写整表**：实测 20 万行 0.19 s / 100 万行 1.15 s（独立测量）。若磁盘空间接近水位线，重写期需要额外临时空间（实测文件大小几乎不变，但 WAL 会增长）。
- **`DROP COLUMN` 与索引的交互已验证**：既有 3 个索引在 `DROP COLUMN` 后**全部存活**，计划仍命中 `idx_alarm_records_time`（实测）。无 view / trigger 引用 `severity`。
- **API 契约破坏性变更**：删除响应字段 `severity` 与查询参数 `severity` 属于**破坏性变更**。缓解：前端与后端在同一提交内同步修改（单二进制交付，不存在版本错配）；`severity` 非关键字段，忽略未知参数是现有行为。
- **V1 默认值与 ORM 写入格式叠加**：`created_at` 列在 V1 中为 `DEFAULT CURRENT_TIMESTAMP`（空格分隔），而应用层显式 `Set(Utc::now())` 写入 RFC3339。两者字符串序不同（`' ' < 'T'`），混合格式的范围比较会静默丢行。本次 R3 将 `count_since` 移出 `created_at` 可降低暴露面，但 `find_before`（淘汰扫描）仍依赖 `created_at`；已记录为规范备注，不在本次修复。

## Confirmed Decisions

| 决策点 | 结论 | 确认方式 |
| --- | --- | --- |
| `severity` 处置 | **全链路删除**（DB 列 → 类型 → API → WS → UI），`DROP COLUMN` 独立为 V24 | 用户确认「从未使用过，直接从字段开始就删除」+ 选择 C 方案 |
| `count_since` 时间轴 | **按帧时间（`occurred_at`）** —— 与列表排序/筛选、`CaptureRepo::count_since` 及 `conventions.md:10` 对齐 | 用户确认「按帧时间」 |
| `DROP COLUMN` 验证 | 技术可行（索引存活、行数保持、无 view/trigger 依赖）；100 万行表重写 ≈ 1.15 s | 实测 |

## Open Questions

（无 —— 规划阶段所有决策已通过仓库证据或用户确认消解）
