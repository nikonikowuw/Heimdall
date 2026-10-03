# NPU 亲和配置持久化、版本栅栏与部分隔离实施计划 (Implement)

> 所属阶段：母任务 `10-01-npu-core-allocation` 阶段 D（子任务 4）。  
> 对应计划代号：`10-03-npu-persistence-revision-barrier`。

## 1. 实施检查清单 (Checklist)

### Phase 1: 自动化测试用例先行 (T04, T05, T15-T18, T22-T24, T30, T39, T40)
- [ ] 在 `crates/db/tests/task_repo_tests.rs` 与 `crates/db/tests/migration_tests.rs` 增加：
  - `test_v25_affinity_migration_existing_rows_default_auto`: 验证旧实例行迁移后默认填充 auto spread；
  - `test_revision_barrier_condition_update_drops_stale_results_t04`: 验证 revision N 的晚到结果（成功/失败）无法覆盖已更新至 N+1 的行，受影响行数为 0；
  - `test_atomic_camera_stream_mode_and_task_save_t15`: 验证在 revision 冲突或非法 manual 时，`stream_mode` 与任务配置均回滚且无部分写入；
  - `test_omitted_affinity_and_instance_id_preserves_existing_config_t16`: 验证旧客户端未提供 affinity 与 instance_id 时，库内既有配置完整保留；
  - `test_cross_task_or_mismatched_instance_id_rejected_t17`: 验证非法 instance_id 传入时整体事务拒绝；
  - `test_equivalent_affinity_and_rule_mirror_does_not_bump_revision_t39`: 验证 `auto` 与 `auto/spread` 往返等价判定，以及纯规则镜像变更不触发 `desired_revision` 递增；
  - `test_set_task_enabled_does_not_mutate_affinity_or_sub_switches_t40`: 验证翻转总闸不改变各实例的亲和配置与各自的分闸。
- [ ] 在 `crates/pipeline/tests/coordinator_lifecycle_tests.rs` 增加：
  - `test_cold_start_single_instance_failure_isolation_t05`: 验证同摄像头配置 2 个算法实例，1 个失败时另 1 个正常运行，管线不停止。

### Phase 2: 数据库迁移与实体更新
- [ ] 编写 `crates/db/src/migration/migrations/V25__algorithm_instance_affinity.sql`。
- [ ] 更新 `crates/db/src/entity/algorithm_instance.rs`，添加 `affinity_json: String` 与 `affinity_intent()` 辅助解析方法。
- [ ] 更新 `crates/db/src/migration/mod.rs`（若有硬编码枚举或静态映射）。

### Phase 3: TaskRepo 单事务与实例身份防御
- [ ] 扩展 `crates/db/src/repository/task.rs` 中的 `SaveTaskWithInstancesParams` 与 `SaveTaskAlgorithmInstanceParams`，增加 `stream_mode: Option<types::StreamMode>`、`instance_id: Option<String>`、`affinity_json: Option<String>`。
- [ ] 在 `save_task_with_instances_txn` 中：
  - 合并 `stream_mode` 写入 `cameras` 表；
  - 校验显式 `instance_id` 的任务归属与 `algorithm_id` 一致性；
  - 旧行缺失 `affinity_json` 时保留原有值；
  - 接入 `desired_revision` 变更判定（对比参数、帧率、分闸与归一化后的亲和意图）。

### Phase 4: AlgorithmInstanceRepo 条件更新版本栅栏
- [ ] 重构 `crates/db/src/repository/algorithm_instance.rs` 中的 `mark_apply_applied`、`mark_apply_failed`、`mark_apply_pending`：
  - 统一入参加入 `target_revision: i64`；
  - 改为使用基于 `Column::InstanceId.eq(instance_id).and(Column::DesiredRevision.eq(target_revision))` 的条件 SQL 更新；
  - 检查 `rows_affected`，若为 0 返回 `Ok(false)` / `Ok(None)` 并记录可诊断日志。

### Phase 5: Pipeline 协调器单实例故障隔离与全链路 TargetRevision
- [ ] 在 `crates/pipeline/src/coordinator.rs` 中：
  - 确保实例启动时携带正确的 `desired_revision` 作为 `target_revision`；
  - 在 `sync_camera_instances` 与启动循环中，单个实例报错时捕获并标记失败，不中断其他实例挂载与数据流；
  - 仅当全部实例均失败时，才标记整体启动失败。

### Phase 6: API DTO 与 Web 跨层兼容
- [ ] 在 `crates/api/src/routes/task.rs` 中：
  - 更新 `TaskAlgorithmInstanceDto` 与 `TaskAlgorithmInstanceSummaryDto`，增加 `pub affinity: Option<types::AffinityIntent>`；
  - `TaskRepo::save_task_with_instances` 调用传入 API 接收到的 `stream_mode`，移除先单独调用 `CameraRepo::set_stream_mode` 的独立步骤；
  - 验证 `PUT /api/v1/tasks/{cameraId}` 接口行为与错误响应。
- [ ] 在 `web/src/api/`（或相关类型定义文件）中更新 TypeScript DTO 接口，声明可选 `affinity?: AffinityIntent` 字段。

### Phase 7: 全量验证门禁
- [ ] 运行 `cargo fmt --all -- --check`。
- [ ] 运行 `cargo clippy --all-targets -- -D warnings`。
- [ ] 运行 `cargo nextest run --workspace`。
- [ ] 运行全部算法包平台编译检查（macOS, RK3568, RK3576, RK3588）。
- [ ] 运行 Web 前端检查 (`pnpm lint`, `pnpm typecheck`, `pnpm test`)。

---

## 2. 门禁验证命令

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo nextest run --workspace
cargo nextest run -p heimdall-db
cargo nextest run -p heimdall-pipeline
cargo nextest run -p heimdall-api

for manifest in \
  algo-packages/macos/Cargo.toml \
  algo-packages/rknn/rk3568/Cargo.toml \
  algo-packages/rknn/rk3576/Cargo.toml \
  algo-packages/rknn/rk3588/Cargo.toml
do
  cargo fmt --manifest-path "$manifest" --all -- --check
  cargo clippy --manifest-path "$manifest" --workspace --all-targets -- -D warnings
  cargo nextest run --manifest-path "$manifest" --workspace
done

cd web
pnpm lint
pnpm typecheck
pnpm test
```
