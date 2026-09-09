# 数据库规范

实现入口：[db](../../../crates/db/src/lib.rs)、[迁移](../../../crates/db/src/migration/mod.rs)、[Repository](../../../crates/db/src/repository/)。

## 连接与迁移

连接池由 `db` 构建，默认 1～4 个连接，关闭高频 SQL 日志。连接初始化必须保证：

| PRAGMA         | 值       | 目的               |
| -------------- | -------- | ------------------ |
| `journal_mode` | `WAL`    | 降低读写互斥       |
| `synchronous`  | `NORMAL` | 减少 fsync 写放大  |
| `busy_timeout` | `5000`   | 写锁等待 5000ms    |
| `foreign_keys` | `ON`     | 保证外键与级联约束 |

- Refinery 文件：`src/migration/migrations/V{version}__{snake_case_description}.sql`。
- 版本递增，已合并迁移只增不改；修复或回滚用新的前向迁移。
- SQL 通过 `embed_migrations!` 内嵌，启动先迁移再初始化 SeaORM；失败立即退出。

## 表与查询

- 表名单数 snake_case；主键 `id`，事件保留唯一 `event_id: TEXT` 用于幂等。
- 绝对时间用 UTC 毫秒 `INTEGER/i64`，布尔用 0/1；图片/视频存文件，库内只保存相对路径，不存 BLOB。
- Alarms、Captures、Recognitions 分开管理；告警支持待处理/已核验状态流转。
- 查询封装在 Repository，`api` / `pipeline` 不直接使用 SeaORM DSL；列表必须有 `limit`。
- 时间范围与摄像头过滤建立对应复合索引，例如 `(camera_id, timestamp)`；分页遵循 [API 契约](./api-guidelines.md#分页)。

## 写入与存储保护

- 高频写入经过有界通道攒批提交，例如 32 条或 1000ms 到期；满载丢旧并计数告警，禁止逐事件单事务刷盘。
- 写盘前通过 `statvfs` 检查容量、inode 与只读状态，按高低水位清理/停写；禁止用 `du` 递归扫描。
- 先淘汰无告警的普通抓拍；物理文件与 DB 记录在同一清理事务内配套删除，杜绝孤儿文件与死记录。
- SQLite 回滚不等于文件恢复；清理验证必须覆盖文件删除失败、DB 失败及中断后的恢复行为。
- 保留天数/容量必须有限；配置 `auto_vacuum = INCREMENTAL` 并定期 `incremental_vacuum` 回收删除页面。
- [StorageCleaner](../../../crates/pipeline/src/storage_cleaner/mod.rs) 与 Pipeline 使用同一证据目录，在应用启动时注入，不在 Handler 临时创建。

## 验证

使用独立临时 SQLite 文件验证 WAL、迁移、查询上限、幂等与批量写入；证据清理同时断言文件和记录状态。
格式、lint 和测试门禁见 [质量规范](./quality-guidelines.md)。
