-- V19__drop_legacy_galleries_table.sql
-- 移除历史遗留的单表 galleries 表 (已由 personnel 与 gallery_faces 全面替代)

DROP TABLE IF EXISTS galleries;
