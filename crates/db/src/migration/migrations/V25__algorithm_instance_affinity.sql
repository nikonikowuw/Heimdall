-- V25__algorithm_instance_affinity.sql
-- 为 algorithm_instances 表增加 affinity_json 列以持久化 NPU 核心分配与卡亲和意图
-- 默认值: '{"mode":"auto","policy":"spread"}'（与既有系统 auto spread 默认行为保持一致）

ALTER TABLE algorithm_instances ADD COLUMN affinity_json TEXT NOT NULL DEFAULT '{"mode":"auto","policy":"spread"}';
