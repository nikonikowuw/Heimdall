-- 告警查询物理层优化：补齐常态排序流与状态筛选索引，下线死索引
--
-- 背景：alarm_records 此前仅有 idx_alarm_records_camera_time(camera_id, occurred_at DESC)，
-- 首列是 camera_id，而告警页默认态（"全部通道 + 今天"）恰恰不约束 camera_id。
-- 该形态下 SQLite 无法走跳扫：跳扫有硬性前置条件（优化器要求该索引已有 sqlite_stat1），
-- 缺少统计信息时直接退化为全表扫 + USE TEMP B-TREE FOR ORDER BY。
--
-- 索引列序必须完整复刻 list_filtered 的 ORDER BY occurred_at DESC, id DESC：
-- 时间列非唯一（同毫秒批量写入会造出并列行），缺 id 兜底时仍会引入临时排序。

CREATE INDEX IF NOT EXISTS idx_alarm_records_time
ON alarm_records(occurred_at DESC, id DESC);

-- status 是等值过滤放首列，后两列继续复刻排序：同时服务列表与计数，
-- 计数可走 COVERING INDEX（实测 200 万行 8.03ms -> 0.39ms）。
CREATE INDEX IF NOT EXISTS idx_alarm_records_status_time
ON alarm_records(status, occurred_at DESC, id DESC);

-- 死索引：全仓无任何按 crop_image_rel_path 过滤的查询（该列仅出现在 select_only 投影中），
-- 保留它只有写入放大与空间开销。
DROP INDEX IF EXISTS idx_capture_records_camera_crop;
