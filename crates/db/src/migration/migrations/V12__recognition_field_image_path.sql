-- 识别对账表扩展现场全景大图路径
ALTER TABLE recognition_records ADD COLUMN field_image_path TEXT NOT NULL DEFAULT '';

-- 回填按摄像头和特写路径关联，先建立复合索引避免大库迁移时全表扫描。
CREATE INDEX IF NOT EXISTS idx_capture_records_camera_crop
    ON capture_records(camera_id, crop_image_rel_path);

-- 既有识别记录通过现场特写路径关联抓拍表，尽可能回填对应全景图。
UPDATE recognition_records
SET field_image_path = (
    SELECT c.image_rel_path
    FROM capture_records AS c
    WHERE c.camera_id = recognition_records.camera_id
      AND c.crop_image_rel_path = recognition_records.field_crop_path
      AND c.image_rel_path <> ''
    ORDER BY c.captured_at DESC, c.id DESC
    LIMIT 1
)
WHERE field_image_path = ''
  AND field_crop_path <> ''
  AND EXISTS (
      SELECT 1
      FROM capture_records AS c
      WHERE c.camera_id = recognition_records.camera_id
        AND c.crop_image_rel_path = recognition_records.field_crop_path
        AND c.image_rel_path <> ''
  );
