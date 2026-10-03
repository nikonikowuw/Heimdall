# NPU 亲和配置持久化、版本栅栏与部分隔离需求文档 (PRD)

> 所属阶段：母任务 `10-01-npu-core-allocation` 阶段 D（子任务 4）。  
> 对应计划代号：`10-03-npu-persistence-revision-barrier`。

## 1. 目标与背景 (Goal & Background)

前序子任务 1~3 已完成 Worker 隔离协议（Subtask 1）、宿主确定性放置账本（Subtask 2）以及 SDK 放置注入与物理权重共享运行时（Subtask 3）。本子任务（阶段 D）聚焦于**持久层持久化、版本栅栏防旧覆盖、冷启动单实例失败隔离，以及 API/Web 跨层兼容**：

1. **加法迁移与事务安全持久化**：SQLite `algorithm_instances` 增加 `affinity_json` 字段（V25 迁移，默认 `{"mode":"auto","policy":"spread"}`），在单事务内保证任务、实例配置与摄像头码流模式（`stream_mode`）原子回写，消除局部更新风险；
2. **版本栅栏与防旧覆盖（Revision Barrier）**：重构 `AlgorithmInstanceRepo`，使用带 `instance_id` 与 `desired_revision == targetRevision` 的条件 SQL 更新检查受影响行数，杜绝迟到的旧版本（成功/失败/pending）覆盖已更新的更高版本配置（修复历史缺陷）；
3. **冷启动与运行时单实例故障隔离**：当某摄像头配置多个算法实例且其中一个算法实例启动/设核失败时，仅该实例进入 Failed 状态，不得连坐停止健康的兄弟实例或同路视频管道；
4. **API 与前端兼容**：更新 `TaskAlgorithmInstanceDto` 与 Web 接口类型，确保无论省略/显式 null/旧单算法桥接，原有亲和配置均不丢失，且无副作用。

## 2. 需求清单 (Requirements)

- **R07-1 加法迁移与向前兼容**：通过 `V25__algorithm_instance_affinity.sql` 增加 `affinity_json TEXT NOT NULL DEFAULT '{"mode":"auto","policy":"spread"}'`，旧数据平滑升级。
- **R07-2 事务内解析与属性三态合并**：`TaskRepo` 在单事务内解析“未提交/重置/替换”，缺失值一直传到事务内合并，杜绝在 API 读取旧值后全量盲目回写导致的并发丢失。
- **R07-3 实例身份完整性校验**：显式提交的 `instance_id` 必须属于目标任务且 `algorithm_id` 一致；跨任务 ID、错配 ID、重复 ID 必须在写入前严格拒绝。无 ID 旧客户端按同任务唯一 `algorithm_id` 匹配已有实例并保留 ID 与原有 affinity。保持 `UNIQUE(task_id, algorithm_id)`。
- **R02/R16 事务原子性**：将摄像头的 `stream_mode` 配置修改与任务、实例的保存纳入同一个数据库事务；任何校验失败或版本冲突不得留下半更新。
- **R04/R15 条件更新与版本栅栏**：`AlgorithmInstanceRepo::mark_apply_applied`、`mark_apply_failed` 与 `mark_apply_pending` 必须通过带 `desired_revision = target_revision` 的条件更新执行，若影响行数为 0 则诊断为过期丢弃，严禁旧结果复活实例或污染新版本。
- **R07/R25 等价配置不重建**：`desiredRevision` 仅在有效期望配置（参数、帧率、启停、亲和意图）改变时递增；等价亲和归一化（如省略 vs `auto/spread`）与纯规则镜像更新不递增 `desired_revision`，不触发硬件重建。
- **R12/R13 冷启动单实例故障隔离**：同摄像头多算法并发启动时，单个实例的 Manual 设核失败或硬件异常仅标记该实例为失败，其余健康实例正常进入运行态，任务状态聚合为 Degraded/Error 但不停止运行。
- **R08/R10 外部接口与 Web 兼容**：API 端点保持 `/api/v1` 契约，DTO 增加可选 `affinity` 字段并正确序列化/反序列化；Web 端类型同步，不产生回归。

## 3. 验收标准 (Acceptance Criteria)

- [ ] **AC07**: 持久化/重启/任务编辑往返保留 affinity 配置；省略/null/auto/manual 各场景均回归测试通过。
- [ ] **AC13**: 同摄像头多算法冷启动时，单实例失败不拖停同路/其他摄像头算法（T05）；换核不停止兄弟实例。
- [ ] **AC15**: 旧成功/失败/pending 迟到回调不覆盖新版本（T04, T18）；条件更新 0 行正确识别并丢弃。
- [ ] **AC16**: 非法 manual affinity、跨任务 ID 错配整体拒绝且无部分写入，`stream_mode` 与任务配置保持原子（T15, T16, T17）。
- [ ] **AC17**: 无 reservation 失败、旧资源继续运行、独立实例关联共享权重在 API 与 DTO 中准确可查。
- [ ] **AC25**: 等价 affinity 归一化与无关规则镜像变更不触发重建；总闸 `PUT /tasks/{id}/enabled` 不修改 affinity 或实例分闸（T39, T40）。

## 4. 边界与约束 (Constraints & Non-Goals)

- **非目标**：本子任务不修改底层 NPU 驱动或硬件算子，专注在宿主持久化、协调器生命周期与版本栅栏；
- **零 CPU 像素拷贝**：保持全链路零拷贝与三路径边界，不在此处引入任何像素缓冲转换；
- **并发锁约束**：保持短锁约定，数据库事务与异步 IO 不进入内存锁（如 Ledger 内部锁）。
