-- 算法实例两阶段配置提交：期望配置 revision、已应用 revision 与运行时应用状态
--
-- 此前「应用参数」把整路摄像头分析管线停掉重启，数据库写入成功即被视为生效，
-- 前端无法分辨「已持久化」与「目标 Worker 真的在用这份配置」。
--
-- 1) desired_revision：用户期望配置的代际，每次通过实例级 Apply 提交期望配置时 +1。
-- 2) applied_revision：运行时确认目标 Worker 已使用该代际配置的代际；相等即已收敛。
-- 3) runtime_apply_state：0=applied / 1=pending / 2=failed，与 actual_status 的健康
--    生命周期语义正交，不复用 `actual_status`，避免把「配置没生效」伪装成「实例不健康」。
--
-- 存量行一律 0/0/applied：迁移前不存在期望与实际不一致的概念，不回填伪造的中间态。
ALTER TABLE algorithm_instances ADD COLUMN desired_revision INTEGER NOT NULL DEFAULT 0;
ALTER TABLE algorithm_instances ADD COLUMN applied_revision INTEGER NOT NULL DEFAULT 0;
ALTER TABLE algorithm_instances ADD COLUMN runtime_apply_state INTEGER NOT NULL DEFAULT 0;
