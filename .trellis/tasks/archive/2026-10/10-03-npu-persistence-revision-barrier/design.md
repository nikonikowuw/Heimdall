# NPU 亲和配置持久化、版本栅栏与部分隔离技术设计 (Design)

> 所属阶段：母任务 `10-01-npu-core-allocation` 阶段 D（子任务 4）。  
> 对应计划代号：`10-03-npu-persistence-revision-barrier`。

## 1. 架构总览与模块拓扑

```
[ Web / REST Client ]
         │ (HTTP PUT /api/v1/tasks/{cameraId} with TaskConfigDto)
         ▼
┌────────────────────────────────────────────────────────┐
│                      crates/api                        │
│  - TaskAlgorithmInstanceDto (affinity: Option<Intent>)  │
│  - 前置校验 (manual 合法性、跨任务 ID 防御)             │
└──────────────────────────┬─────────────────────────────┘
                           │ 单事务调用
                           ▼
┌────────────────────────────────────────────────────────┐
│                       crates/db                        │
│  - V25 迁移: affinity_json 加法列                       │
│  - TaskRepo::save_task_with_instances_txn              │
│    (camera.stream_mode + task + instances 原子事务)    │
│  - AlgorithmInstanceRepo 条件更新版本栅栏               │
│    WHERE instance_id = ? AND desired_revision = ?      │
└──────────────────────────┬─────────────────────────────┘
                           │ 启动/收敛调用
                           ▼
┌────────────────────────────────────────────────────────┐
│                    crates/pipeline                     │
│  - PipelineCoordinator::start_camera_instances         │
│  - 部分启动失败隔离 (单实例 manual 失败不连坐兄弟实例)     │
│  - 真实 target_revision 贯穿调度全生命周期             │
└────────────────────────────────────────────────────────┘
```

---

## 2. 数据库加法迁移与模型扩展

### 2.1 V25 数据库迁移脚本
在 `crates/db/src/migration/migrations/V25__algorithm_instance_affinity.sql`：
```sql
-- V25__algorithm_instance_affinity.sql
-- 为 algorithm_instances 增加 affinity_json 字段以持久化 NPU 核心分配与卡亲和意图
-- 默认值: '{"mode":"auto","policy":"spread"}'（与既有系统 auto spread 默认行为逐位一致）

ALTER TABLE algorithm_instances ADD COLUMN affinity_json TEXT NOT NULL DEFAULT '{"mode":"auto","policy":"spread"}';
```

### 2.2 实体模型 (`crates/db/src/entity/algorithm_instance.rs`)
扩充 `Model` 字段：
```rust
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "algorithm_instances")]
pub struct Model {
    ...
    #[sea_orm(column_type = "Text")]
    pub affinity_json: String,
    ...
}

impl Model {
    /// 解析放置亲和意图；若反序列化失败回退为默认 auto spread
    pub fn affinity_intent(&self) -> types::AffinityIntent {
        serde_json::from_str(&self.affinity_json).unwrap_or_default()
    }
}
```

---

## 3. 事务原子性与实例身份完整性

### 3.1 跨表与跨字段原子事务
在 `TaskRepo::save_task_with_instances_txn` 中：
1. **合并 `stream_mode` 写入**：
   - 现存问题：`routes/task.rs` 原先在进入任务保存前先行单独 `CameraRepo::set_stream_mode`，若随后任务版本冲突（409）或校验失败，摄像头模式已被脏写；
   - 改造方案：将 `stream_mode` 作为 `SaveTaskWithInstancesParams` 的可选字段传入，在同一 `DatabaseTransaction` 内部更新 `cameras` 表与 `analysis_tasks` 表，实现全有或全无（All-or-Nothing）。

2. **实例身份与三态合并防御**：
   - 显式传入 `instance_id`：必须校验其属于当前 `task_id` 且 `algorithm_id` 一致；若检测到跨任务、不存在或同任务重复的 `instance_id`，立即拒绝（返回 `DbError::Validation`），绝无部分写入；
   - 未传入 `instance_id`（兼容旧客户端/未填表单）：按同任务唯一 `algorithm_id` 匹配库中已有行，保留已有行的 `instance_id` 与库内现有的 `affinity_json`（不因未提交字段而被置为默认 auto）；
   - 新增算法：系统生成新的 UUIDv7 `instance_id`，写入指定的 `affinity_json`（若未指定则填默认 auto）。

3. **`desired_revision` 递增不变量**：
   - 仅在以下有效期望字段发生变更时递增：`analysis_fps`、`params_json`、`enabled`、归一化后的 `affinity_json`；
   - 等价归一化防护：`{"mode":"auto"}` 与 `{"mode":"auto","policy":"spread"}` 判定为等价，不递增 `desired_revision`；
   - 纯 rules 规则与 motion_gate 门控镜像写入不递增 `desired_revision`，不产生多余的 pending 状态与硬件重配。

---

## 4. 条件更新版本栅栏 (Revision Barrier)

### 4.1 传统“查后再改”的并发漏洞
原 `mark_apply_applied`、`mark_apply_failed` 与 `mark_apply_pending` 使用如下伪代码：
```rust
let model = find_by_instance_id(...);
if model.desired_revision != target_revision { return; }
active.update(db).await; // 漏洞：在 SELECT 与 UPDATE 之间，若新配置提交，新配置将被旧结果覆盖！
```
特别是在 `mark_apply_failed` 中，甚至完全没有判断 `desired_revision`，导致旧 revision N 的失败会直接覆盖新 revision N+1 的 pending 或 applied 状态。

### 4.2 条件 SQL 栅栏实现
全面重构 `AlgorithmInstanceRepo`，使用单条条件更新 SQL：
```rust
pub async fn mark_apply_applied(
    db: &DatabaseConnection,
    instance_id: &str,
    target_revision: i64,
) -> Result<bool, DbError> {
    let now = chrono::Utc::now();
    // 使用纯原生条件更新或 Sea-ORM filter update:
    let res = Entity::update_many()
        .filter(Column::InstanceId.eq(instance_id))
        .filter(Column::DesiredRevision.eq(target_revision))
        .col_expr(Column::AppliedRevision, Expr::value(target_revision))
        .col_expr(Column::RuntimeApplyState, Expr::value(types::InstanceApplyState::Applied.as_i32()))
        .col_expr(Column::StatusMessage, Expr::value(""))
        .col_expr(Column::UpdatedAt, Expr::value(now))
        .exec(db)
        .await?;
    Ok(res.rows_affected > 0)
}

pub async fn mark_apply_failed(
    db: &DatabaseConnection,
    instance_id: &str,
    target_revision: i64,
    reason: &str,
) -> Result<bool, DbError> {
    let now = chrono::Utc::now();
    let res = Entity::update_many()
        .filter(Column::InstanceId.eq(instance_id))
        .filter(Column::DesiredRevision.eq(target_revision))
        .col_expr(Column::RuntimeApplyState, Expr::value(types::InstanceApplyState::Failed.as_i32()))
        .col_expr(Column::StatusMessage, Expr::value(reason))
        .col_expr(Column::UpdatedAt, Expr::value(now))
        .exec(db)
        .await?;
    Ok(res.rows_affected > 0)
}
```
当 `rows_affected == 0` 时：
- 说明该结果抵达时库中已有更高代际被保存，或该实例已被删除；
- 记录 Debug/Info 日志，丢弃该迟到结果，绝不覆盖新版本。

---

## 5. 冷启动与运行时单实例故障隔离

### 5.1 实例独立错误传播
在 `crates/pipeline/src/coordinator.rs` 中：
1. 启动摄像头管线并挂载多算法实例时，逐个实例应用 placement 预留并启动 Worker；
2. 当某实例 `A` 因 manual 核心不存在或模型初始化失败时：
   - 记录实例 `A` 的失败结果（调用 `mark_apply_failed(instance_A, target_rev, reason)`）；
   - **不调用全局 `stop_pipeline` 或中断其他实例**；
   - 实例 `B` 与 `C` 继续正常初始化并进入 `Running`；
3. 聚合任务状态：
   - `aggregate_task_instance_status` 计算得出 `TaskStatus::Degraded`（部分运行）或 `TaskStatus::Error`；
   - 视频解码泵与健康算法实例持续工作，只有当**所有**算法实例均失败时，才安全进入停止阶段，绝不影响其他摄像头。

---

## 6. API 与 Web 跨层契约

### 6.1 API DTO 增强 (`crates/api/src/routes/task.rs`)
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskAlgorithmInstanceDto {
    ...
    #[serde(default)]
    pub affinity: Option<types::AffinityIntent>,
    ...
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskAlgorithmInstanceSummaryDto {
    ...
    #[serde(default)]
    pub affinity: Option<types::AffinityIntent>,
    ...
}
```

### 6.2 前端 Web 类型兼容 (`web/src/api/types/task.ts`)
在前端 DTO 接口中增加可选的 `affinity?: AffinityIntent`，确保表单读取与回传保持字段完整，省略时优雅兼容。
