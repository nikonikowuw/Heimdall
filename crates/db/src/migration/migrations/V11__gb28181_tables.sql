-- V11__gb28181_tables.sql
-- GB/T 28181 国标协议配置、已注册设备及通道表

CREATE TABLE IF NOT EXISTS sys_gb28181_config (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    sip_id TEXT NOT NULL DEFAULT '34020000002000000001',
    sip_domain TEXT NOT NULL DEFAULT '3402000000',
    sip_port INTEGER NOT NULL DEFAULT 5060,
    sip_password TEXT NOT NULL DEFAULT 'admin123',
    rtp_port_range_start INTEGER NOT NULL DEFAULT 30000,
    rtp_port_range_end INTEGER NOT NULL DEFAULT 30500,
    auto_catalog_sync INTEGER NOT NULL DEFAULT 1,
    heartbeat_timeout_sec INTEGER NOT NULL DEFAULT 180,
    updated_at_ms INTEGER NOT NULL
);

INSERT OR IGNORE INTO sys_gb28181_config (
    id, sip_id, sip_domain, sip_port, sip_password,
    rtp_port_range_start, rtp_port_range_end, auto_catalog_sync,
    heartbeat_timeout_sec, updated_at_ms
) VALUES (
    1, '34020000002000000001', '3402000000', 5060, 'admin123',
    30000, 30500, 1, 180, 0
);

CREATE TABLE IF NOT EXISTS gb28181_devices (
    device_id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    ip_addr TEXT NOT NULL,
    sip_port INTEGER NOT NULL,
    transport TEXT NOT NULL DEFAULT 'udp',
    status TEXT NOT NULL DEFAULT 'online',
    channel_count INTEGER NOT NULL DEFAULT 0,
    last_keepalive_ms INTEGER NOT NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS gb28181_channels (
    device_id TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    name TEXT NOT NULL,
    manufacturer TEXT NOT NULL DEFAULT '',
    model TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'ON',
    parent_id TEXT NOT NULL DEFAULT '',
    sub_stream_supported INTEGER NOT NULL DEFAULT 1,
    last_seen_ms INTEGER NOT NULL,
    PRIMARY KEY (device_id, channel_id),
    FOREIGN KEY(device_id) REFERENCES gb28181_devices(device_id) ON DELETE CASCADE
);
