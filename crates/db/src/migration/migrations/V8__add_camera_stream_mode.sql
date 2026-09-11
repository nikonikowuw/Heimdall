-- V8: 为摄像头增加分析码流偏好模式 (stream_mode)
-- 支持 'auto' (自动适配), 'main' (主码流高清分析), 'sub' (子码流低能耗分析)
ALTER TABLE cameras ADD COLUMN stream_mode TEXT NOT NULL DEFAULT 'auto';
