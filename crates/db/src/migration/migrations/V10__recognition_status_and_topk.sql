-- 识别对账表扩展状态字段与 Top-K 候选 JSON
ALTER TABLE recognition_records ADD COLUMN status TEXT NOT NULL DEFAULT 'confirmed';
ALTER TABLE recognition_records ADD COLUMN candidates_json TEXT;
ALTER TABLE recognition_records ADD COLUMN reviewer_id TEXT;
ALTER TABLE recognition_records ADD COLUMN reviewed_at DATETIME;

CREATE INDEX IF NOT EXISTS idx_recognition_records_status ON recognition_records(status);
