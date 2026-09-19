-- 日志中心把筛选下推服务端后，条件列需要索引支撑，避免在保留期内退化成全表扫描。
--   operation_logs：status 分类（2xx / 4xx+5xx）与按时间倒序的 offset 翻页
--   operational_logs：target 归属模块过滤
-- 关键字检索（LIKE '%...%'）无法走索引，由保留策略与关键字长度上限约束其代价。
CREATE INDEX IF NOT EXISTS idx_operation_logs_status_time
    ON operation_logs(status_code, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_oplog_target_ts
    ON operational_logs(target, ts_ms DESC);
