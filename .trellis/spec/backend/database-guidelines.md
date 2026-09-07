# 数据库规范 (Database Guidelines)

> 基于 SeaORM + SQLite + Refinery。
> 核心原则：**SQLite 运行在嵌入式 eMMC/SD 存储介质上，写入保护与生命周期策略比查询更重要**。

---

## 1. 连接池与必备 PRAGMA 契约

连接池与连接参数必须在 `crates/db` 中统一构建，其他 crate 仅持有共享连接句柄：

```rust
// crates/db/src/lib.rs
let mut opt = ConnectOptions::new(format!("sqlite://{}?mode=rwc", path.display()));
opt.max_connections(4)          // SQLite 写是串行的，边缘端小连接池足以满足需求
   .min_connections(1)
   .sqlx_logging(false);        // 避免逐帧路径产生海量 SQL 日志刷屏
let db = Database::connect(opt).await?;
```

启动初始化连接时**必须显式执行以下 PRAGMA**（缺一不可）：

| PRAGMA | 取值 | 硬性理由 |
|--------|------|---------|
| `journal_mode` | `WAL` | **核心生命线**：读写互不阻塞，事件写入不锁死 API 查询 |
| `synchronous` | `NORMAL` | WAL 模式下安全且减少 fsync 调用，大幅减轻 eMMC 擦写磨损 |
| `busy_timeout` | `5000` | 写锁争用时等待 5000ms 而非立即报 DatabaseLocked 错误 |
| `foreign_keys` | `ON` | SQLite 默认关闭外键约束，必须显式激活保证级联一致性 |

---

## 2. 数据库迁移规范 (Refinery)

使用 **Refinery** 执行单向前向 SQL 迁移管理：
- **命名规范**：`crates/db/src/migration/migrations/V{version}__{description}.sql`（如 `V1__init_schema.sql`、`V2__add_event_score.sql`）；
- **版本号递增**：版本号为单调递增正整数，双下划线 `__` 分隔，描述使用 snake_case 英文短语；
- **单向只增不改**：已合并的迁移文件永不篡改；若需修复或回滚，编写新的前向递增版本迁移（如 `V3__revert_xxx.sql`）；
- **嵌入编译与启动托管**：使用 `embed_migrations!` 宏将 SQL 文件直接编译进主二进制，应用启动时在初始化 SeaORM 前优先执行 `run_migrations`，迁移失败立即 panic 退出，杜绝在脏版本库上运行。

---

## 3. 表设计核心契约

| 规范项 | 约定 | 说明 |
|-------|------|------|
| **表名** | 单数 snake_case（如 `camera`, `alarm_record`, `user`） | 统一单数形式 |
| **主键** | 字段名统一为 `id` | 事件表使用全局唯一 `event_id: TEXT`（天然幂等）；配置类使用整型自增 |
| **时间戳铁律** | **13 位 UTC Unix 毫秒 `INTEGER` (`i64`)** | 严禁存储字符串、日期对象或秒级时间戳，后端只存 UTC |
| **相对图片路径** | `TEXT` 相对路径（如 `var/images/...`） | **严禁存绝对路径**，确保数据目录整体迁移或挂载时不损坏引用 |
| **布尔类型** | `INTEGER` 0 或 1 | SQLite 无原生布尔类型 |
| **严禁大对象** | **严禁在数据库中存储 BLOB 图片或视频** | 抓拍图片与视频切片一律存文件系统，数据库仅持有相对路径指针 |

---

## 4. 批量写入与级联淘汰保护

- **批量插入契约**：告警事件等高频写入必须通过通道缓冲并**攒批批量提交**（如 32 条或 1 秒到期），严禁每产生一个事件启动一次单事务写盘；
- **有界缓冲防御**：事件写入通道必须设固定上限，满载时主动丢弃最旧数据并记录告警，严禁无界内存累积；
- **原子级联淘汰（图在案在，图销案销）**：
  - 存储清理任务在删除数据库历史记录时，**必须在单一事务中同步销毁物理磁盘上的图片/录像文件**，彻底杜绝孤儿文件；
  - 配合 `auto_vacuum = INCREMENTAL`，定期执行 `PRAGMA incremental_vacuum` 释放被删除的页面空间，防止数据库文件只增不减。

---

## 5. 查询与分层边界

- **Repository 封装**：所有 SQL 查询必须收敛在 `crates/db/src/repository/` 中，**`api` 层与 `pipeline` 层严禁直接依赖 `sea-orm` 查询 DSL**；
- **有界查询铁律**：列表查询**一律强制带 `limit` 限制**，严禁发起无界全表扫描；
- **高频复合索引**：时间范围查询的字段必须建立联合索引（如 `CREATE INDEX idx_alarm_cam_ts ON alarm_record (camera_id, timestamp)`）。

---

## 6. 禁止事项 (Iron Rules)

- ❌ 在 `api` 或 `pipeline` 层直接编写 `sea-orm` 查询代码
- ❌ 修改已合并入库的已发布 migration SQL 文件
- ❌ 在数据库中将时间戳存为字符串或秒级数字
- ❌ 在数据库中用 `BLOB` 存储抓拍原图或视频切片
- ❌ 忘记开启 `journal_mode = WAL` 导致读写锁死
- ❌ 在高频流式数据上执行无 `limit` 的全表 `select`
