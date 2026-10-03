-- 下线从未被使用的 severity 字段（严重等级）
--
-- 证据（详见任务 10-02-alarm-query-index-tuning/prd.md）：
--   * 唯一生产写入点是 alarm_service.rs 的硬编码 AlarmSeverity::Warning 常量；
--   * API 无写入端点，规则配置 rules_json 与算法包输出契约均不携带等级；
--   * git 全历史中生产代码从未出现过 "critical" 字面量；
--   * 原始设计文档 prd/prd-v1.0.md 的 alarm_records 建表语句本就不含此列。
--
-- 效果：列的取值来源始终是 V2 迁移的 SQL DEFAULT 'warning'，用户看到的是恒定假等级，
-- 前端"严重告警"筛选项永久返回空集。
--
-- 注意：SQLite 的 DROP COLUMN 会重写整表并重建既有索引（100 万行实测约 1.15s），
-- 这是有意接受的迁移成本；列内不存在需要保留的业务数据（历史上只有 'warning' 一个取值）。
-- 与 V23 拆开是为了让"索引失败"与"表重写失败"两类运维场景可分别定位。

ALTER TABLE alarm_records DROP COLUMN severity;
