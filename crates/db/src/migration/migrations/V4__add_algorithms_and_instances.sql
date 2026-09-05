-- V4__add_algorithms_and_instances.sql
-- 算法包与多算法实例持久化 Schema 迁移

CREATE TABLE IF NOT EXISTS algorithms (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    algorithm_id   TEXT NOT NULL UNIQUE,
    name           TEXT NOT NULL,
    algorithm_type TEXT NOT NULL,
    alarm_type_id  TEXT NOT NULL DEFAULT '',
    active_version TEXT NOT NULL DEFAULT '',
    description    TEXT NOT NULL DEFAULT '',
    is_builtin     INTEGER NOT NULL DEFAULT 0,
    created_at     DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at     DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_algorithms_type ON algorithms(algorithm_type);

CREATE TABLE IF NOT EXISTS algorithm_versions (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    algorithm_id        TEXT NOT NULL,
    version             TEXT NOT NULL,
    platform_id         TEXT NOT NULL,
    min_adapter_version TEXT NOT NULL DEFAULT '',
    package_root        TEXT NOT NULL DEFAULT '',
    fps_tiers           TEXT NOT NULL DEFAULT '[]',
    config_schema       TEXT NOT NULL DEFAULT '{}',
    manifest_raw        TEXT NOT NULL DEFAULT '{}',
    package_size_bytes  INTEGER NOT NULL DEFAULT 0,
    is_active           INTEGER NOT NULL DEFAULT 0,
    is_builtin          INTEGER NOT NULL DEFAULT 0,
    created_at          DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at          DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(algorithm_id, version, platform_id)
);

CREATE INDEX IF NOT EXISTS idx_algo_versions_algo_id ON algorithm_versions(algorithm_id);

CREATE TABLE IF NOT EXISTS algorithm_instances (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    instance_id    TEXT NOT NULL UNIQUE,
    camera_id      TEXT NOT NULL,
    algorithm_id   TEXT NOT NULL,
    analysis_fps   INTEGER NOT NULL DEFAULT 0,
    params_json    TEXT NOT NULL DEFAULT '{}',
    rules_json     TEXT NOT NULL DEFAULT '[]',
    motion_gate_json TEXT NOT NULL DEFAULT '{}',
    enabled        INTEGER NOT NULL DEFAULT 0,
    actual_status  INTEGER NOT NULL DEFAULT 0,
    status_message TEXT NOT NULL DEFAULT '',
    created_at     DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at     DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(camera_id) REFERENCES cameras(camera_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_algo_instances_camera ON algorithm_instances(camera_id);
CREATE INDEX IF NOT EXISTS idx_algo_instances_algo ON algorithm_instances(algorithm_id);
