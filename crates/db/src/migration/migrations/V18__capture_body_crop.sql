-- 抓拍记录的人体特写（行迹回溯与人工复核的主体证据）
--
-- 背景：抓拍语义从「人脸通行证据」收敛为「人体通行证据」。此前特写裁剪目标是
-- `face_bbox.unwrap_or(bbox)`，即**有脸时裁的是人脸**，无脸时根本不落库——背身、低头的
-- 目标既没有记录、也没有可用于辨认衣着的图。本次迁移为该产物开两列独立落库：
--   * `crop_image_rel_path`     人脸特写（有脸时；识别复核的证据图，语义不变）
--   * `body_crop_image_rel_path` 人体特写（抓拍记录恒产出；人工复查看衣着）
-- 空串表示"本次未产出"，与 `image_source` 的空串语义一致，读取侧归一为空值/回退展示。
--
-- 历史行不回填：旧记录的 `crop_image_rel_path` 可能是人脸特写也可能是目标特写，
-- 且不存在人体特写，用近似值冒充会产生无法解释的图源。
ALTER TABLE capture_records ADD COLUMN body_crop_image_id TEXT NOT NULL DEFAULT '';
ALTER TABLE capture_records ADD COLUMN body_crop_image_rel_path TEXT NOT NULL DEFAULT '';
