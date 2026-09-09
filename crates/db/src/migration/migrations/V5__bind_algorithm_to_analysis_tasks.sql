-- V5__bind_algorithm_to_analysis_tasks.sql
-- 任务表增补算法包绑定、分析帧率、算法参数与状态消息

ALTER TABLE analysis_tasks ADD COLUMN algorithm_id TEXT NOT NULL DEFAULT '';
ALTER TABLE analysis_tasks ADD COLUMN analysis_fps INTEGER NOT NULL DEFAULT 0;
ALTER TABLE analysis_tasks ADD COLUMN algo_params_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE analysis_tasks ADD COLUMN status_message TEXT NOT NULL DEFAULT '';

CREATE INDEX IF NOT EXISTS idx_analysis_tasks_algorithm_id ON analysis_tasks(algorithm_id);
