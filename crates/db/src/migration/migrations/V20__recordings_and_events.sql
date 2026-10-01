-- 事件录像片段表
CREATE TABLE IF NOT EXISTS recordings (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    recording_id    TEXT    NOT NULL UNIQUE,
    camera_id       TEXT    NOT NULL,
    file_path       TEXT    NOT NULL,
    start_time      INTEGER NOT NULL,
    end_time        INTEGER,
    duration_ms     INTEGER,
    file_size       INTEGER,
    codec           TEXT    NOT NULL DEFAULT 'h264',
    status          TEXT    NOT NULL DEFAULT 'recording',
    created_at      INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_recordings_camera_time ON recordings(camera_id, start_time);
CREATE INDEX IF NOT EXISTS idx_recordings_status ON recordings(status);
CREATE INDEX IF NOT EXISTS idx_recordings_created ON recordings(created_at);

-- 录像-事件关联表（多对多）
CREATE TABLE IF NOT EXISTS recording_events (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    recording_id    TEXT    NOT NULL,
    event_type      TEXT    NOT NULL,
    event_id        TEXT    NOT NULL,
    event_time      INTEGER NOT NULL,
    offset_ms       INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_recording_events_recording ON recording_events(recording_id);
CREATE INDEX IF NOT EXISTS idx_recording_events_event ON recording_events(event_type, event_id);
