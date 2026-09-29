# db — 规范入口

适用于 `crates/db`：SQLite (WAL)、SeaORM 与 Refinery 迁移、Repository。仓库级契约见 [AGENTS.md](../../../../AGENTS.md)。

## Pre-Development Checklist

1. 阅读 [数据库规范](./database-guidelines.md)：连接与迁移、表与查询、任务与算法实例原子同步、写入与存储保护、验证。
2. 存储保护与淘汰硬约束见 [全局约定](../../guides/conventions.md#存储保护)（`statvfs` 水位、单事务级联淘汰）。
3. 错误传播按 [错误处理](../../guides/error-handling.md)；写盘攒批与队列上限见 [全局约定](../../guides/conventions.md#队列与通道)。
4. 查询形态（分页、关键字过滤）与 API 契约对齐：[API 规范](../../api/backend/api-guidelines.md)。

## Quality Check

- 执行 [AGENTS.md](../../../../AGENTS.md) 的 Rust/Native 门禁。
- 按 [数据库规范](./database-guidelines.md) 的验证节与 [质量检查](../../guides/quality-guidelines.md) 选择测试：迁移、淘汰事务、查询形态。
