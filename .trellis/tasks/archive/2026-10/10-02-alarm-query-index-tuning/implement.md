# 实施与验证计划

对应 `prd.md` R1–R5 与 `design.md`。执行顺序按依赖方向（db → types → api → web → spec），每阶段结束即验证。

## 0. 开工门禁

- [ ] 用户确认 `prd.md` / `design.md` 最新摘要，并明确批准实施（三个评审点已在 `design.md` §8 记录为已确认）。
- [ ] 批准后才执行 `task.py start`；当前保持 planning。
- [ ] 检查 `git status`，保留工作区既有未跟踪/已修改文件（当前有 `web/src/features/tasks/` 与 `10-02-task-list-search-filter` 等无关改动），不覆盖不回退。
- [ ] 按目标包 spec 读取规范：`.trellis/spec/db/backend/index.md`、`.trellis/spec/api/backend/index.md`、`.trellis/spec/web/frontend/index.md`、`.trellis/spec/guides/conventions.md`；上下文清单见 `implement.jsonl` / `check.jsonl`。
- [ ] 确认验收环境磁盘可用空间（`DROP COLUMN` 会重写整表，100 万行约需额外 WAL 空间）。

## 1. 迁移层（R2 / R4.1）

- [ ] `crates/db/src/migration/migrations/V23__alarm_query_indexes.sql`：2 个 `CREATE INDEX IF NOT EXISTS` + 1 个 `DROP INDEX IF EXISTS idx_capture_records_camera_crop`。
- [ ] `crates/db/src/migration/migrations/V24__drop_alarm_severity.sql`：单条 `ALTER TABLE alarm_records DROP COLUMN severity`。
- [ ] 确认 refinery 自动扫描目录（`refinery::embed_migrations!("src/migration/migrations")`），无需手工注册。
- [ ] 临时库验证：跑迁移 → 断言 `refinery_schema_history` 含 V23/V24 → 重跑断言幂等 → `PRAGMA table_info` 无 `severity` → 2 索引存在 → 死索引消失。
- [ ] 验证 `run_migrations_on_seaorm`（`:memory:` 路径）同样通过——该函数手工读取 `migrations::runner()`，需确认 V23/V24 被正确排序执行。

## 2. 统计信息引导（R1）

- [x] `crates/db/src/connection.rs::init_db` pragma 列表增加 `"PRAGMA optimize=0x10012;"`，附注释说明 bitmask 语义（含 `0x10` 位不可省的实测依据，见 design.md §3.2）。
- [ ] `crates/db/src/migration/mod.rs::run_migrations` 的 rusqlite `execute_batch` 同样增加该 pragma。
- [ ] 验证：全新库跑 `init_db` 后 `SELECT name FROM sqlite_master WHERE name='sqlite_stat1'` 非空（AC1）。

## 3. 索引与计划验证（R2 验收）

- [ ] 灌 ≥50 万行合成数据到临时库（时间列用 RFC3339 格式，与 SQLx 写入一致）。
- [ ] 断言 AC2：默认态计划为 `SEARCH ... USING INDEX` 且无 `USE TEMP B-TREE FOR ORDER BY`。
- [ ] 断言 AC3：`DELETE FROM sqlite_stat1` 后计划**保持** `SEARCH`（证明不依赖统计信息）。
- [ ] 断言 AC4：`status` 筛选命中 `idx_alarm_records_status_time`。
- [ ] 记录 AC14 量化对比（改动前后同一查询的计划 + 耗时），写入 `task.json` notes 或任务目录内临时记录，**不入库**。

## 4. `count_since` 时间轴（R3）

> **时间轴已确认**：按帧时间（`occurred_at`）。用户已确认此决定。

- [ ] `crates/db/src/repository/alarm.rs::count_since`：`Column::CreatedAt` → `Column::OccurredAt`。
- [ ] `count_since` 入参 `chrono::NaiveDateTime` → `DateTimeUtc`。**不可省略**：`NaiveDateTime` 被 SQLx 编码为空格分隔格式，与 `occurred_at` 的 RFC3339 存储不匹配，非零点边界会静默丢行（见 `design.md` §4.2.1 实测）。
- [ ] `crates/db/src/repository/capture.rs::count_since` 入参一并收窄为 `DateTimeUtc`（时间轴本已正确，仅类型对齐，见 `design.md` §4.4）。
- [ ] `crates/api/src/routes/system/overview.rs:36-48`：`today_start` 构造改为 UTC 时刻，两个调用点同步。
- [ ] 新增/更新测试：构造 `occurred_at` 在今日、`created_at` 在昨日的记录，断言被 `count_since` 计入（AC7）。
- [ ] 回归验证：非零点边界的正确性——用 `DateTimeUtc` 绑定与用 `NaiveDateTime` 绑定对比，确认后者会多含早于边界的记录（钉住该隐患不会复发）。

## 5. `severity` 下线（R4，自底向上）

> **易误删警告**：三个告警组件中的 `--status-danger-*` 样式大多属于「未处理」状态（`isProcessed`）语义，**不是** severity。严格按 `design.md` §5.3 的归属表区分 `isCritical` 与 `isProcessed` 三元表达式，不得批量替换 `--status-danger-soft`。

- [ ] **db 层**：`entity/alarm.rs:25` 字段删除；`repository/alarm.rs:28` `AlarmFilter.severity` 删除；`:49-50` 过滤分支删除。
- [ ] **types 层**：`types/src/alarm.rs` 删除 `AlarmSeverity` 枚举与 `impl`（`:34-58`）及测试（`:120-145`）；`types/src/lib.rs:15` re-export 删除。
- [ ] **api 层**：`alarm_service.rs:7` import、`:104` 写入、`:136` WS 载荷；`routes/alarm.rs:6` import、`:35` DTO 字段、`:58` 映射、`:72,89` 两个 Query 字段、`:144,172` 两处 `AlarmFilter` 构造。检查 `AlarmDto` 其它字段不依赖 severity 排序/分组。
- [ ] **测试同步**：`crates/pipeline/tests/rules_engine_tests.rs:152`、`crates/db/tests/evidence_repo_tests.rs:173`、`crates/api/tests/alarm_persistence_broadcast_tests.rs:193,322,727,747,767`。
- [ ] **web 层**：按 `design.md` §5 表格逐项删除（类型 → filters → 3 组件 → AlarmsContent 表头 → LivePage ×4 → api.ts ×4）。
- [ ] **i18n**：3 个语言包删除 `columns.severity` / `filter.severityWarning` / `filter.severityCritical`。
- [ ] 确认 `defineFilters` 辅助函数**保留**（仍被 3 张筛选表使用）。
- [ ] 断言 AC9：`grep -rn "severity\|Severity" crates/ web/src/ --include=*.rs --include=*.ts --include=*.tsx` 零命中。
- [ ] 断言 AC8：`PRAGMA table_info` 无 severity；行数与迁移前一致。
- [ ] 断言 AC10：`GET /api/v1/alarms` 响应不含 severity；`?severity=critical` 不报错。
- [ ] 断言 AC11：WS payload 不含 severity。

## 6. 规范回填（R5）

- [ ] `.trellis/spec/db/backend/database-guidelines.md` 补 3 条（R5.1 索引首列未约束 + 统计信息引导；R5.2 `count_*` 时间轴一致性；R5.3 预留字段不得暴露）。
- [ ] 检查 `.trellis/spec/api/backend/api-guidelines.md` 是否记载了 `severity` 参数或字段契约，若有需同步删除。
- [ ] 检查 `.trellis/spec/web/frontend/` 是否记载 severity 相关 UI 契约。

## 7. 门禁

- [ ] `cargo fmt --all`
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets -- -D warnings`
- [ ] `cargo nextest run --workspace`
- [ ] `cd web && pnpm format && pnpm lint && pnpm typecheck && pnpm test && pnpm check:cycles && pnpm build`
- [ ] 确认无 `dbg!` / `println!` / `console.log` / `todo!()` / `@ts-ignore` 残留。

## 8. 风险点与回滚

| 阶段 | 风险 | 回滚 |
| --- | --- | --- |
| V23 | 磁盘空间不足导致 `CREATE INDEX` 失败 | `DROP INDEX`，重跑 |
| V24 | 表重写空间不足 | 迁移 fail-fast，服务不启动（无半迁移状态） |
| V24 后 | 需恢复字段 | `ALTER TABLE ADD COLUMN severity TEXT NOT NULL DEFAULT 'warning'`，无业务数据损失 |
| R3 | 语义变化引起误解 | 改回 `CreatedAt`（纯读逻辑，无数据影响） |
| R4 | API 破坏性变更 | 前后端同仓同步修改，无版本错配 |

## 9. 交付前复核

- [ ] 全部 AC（AC1–AC14）逐条勾选。
- [ ] 迁移在**已有数据的真实库**上验证过（不只空库）。
- [ ] 记录迁移实测耗时（100 万行基准：V23 ≈ 1.03 s，V24 ≈ 0.93 s）与验收环境的实际差异。
- [ ] 复核 `--status-danger-*` 未被误删：`grep -c "status-danger" web/src/features/alarms/components/*.tsx` 结果应少于改动前但**大于 0**，且剩余项均为 `isProcessed` 分支。
- [ ] 复核 `defineFilters` 辅助函数保留（仍服务 `RULE_TYPE_FILTERS` / `ALARM_STATUS_FILTERS` / `RECOGNITION_STATUS_FILTERS`）。
- [ ] 复核 `prd/prd-v1.0.md:361-379` 的原始建表语句无需修改（它本就不含 `severity`，删列后反而与设计文档重新一致）。
