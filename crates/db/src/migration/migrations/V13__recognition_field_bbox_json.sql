-- 识别对账表扩展现场目标框数据
ALTER TABLE recognition_records ADD COLUMN field_bbox_json TEXT NOT NULL DEFAULT '';

-- 既有识别记录复用抓拍表中已持久化的人体/人脸框。
UPDATE recognition_records
SET field_bbox_json = (
    SELECT c.bbox_json
    FROM capture_records AS c
    WHERE c.camera_id = recognition_records.camera_id
      AND c.crop_image_rel_path = recognition_records.field_crop_path
      AND c.bbox_json <> ''
    ORDER BY c.captured_at DESC, c.id DESC
    LIMIT 1
)
WHERE field_bbox_json = ''
  AND field_crop_path <> ''
  AND EXISTS (
      SELECT 1
      FROM capture_records AS c
      WHERE c.camera_id = recognition_records.camera_id
        AND c.crop_image_rel_path = recognition_records.field_crop_path
        AND c.bbox_json <> ''
  );
