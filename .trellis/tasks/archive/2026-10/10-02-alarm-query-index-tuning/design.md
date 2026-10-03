# 技术设计：告警查询物理层修复与 severity 下线

对应 `prd.md` 的 R1–R5。本文只记录**边界、契约、顺序与权衡**，具体执行清单见 `implement.md`。

---

## 1. 边界与依赖方向

### 1.1 改动地图

```
crates/db/          ← 主战场：connection.rs / migration / entity / repository
crates/types/       ← AlarmSeverity 类型下线
crates/api/         ← DTO / Query 参数 / WS 载荷 / overview 调用点
web/src/            ← 类型 / 筛选器 / 3 组件 / i18n
.trellis/spec/db/   ← 规范回填（R5）
```

依赖方向保持单向（`types` → `db` → `api` → `web`），无新增跨层依赖。

### 1.2 明确不碰的东西

- **`pipeline` crate**：`storage_cleaner` 的 `EvictionStore` trait 是本次**刻意不扩展**的边界。周期化 `PRAGMA optimize` 需要在该 trait 上加方法，属独立改进（见 `prd.md` Out of Scope）。因此本任务对 `pipeline` 的改动**仅限于测试文件**（`rules_engine_tests.rs:152` 的 severity 构造）。
- **`infer` / `media` crate**：完全不涉及。
- **`algo-packages/`**：算法包输出契约不含 severity，无需改动。这也是本次选择"下线"而非"接上等级来源"的核心依据——接上需要改算法包 ABI。

---

## 2. 迁移设计（核心）

### 2.1 迁移划分与理由

**为什么拆成 V23 / V24 而不是合成一个文件：**

1. **失败可定位**：索引创建失败（磁盘空间不足）与 `DROP COLUMN` 失败（表重写空间不足）是两个不同的运维场景，合并后只能看到一个"迁移失败"。
2. **回滚粒度**：若 V23 索引计划被证明有副作用，可以单独 revert 而不影响 severity 下线。
3. **语义内聚**：V23 是"查询性能"，V24 是"契约清理"，两者验收标准完全不同。

**为什么不按性能排序**（先 DROP 再建索引减少重建量）：
实测 100 万行下两种顺序总耗时几乎相同（先建索引 1.96 s vs 先 DROP 1.94 s）——因为 `DROP COLUMN` 重建表时，索引重建的成本只与**索引数量**有关，与创建先后无关。既然性能无差异，就按**语义内聚**划分。

### 2.2 V23 内容

```sql
-- V23__alarm_query_indexes.sql
-- 告警查询物理层优化：补常态排序流与状态筛选索引，下线死索引。
-- 背景：alarm_records 仅有 (camera_id, occurred_at DESC)，而页面默认态不约束 camera_id，
-- 导致计划退化为全表扫 + 临时排序（且依赖 sqlite_stat1 才可能选跳扫）。

CREATE INDEX IF NOT EXISTS idx_alarm_records_time
ON alarm_records(occurred_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_alarm_records_status_time
ON alarm_records(status, occurred_at DESC, id DESC);

-- 死索引：全仓无按 crop_image_rel_path 的过滤查询，仅纯写入放大
DROP INDEX IF EXISTS idx_capture_records_camera_crop;
```

**索引列序设计说明：**

- `idx_alarm_records_time(occurred_at DESC, id DESC)`：完整复刻 `list_filtered` 的 `ORDER BY occurred_at DESC, id DESC`（`crates/db/src/repository/alarm.rs:97-98`）。`id` 必须入索引——规范要求时间列非唯一时以 `id` 兜底，索引不覆盖 `id` 则仍需 TEMP B-TREE。
- `idx_alarm_records_status_time(status, occurred_at DESC, id DESC)`：`status` 是等值过滤放首列，后两列复刻排序。与 `idx_recognition_records_status(status)` 的设计取向不同（后者只索引 status 单列），但本设计**同时服务列表排序与计数**，故带上时间列；计数可走 COVERING INDEX（实测 8.03 ms → 0.39 ms）。

**为什么不建 `(occurred_at)` 单列索引**：`(occurred_at DESC, id DESC)` 的前缀已可服务纯时间范围，单列索引是冗余。

### 2.3 V24 内容

```sql
-- V24__drop_alarm_severity.sql
-- 下线从未被使用的 severity 字段（严重等级）。
-- 证据：唯一生产写入点为硬编码 Warning；类型/DB/API/WS/UI 全链路无第二来源。
-- 注意：SQLite DROP COLUMN 会重写整表并重建索引，此为有意接受的迁移成本。

ALTER TABLE alarm_records DROP COLUMN severity;
```

**技术约束（已实测验证）：**

| 约束 | 验证结果 |
| --- | --- |
| SQLite 版本支持 | 3.43.2（系统 sqlite3）与 3.46.0（bundled）均支持，需 3.35+ |
| 列是否真的移除 | `PRAGMA table_info` 确认 `severity` 消失 |
| 既有索引是否存活 | 全部存活（`idx_alarm_records_camera_time` + V23 的 2 个） |
| 行数是否保持 | 20 万 / 100 万行均保持 |
| 是否阻塞其他会话 | WAL 模式下执行期间持有写锁，需在迁移窗口（服务未启动）完成 |
| view / trigger 依赖 | 无（`grep "create view\|create trigger"` 零命中） |
| refinery 分号拆分器 | 安全（`ALTER TABLE` 无内嵌分号） |

### 2.4 迁移幂等性

- V23 使用 `IF NOT EXISTS` / `IF EXISTS`，可安全重跑。
- V24 的 `ALTER TABLE ... DROP COLUMN` **不幂等**——但 refinery 通过 `refinery_schema_history` 表按版本号跳过已应用的迁移，因此实际执行只发生一次。**注意**：这意味着 V24 一旦应用，**无法通过重跑迁移恢复**该列；回滚需手写 `ALTER TABLE ... ADD COLUMN severity TEXT NOT NULL DEFAULT 'warning'`（可以恢复列与默认值，但**原始数据不可恢复**——不过该列本就只有 `warning` 一个取值，故无实际损失）。

---

## 3. 统计信息引导设计

### 3.1 落点：两处 pragma 列表

`crates/db/src/connection.rs::init_db` 与 `crates/db/src/migration/mod.rs::run_migrations` 各自维护一份 pragma 序列（前者用 sea-orm，后者用 rusqlite）。两处都需要：

```rust
"PRAGMA optimize=0x10012;",
```

**为什么两处都要**：

- `run_migrations` 用 **rusqlite 独立连接**（`migration/mod.rs:14`），不复用 sea-orm 连接池，因此 `init_db` 的 pragma 不会作用于迁移连接。
- 迁移后立即使统计信息生效，可避免"迁移完成但计划仍是旧决策"的窗口。
- 启动顺序为 `run_migrations` → `init_db`（`crates/app/src/main.rs:123,126`），两处都设是幂等的。

### 3.2 bitmask 语义

```
0x10000  → 让本连接从未查询过的表也进入检查（非无条件必需：仅有未分析索引的表靠条件 4b 已覆盖；不可替代的是无索引表）
0x10     → 以有界 analysis_limit 执行 ANALYZE（内部取 SQLITE_DEFAULT_OPTIMIZE_LIMIT = 2000 行）
0x2      → 对可能受益的表运行 ANALYZE（默认位，与 skip-scan 无关）
```

组合值采用 **`0x10012`**。

> **重要修正（评审发现）**：`0x10` 位**不可省**。缺它时 SQLite 置 `nLimit = 0`，ANALYZE 不受行数限制，冷启动会完整扫描库内每个索引。
>
> 曾采用 `0x10002` 并附注释「3.46 起自动施加临时 analysis_limit」——该说法**仅对默认 mask（`0xfffe`，含 `0x10`）成立**；显式传 `0x10002` 反而把 `0x10` 位清掉。实测（真实 V1..V24 schema + bundled 3.46.0）：
>
> | mask | 50 万 + 20 万 + 20 万 | 200 万 + 50 万 + 50 万 |
> | --- | --- | --- |
> | `0x10002`（缺 `0x10`） | 302 ms | 2912 ms |
> | **`0x10012`（已采纳）** | **40 ms** | **1915 ms** |
>
> 另：`0x2` 只表示「运行 ANALYZE」，**与 skip-scan 无关**；跳扫由优化器依据 `sqlite_stat1` 自行决策（硬前置条件 `hasStat1!=0`，见 `sqlite3.c:165489`），没有任何 pragma bit 用于开关它。

### 3.3 为什么不在本任务做周期化

周期刷新需要从 `pipeline` 的 300s worker 发起，但其 `EvictionStore` trait 的设计目的是**让 pipeline 不直接接触 SQLite**（`crates/pipeline/src/storage_cleaner/mod.rs:80-190` 的 trait 定义全部返回领域类型，无 `DatabaseConnection`）。要实现必须在该 trait 上新增 `async fn optimize_stats(&self)` 方法，并同步实现 `DbEvictionStoreAdapter`——这是独立的分层改动，不应混进本任务。

**风险接受**：不做周期刷新意味着统计信息只在启动时刷新一次。对告警场景可接受——表的行数在保留期约束下趋于稳定，且 R2.1 的新索引已使**默认态查询不再依赖统计信息**（AC3 专门验证此点）。

---

## 4. `count_since` 时间轴契约

### 4.1 签名变更

```rust
// before
pub async fn count_since(
    db: &DatabaseConnection,
    since: chrono::NaiveDateTime,
) -> Result<u64, DbError>
// 过滤 Column::CreatedAt.gte(since)

// after
pub async fn count_since(
    db: &DatabaseConnection,
    since: DateTimeUtc,
) -> Result<u64, DbError>
// 过滤 Column::OccurredAt.gte(since)
```

### 4.2 调用点调整

`crates/api/src/routes/system/overview.rs:36-48` 当前构造 `NaiveDateTime`：

```rust
let today_start = chrono::Utc::now().date_naive().and_hms_opt(0, 0, 0).unwrap_or_else(...);
let today_alarms = db::AlarmRepo::count_since(db, today_start).await...;
```

改为 UTC 时刻（与 `occurred_at` 的存储类型 `DateTimeUtc` 对齐）：

```rust
let today_start = chrono::Utc::now()
    .date_naive()
    .and_hms_opt(0, 0, 0)
    .map(|naive| DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc))
    .unwrap_or_else(chrono::Utc::now);
```

### 4.2.1 为什么入参类型必须同步收窄（不只是卫生问题）

`occurred_at` 以 **RFC3339** 落盘（应用层 `Set(Utc::now())`），而 SQLx 对 `NaiveDateTime` 编码为**空格分隔**格式（`sqlx-sqlite-0.8.6/src/types/chrono.rs:73-77`）。字符串比较下 `' ' < 'T'`，混合格式的范围比较会静默丢行：

| 边界值 | `NaiveDateTime` 空格绑定 | `DateTimeUtc` RFC3339 绑定 |
| --- | --- | --- |
| 今日零点（当前唯一用法） | ✓ | ✓ |
| 今日 09:30（未来非零点下界） | ✗ **多含早于边界的记录** | ✓ |

当前 `overview.rs` 恰好只用零点所以未爆，但改 `occurred_at` 后若保留 `NaiveDateTime` 入参，这个隐患会**从潜伏变为随列走**。收窄为 `DateTimeUtc` 是 R3 的必要组成部分，不是可选优化。

### 4.3 行为变更的可见影响

| 场景 | 改动前 | 改动后 |
| --- | --- | --- |
| 实时链路（`occurred_at` ≈ `created_at`） | 计入 | 计入（不变） |
| 补录 / 时移帧：帧时间在昨日、入库在今日 | 计入今日（错） | 计入昨日（对） |
| 历史录像回溯分析 | 计入今日（错） | 计入帧时间（对） |

这是**有意的语义修正**，AC7 专门验证此场景。用户已确认选择帧时间轴：告警归属「它实际发生的那一天」，与列表筛选自洽。

### 4.4 `CaptureRepo::count_since` 的入参同步

`capture.rs:225-235` 已用 `captured_at`（帧时间，时间轴正确），但入参同样是 `NaiveDateTime`，存在与 4.2.1 相同的编码不匹配风险。**应一并收窄为 `DateTimeUtc`**，保持三支柱签名一致。这是唯一对 `capture` 侧的改动（纯类型收窄，无行为变更；`captured_at` 同样以 RFC3339 落盘）。

---

## 5. `severity` 下线清单（跨层）

按依赖顺序删除（自底向上，保证每层删除后上一层已无引用）：

| 层 | 文件:行 | 内容 |
| --- | --- | --- |
| DB | `V24__drop_alarm_severity.sql` | `ALTER TABLE alarm_records DROP COLUMN severity` |
| DB | `crates/db/src/entity/alarm.rs:25` | `pub severity: String` |
| DB | `crates/db/src/repository/alarm.rs:28` | `AlarmFilter.severity` |
| DB | `crates/db/src/repository/alarm.rs:49-50` | `if let Some(sev) = filter.severity...` 分支 |
| types | `crates/types/src/alarm.rs:34-58` | `AlarmSeverity` 枚举 + `impl` |
| types | `crates/types/src/alarm.rs:120-145` | `test_alarm_severity_serialization_and_loose_parsing` |
| types | `crates/types/src/lib.rs:15` | re-export 中的 `AlarmSeverity` |
| api | `crates/api/src/alarm_service.rs:7` | `use types::{AlarmSeverity, ...}` |
| api | `crates/api/src/alarm_service.rs:104` | `severity: Set(...)` |
| api | `crates/api/src/alarm_service.rs:136` | WS payload `"severity": saved_alarm.severity` |
| api | `crates/api/src/routes/alarm.rs:6` | `use types::{AlarmSeverity, ...}` |
| api | `crates/api/src/routes/alarm.rs:35` | `AlarmDto.severity` |
| api | `crates/api/src/routes/alarm.rs:58` | DTO 映射 |
| api | `crates/api/src/routes/alarm.rs:72,89` | 两个 Query 结构体的 `severity` |
| api | `crates/api/src/routes/alarm.rs:144,172` | 两处 `AlarmFilter` 构造 |
| web | `types/index.ts:61` | `export type AlarmSeverity` |
| web | `types/index.ts:327` | `AlarmRecord.severity` |
| web | `features/alarms/filters.ts:1,46-48` | import / `SEVERITY_FILTERS` / `SeverityFilter` |
| web | `features/alarms/AlarmsPage.tsx` | `:27` import / `:52` type / `:161` state / `:400,421,436` 请求 / `:590,611,648` WS / `:777,787` refresh / `:1092-1103` SelectField |
| web | `components/AlarmTableRow.tsx` | `:28,48,110,115` |
| web | `components/AlarmCardItem.tsx` | `:29,87,157,162` |
| web | `components/AlarmLightboxModal.tsx` | `:59,225,230,235` |
| web | `components/AlarmsContent.tsx:152` | 表头 `<th>` |
| web | `features/live/LivePage.tsx` | `:32` import / `:96` 接口 / `:435` WS 类型 / `:449` 赋值 |
| web | `lib/api.ts` | `:354,368`（list）/ `:384,396`（count） |
| i18n | `en/zh-CN/zh-TW` `alarm.json` | `columns.severity` / `filter.severityWarning` / `filter.severityCritical` |
| test | `crates/pipeline/tests/rules_engine_tests.rs:152` | `severity: Set("critical")` |
| test | `crates/db/tests/evidence_repo_tests.rs:173` | `severity: Set("high")` |
| test | `crates/api/tests/alarm_persistence_broadcast_tests.rs` | `:193` 断言 / `:322,727,747,767` 构造 |

### 5.1 删除顺序的强制约束

**必须先删 DB 列，再删 Rust 实体字段**——否则 SeaORM 查询会因 `SELECT` 引用不存在的列而运行时失败。

**但 `AlarmFilter.severity` 的删除必须先于 DTO 删除**：`AlarmFilter` 是 `db` crate 的公共类型，`api` crate 构造它时传入 `params.severity`。若先删 DTO 字段，`api` 会编译失败于"缺少字段"而非"多余字段"——两种顺序都能被编译器捕获，但自底向上（db → types → api → web）能让每一层的错误信息更聚焦。

### 5.2 前端 `defineFilters` 的连带影响

`SEVERITY_FILTERS` 用 `defineFilters<AlarmSeverity>()` 声明，其中 `AlarmSeverity` 是 `web/src/types/index.ts:61` 的联合类型。删除 `SEVERITY_FILTERS` 后，`defineFilters` 仍被 `RULE_TYPE_FILTERS` / `ALARM_STATUS_FILTERS` / `RECOGNITION_STATUS_FILTERS` 使用，**保留该辅助函数**。`AlarmSeverity` 类型若在其它位置仍有引用需一并清理（当前枚举已确认仅上述位置使用）。

### 5.3 高度易误删：`--status-danger-*` 是处理状态语义，勿连带清理

三个组件中 `--status-danger-soft` / `--status-danger` / `--status-danger-border` 出现多处，**绝大多数与 severity 无关**，而是「未处理」状态（`isProcessed === false`）的语义色。已在源码逐一核对：

| 文件:行 | 归属 | 处置 |
| --- | --- | --- |
| `AlarmTableRow.tsx:48-50` | `isCritical` 三元 | **删** |
| `AlarmTableRow.tsx:110-112` | `isCritical` 三元 | **删** |
| `AlarmTableRow.tsx:126-127` | `isProcessed` 状态徽章 | **保留** |
| `AlarmTableRow.tsx:145-146` | `isProcessed` 操作按钮 | **保留** |
| `AlarmCardItem.tsx:87-88` | `isCritical` 三元 | **删** |
| `AlarmCardItem.tsx:157-162` | `isCritical` 等级标签 | **删** |
| `AlarmCardItem.tsx:208-209` | `isProcessed` 操作按钮 | **保留** |
| `AlarmLightboxModal.tsx:59,225,230,235` | `isCritical` | **删** |
| `AlarmLightboxModal.tsx:368` | 通用按钮 hover | **保留** |
| `AlarmLightboxModal.tsx:528-529` | `isProcessed` 操作按钮 | **保留** |

**判别方法**：`isCritical` 变量的三元表达式读 `alarm.severity`；`isProcessed` 的三元表达式读 `alarm.status`。删除 `isCritical` 变量后仍需保留 `isProcessed` 的分支，因此**不能**按 `--status-danger-soft` 字符串批量替换。

`--status-danger-soft` 是主题 token，仍被处理状态广泛使用，**不得从 CSS 变量中移除**。

### 5.4 破坏性契约变更的影响面

`GET /api/v1/alarms` 的响应体删除 `severity` 字段、query 参数忽略 `severity`。由于：
- 前后端同仓、同一二进制交付（`rust-embed`），不存在版本错配；
- `toQueryString` 会省略 `undefined` 参数，前端删除后不会再发送该参数；
- 后端删除 `AlarmQuery.severity` 后，若客户端仍传 `?severity=x`，serde 的 `Deserialize` 默认忽略未知字段（结构体未加 `deny_unknown_fields`），**不报错**。

因此这是安全的同步变更，无需兼容层。AC10 验证此行为。

---

## 6. 验证策略

### 6.1 迁移层

用独立临时 SQLite 文件（遵循 `.trellis/spec/db/backend/database-guidelines.md:106` 的验证约定）：
1. 建库 → 跑 `run_migrations` → 断言 V23/V24 在 `refinery_schema_history` 中
2. 重跑 `run_migrations` → 断言幂等、无报错
3. 断言 `PRAGMA table_info` 无 `severity`、2 个新索引存在、死索引消失
4. 插入数据 → 断言行数与插入数一致

### 6.2 计划层（需真实数据量）

`EXPLAIN QUERY PLAN` 无法在空表上暴露问题，必须灌入 ≥50 万行。**关键**：验证 AC3（删除 `sqlite_stat1` 后计划仍为 `SEARCH`）——这是 R2.1 价值的直接证据，也是本设计相对"只加 pragma"的增量的证明。

### 6.3 行为层

- `AlarmRepo::count_since` 的跨天场景：需构造 `occurred_at` 与 `created_at` 分属不同日的记录，断言按 `occurred_at` 计数（AC7）。
- WS 载荷：`alarm_persistence_broadcast_tests.rs` 现有断言覆盖 payload 形状，删除 `severity` 断言即可。

---

## 7. 回滚考量

| 变更 | 回滚方式 | 数据影响 |
| --- | --- | --- |
| R1 pragma | 删除 2 行 pragma | 无（`sqlite_stat1` 残留无害） |
| R2 索引 | `DROP INDEX` | 无（索引可重建） |
| R3 时间轴 | 改回 `CreatedAt` / `NaiveDateTime` | 无（纯读逻辑） |
| R4 severity | `ALTER TABLE ADD COLUMN severity TEXT NOT NULL DEFAULT 'warning'` + 恢复代码 | **无实际数据损失**——该列历史上只有 `warning` 一个取值 |

R4 的回滚成本被"该列从未被真正使用"这一事实大幅降低：不存在需要恢复的业务数据。

---

## 8. 已确认的评审点

1. **迁移拆分 V23（索引）+ V24（DROP COLUMN）** —— 用户选择 C 方案后确定。
2. **`count_since` 改为帧时间轴** —— 用户确认「按帧时间」。语义变化：补录/时移帧归属到其**实际发生日**，与列表筛选和 `CaptureRepo::count_since` 自洽。
3. **破坏性 API 变更（响应体删除 `severity`）无兼容层** —— 依据：仓库内无第三方 API 消费方证据（无 API 客户端/SDK 目录），前后端同仓单二进制交付，且 serde 默认忽略未知查询参数。

以上三点均已确认，可进入实施。
