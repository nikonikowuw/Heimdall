-- 摄像头模块与国标通道索引优化

-- 1. 覆盖高频探活健康状态统计（/api/v1/system/overview 轮询 count_healthy），使用部分索引
CREATE INDEX IF NOT EXISTS idx_cameras_healthy_status
ON cameras(last_probe_status)
WHERE last_probe_status = 'healthy';

-- 2. 覆盖国标 GB28181 通道匹配与反查（部分索引，仅索引国标设备）
CREATE INDEX IF NOT EXISTS idx_cameras_gb28181_lookup
ON cameras(gb28181_device_id, gb28181_channel_id)
WHERE protocol = 'gb28181';

-- 3. 覆盖协议筛选
CREATE INDEX IF NOT EXISTS idx_cameras_protocol
ON cameras(protocol);
