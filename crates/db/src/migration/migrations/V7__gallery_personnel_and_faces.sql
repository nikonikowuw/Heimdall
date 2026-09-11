-- V7__gallery_personnel_and_faces.sql
-- 人员档案与人脸特征样本库

-- 1. 人员主体档案表 (personnel)
CREATE TABLE IF NOT EXISTS personnel (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    subject_id TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    id_card TEXT NOT NULL DEFAULT '',
    remark TEXT NOT NULL DEFAULT '',
    primary_photo_path TEXT NOT NULL DEFAULT '',
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_personnel_subject_id ON personnel(subject_id);
CREATE INDEX IF NOT EXISTS idx_personnel_name ON personnel(name);

-- 2. 人脸特征样本从表 (gallery_faces)
CREATE TABLE IF NOT EXISTS gallery_faces (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    face_id TEXT NOT NULL UNIQUE,
    subject_id TEXT NOT NULL,
    photo_rel_path TEXT NOT NULL,
    aligned_rel_path TEXT NOT NULL DEFAULT '',
    feature_vector BLOB NOT NULL,
    quality_score REAL NOT NULL DEFAULT 0.0,
    detection_score REAL NOT NULL DEFAULT 0.0,
    is_primary INTEGER NOT NULL DEFAULT 0,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(subject_id) REFERENCES personnel(subject_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_gallery_faces_subject ON gallery_faces(subject_id);
CREATE INDEX IF NOT EXISTS idx_gallery_faces_face_id ON gallery_faces(face_id);
