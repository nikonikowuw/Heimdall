-- 运维事件日志表：记录系统状态变迁（摄像头上下线、任务状态、存储淘汰等）
-- 与 operation_logs（操作审计）区分：
--   operation_logs = 用户操作记录（who did what）
--   operational_logs = 系统状态变迁（what happened）
CREATE TABLE operational_logs (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    ts_ms       INTEGER NOT NULL,                -- 13位 UTC 毫秒
    level       TEXT    NOT NULL,                 -- error / warn / info
    event       TEXT    NOT NULL,                 -- 事件标记，如 camera_offline
    target      TEXT    NOT NULL DEFAULT '',       -- crate::module，如 media::rtsp
    message     TEXT    NOT NULL,                 -- 人类可读中文短消息
    camera_id   TEXT,                             -- 可选：关联摄像头 ID
    extra_json  TEXT                              -- 可选：变体特有字段 JSON
);

CREATE INDEX idx_oplog_ts      ON operational_logs(ts_ms DESC);
CREATE INDEX idx_oplog_level   ON operational_logs(level);
CREATE INDEX idx_oplog_event   ON operational_logs(event);
CREATE INDEX idx_oplog_camera  ON operational_logs(camera_id);
CREATE INDEX idx_oplog_cam_ts  ON operational_logs(camera_id, ts_ms DESC);
