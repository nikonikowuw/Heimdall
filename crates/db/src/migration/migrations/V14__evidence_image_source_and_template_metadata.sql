-- 证据图来源与融合模板元数据落库
--
-- 目标 3「证据可追溯」的最后一块：此前库里只能看到 `image_rel_path` 与 `captured_at`，
-- 无法分辨一张凭据是**结算峰值候选帧**还是**靶向快拍帧**，也无从判断它是主码流高分辨率帧
-- 还是子码流低分辨率帧。融合模板的成熟度同样只在日志里，事后无法解释「抓拍差」是取帧降级
-- 还是质量模型尚未成熟。
--
-- 1) 抓拍表：来源路径 / 码流 / 帧 PTS / 融合模板元数据
ALTER TABLE capture_records ADD COLUMN image_source TEXT NOT NULL DEFAULT '';
ALTER TABLE capture_records ADD COLUMN image_stream TEXT NOT NULL DEFAULT '';
ALTER TABLE capture_records ADD COLUMN image_pts_ms INTEGER NOT NULL DEFAULT 0;
ALTER TABLE capture_records ADD COLUMN fused_count INTEGER;
ALTER TABLE capture_records ADD COLUMN template_quality REAL;

-- 2) 识别对账表：同一组来源与模板元数据（对账记录复用抓拍图鉴）
ALTER TABLE recognition_records ADD COLUMN image_source TEXT NOT NULL DEFAULT '';
ALTER TABLE recognition_records ADD COLUMN image_stream TEXT NOT NULL DEFAULT '';
ALTER TABLE recognition_records ADD COLUMN image_pts_ms INTEGER NOT NULL DEFAULT 0;
ALTER TABLE recognition_records ADD COLUMN fused_count INTEGER;
ALTER TABLE recognition_records ADD COLUMN template_quality REAL;

-- 3) 历史回填：峰值候选机制与本次迁移同批引入，更早的记录必然全部来自靶向快拍路径。
--    `image_stream` / `image_pts_ms` / 融合元数据在旧 schema 中无等价来源，保持「未标注」，
--    不用近似值伪装成已知事实。
UPDATE capture_records SET image_source = 'targeted' WHERE image_source = '';
UPDATE recognition_records SET image_source = 'targeted' WHERE image_source = '';
