-- V2__evidence_triad_and_galleries.sql
-- 业务证据三支柱与特征底库数据表

-- 1. 行迹抓拍表 (capture_records)
CREATE TABLE IF NOT EXISTS capture_records (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    capture_id TEXT NOT NULL UNIQUE,
    camera_id TEXT NOT NULL,
    track_id INTEGER NOT NULL DEFAULT 0,
    target_label TEXT NOT NULL,
    confidence REAL NOT NULL DEFAULT 0.0,
    quality_score REAL NOT NULL DEFAULT 0.0,
    bbox_json TEXT NOT NULL DEFAULT '[]',
    image_id TEXT NOT NULL DEFAULT '',
    image_rel_path TEXT NOT NULL DEFAULT '',
    crop_image_id TEXT NOT NULL DEFAULT '',
    crop_image_rel_path TEXT NOT NULL DEFAULT '',
    captured_at DATETIME NOT NULL,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_capture_records_camera_time ON capture_records(camera_id, captured_at DESC);
CREATE INDEX IF NOT EXISTS idx_capture_records_track ON capture_records(camera_id, track_id);

-- 2. 违规告警表 (alarm_records) 扩充特写图、规则类型与违规等级
ALTER TABLE alarm_records ADD COLUMN crop_image_id TEXT NOT NULL DEFAULT '';
ALTER TABLE alarm_records ADD COLUMN crop_image_rel_path TEXT NOT NULL DEFAULT '';
ALTER TABLE alarm_records ADD COLUMN rule_type TEXT NOT NULL DEFAULT '';
ALTER TABLE alarm_records ADD COLUMN severity TEXT NOT NULL DEFAULT 'warning';

-- 3. 识别对账表 (recognition_records)
CREATE TABLE IF NOT EXISTS recognition_records (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    recognition_id TEXT NOT NULL UNIQUE,
    camera_id TEXT NOT NULL,
    gallery_id TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    subject_name TEXT NOT NULL,
    similarity REAL NOT NULL DEFAULT 0.0,
    field_crop_path TEXT NOT NULL DEFAULT '',
    registered_photo_path TEXT NOT NULL DEFAULT '',
    recognized_at DATETIME NOT NULL,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_recognition_records_camera_time ON recognition_records(camera_id, recognized_at DESC);
CREATE INDEX IF NOT EXISTS idx_recognition_records_subject ON recognition_records(subject_id);

-- 4. 人员/车辆底库表 (galleries)
CREATE TABLE IF NOT EXISTS galleries (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    gallery_id TEXT NOT NULL,
    subject_id TEXT NOT NULL UNIQUE,
    subject_name TEXT NOT NULL,
    subject_type TEXT NOT NULL DEFAULT 'person',
    id_card TEXT NOT NULL DEFAULT '',
    plate_number TEXT NOT NULL DEFAULT '',
    feature_vector BLOB,
    photo_rel_path TEXT NOT NULL DEFAULT '',
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_galleries_gallery_subject ON galleries(gallery_id, subject_id);
