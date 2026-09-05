-- V3__alarm_status_processing.sql
-- 为告警表添加处理状态与处理时间

ALTER TABLE alarm_records ADD COLUMN status TEXT NOT NULL DEFAULT 'unprocessed';
ALTER TABLE alarm_records ADD COLUMN handled_at DATETIME;
