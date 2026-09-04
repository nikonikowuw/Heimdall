# 数据库规范

> SeaORM + SQLite。核心原则：**SQLite 跑在 eMMC/SD 卡上，写入策略比查询优化更要紧。**

> ⚠️ **状态：立项约定（尚未经代码验证）**
> 首批 entity 与 migration 落地后需回填真实表结构与查询示例，并删除本提示。

---

## 为什么是 SQLite

Argus 的数据是单设备本地数据：事件元数据、摄像头配置、录像片段索引。没有多写入者、没有跨节点。SQLite 免运维、零额外进程、内存占用可忽略 —— 这在边缘设备上是决定性的。

**代价与应对**：写并发受限（全库级写锁）。因此必须开 WAL，且写入必须批量化。

---

## 连接配置

在 `db` 中统一建立连接池，其它 crate 不自己开连接：

```rust
// crates/db/src/lib.rs
let mut opt = ConnectOptions::new(format!("sqlite://{}?mode=rwc", path.display()));
opt.max_connections(4)          // SQLite 写是串行的，池子大了没用
   .min_connections(1)
   .sqlx_logging(false);        // 帧路径附近的 SQL 日志会刷屏
let db = Database::connect(opt).await?;
```

启动后必须执行的 PRAGMA：

| PRAGMA | 值 | 理由 |
|--------|-----|------|
| `journal_mode` | `WAL` | 读写不互斥，事件写入不阻塞 API 查询 |
| `synchronous` | `NORMAL` | WAL 模式下足够安全，避免每次提交都 fsync 磨损存储 |
| `busy_timeout` | `5000` | 写锁竞争时等待而非立即报错 |
| `foreign_keys` | `ON` | SQLite 默认关闭，必须显式打开 |

**规则**：这些 PRAGMA 写在连接初始化代码里，不依赖外部配置。漏掉 WAL 是 SQLite 项目最常见的性能事故。

---

## Entity 与 Migration

使用 **Refinery** 管理数据库迁移。Refinery 是 Rust 原生的数据库迁移工具，支持 SQLite，使用单向前向 SQL 迁移文件（`V{version}__{description}.sql`）。

```
crates/db/
├── src/
│   ├── lib.rs
│   ├── entity/          # SeaORM entity，一张表一个文件
│   │   ├── mod.rs
│   │   ├── camera.rs            # 摄像头设备
│   │   ├── task.rs              # 分析任务（关联摄像头与算法配置）
│   │   ├── algorithm.rs         # 已安装的算法包元数据
│   │   ├── alarm_record.rs      # 告警事件记录
│   │   ├── capture.rs           # 通用目标抓拍记录
│   │   ├── face_observation.rs  # 人脸识别通行观测
│   │   ├── plate_observation.rs # 车牌识别通行观测
│   │   ├── user.rs              # 单管理员凭据（admin，存储密码哈希）
│   │   ├── system_config.rs     # 系统配置（网络、对外 API Key、抓拍存储配额）
│   │   └── operation_log.rs     # 关键操作日志（重启、改密、升级）
│   ├── repository/      # 查询与持久化函数，按领域分文件
│   │   ├── mod.rs
│   │   ├── camera.rs
│   │   ├── alarm.rs
│   │   └── system.rs
│   └── migration/
│       ├── mod.rs
│       ├── migrations/          # Refinery 单向前向 SQL 迁移文件
│       │   ├── V1__init_schema.sql
│       │   └── V2__add_event_score.sql
│       └── seed.rs             # 初始数据播种
```

### 迁移文件规范（Refinery）

Refinery 遵循 `V{version}__{description}.sql` 命名规范（单向前向演进，回滚通过编写新的前向迁移实现）：

```
V1__init_schema.sql           # 初始全量 schema
V2__add_event_score.sql       # 增量字段变更
```

- **版本号**：整数，严格单调递增（`V1`、`V2`、...），双下划线 `__` 分隔版本号和描述
- **描述**：snake_case 英文动词短语，描述本次变更意图
- **事务与 DDL**：SQLite 在事务内执行 DDL，变更具备原子性；若需要回滚，编写新的前向迁移（如 `V3__revert_event_score.sql`）

示例：

```sql
-- V1__init_schema.sql
CREATE TABLE IF NOT EXISTS camera (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    rtsp_url TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,  -- UTC Unix 毫秒时间戳 (i64)
    updated_at INTEGER NOT NULL   -- UTC Unix 毫秒时间戳 (i64)
);

CREATE TABLE IF NOT EXISTS alarm_record (
    event_id TEXT PRIMARY KEY,
    camera_id TEXT NOT NULL REFERENCES camera(id),
    alarm_type_id TEXT NOT NULL,
    timestamp INTEGER NOT NULL,   -- UTC Unix 毫秒时间戳 (i64)
    confidence REAL,
    image_rel_path TEXT,
    created_at INTEGER NOT NULL   -- UTC Unix 毫秒时间戳 (i64)
);

CREATE INDEX IF NOT EXISTS idx_alarm_camera_ts ON alarm_record (camera_id, timestamp);
```

### 迁移执行规则

| 规则 | 说明 |
|------|------|
| **只增不改（前向演进）** | 已合并的迁移文件永不修改，需要变更或回滚均添加新的递增版本号文件 |
| **启动时自动执行** | 应用启动时按版本号递增自动执行未运行的迁移（Refinery 自动追踪） |
| **版本追踪** | Refinery 自动在 SQLite 中维护 `refinery_schema_history` 表 |
| **禁止运行时手动执行** | 不提供未经受控的运行时手动 DDL 执行，统一由启动流程托管 |

### 集成方式

在应用启动时，优先使用轻量 `rusqlite::Connection` 执行嵌入式迁移，完成后交由 SeaORM 连接池接管：

```rust
// crates/db/src/migration/mod.rs
use refinery::embed_migrations;
use std::path::Path;

embed_migrations!("src/migration/migrations");

pub fn run_migrations(db_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut conn = rusqlite::Connection::open(db_path)?;
    // 执行嵌入的迁移
    migrations::runner().run(&mut conn)?;
    Ok(())
}
```

- 迁移文件通过 `embed_migrations!` 宏编译进二进制，无需运行时读取外部文件系统
- 启动时在初始化 SeaORM 连接池前执行 `run_migrations`
- 若迁移失败，应用立即 panic/退出并输出错误日志，严禁在损坏或不一致的 schema 上运行

### Seed 播种

初始数据通过独立的 `seed.rs` 模块在首次启动时播种：

- 检查 `user` 表是否为空，为空则创建默认管理员（`admin`），密码哈希后存储
- 检查 `system_config` 表是否为空，为空则插入默认配置
- Seed 逻辑**不走 migration 文件**，而是应用层逻辑（因为 Seed 可能依赖业务代码）

---

## 表设计约定

| 约定 | 规则 | 说明 |
|------|------|------|
| 表名 | 单数 snake_case：`camera`、`alarm_record`、`user` | 避免复数混乱 |
| 主键 | 一律 `id` | 事件类用 `TEXT` 存引擎生成的唯一 `event_id`（天然幂等去重）；配置类用 `INTEGER` 自增 |
| 时间戳 | 存 **UTC Unix 毫秒 `INTEGER`** | 后端只存 UTC，不存储时区信息。SQLite 无原生时间类型，以毫秒整数排序、比较最快，前端按用户时区显示 |
| 相对图片路径 | 存 **相对路径 `TEXT`** | 统一存相对于数据根目录的路径（如 `var/images/...`），**严禁存绝对路径**，便于数据目录迁移 |
| 布尔 | `INTEGER` 0/1 | SQLite 无 BOOL |
| 外键 | `<表名>_id`，例如 `camera_id` | 逻辑外键建立复合索引 |
| 可空 | 默认 NOT NULL | 只有语义上真可能为空才写 Option / NULL |

**时间戳规则是硬性的**：整个系统内部时间统一为 Unix 毫秒整数，只在 API 边界和 UI 上转成人类可读格式。混用会导致排序和范围查询出错。

---

## 写入必须批量化

事件写入在帧路径附近，一条一个事务会把 eMMC 写穿：

```rust
// ❌ 每个事件一个事务
for ev in events { ev.into_active_model().insert(db).await?; }

// ✅ 批量插入，一个事务
Event::insert_many(events.into_iter().map(|e| e.into_active_model()))
    .exec(db)
    .await?;
```

约定：

- 事件写入走**缓冲 + 定时/定量刷盘**（例如攒满 32 条或 1 秒到期就提交）。
- 缓冲区**必须有上界**，满了要丢弃最旧的并计数告警，不能无限增长。
- 快照/录像等大文件写文件系统，数据库里只存路径，**绝不把二进制塞进 BLOB**。

---

## 保留策略是必需功能，不是可选项

设备存储有限，没有清理逻辑就是几天后必然写满：

- 事件表按天数或条数上限自动清理，清理任务定期运行。
- 删除数据库记录时**必须同时删除对应的快照/录像文件**，否则文件系统会残留孤儿文件。
- 清理后周期性执行 `PRAGMA incremental_vacuum`（配合 `auto_vacuum = INCREMENTAL`），否则数据库文件只增不减。

**规则**：任何新增的"会持续增长的表"，在同一个 PR 里必须带上它的保留策略。

---

## 查询规范

- 查询函数集中在 `repository/`，**handler 里不写 SeaORM 查询链**。
- 列表接口一律分页，**禁止无 `limit` 的全表查询**。
- 时间范围查询的字段必须有索引（事件表按 `(camera_id, ts)` 建复合索引）。
- 用 SeaORM 的类型化查询，避免 `raw_sql`；确需 raw SQL 时写在 repository 里并附注释说明为什么。

---

## 禁止事项

- ❌ 在 `api` 或 `pipeline` 里直接依赖 `sea-orm`
- ❌ 修改已合并的 migration 文件
- ❌ 时间戳存字符串
- ❌ 二进制数据存 BLOB
- ❌ 无上界的写入缓冲
- ❌ 忘记开 WAL

---

## 待验证事项

- [ ] 事件表预计写入速率，据此定刷盘批量与间隔
- [ ] 是否需要单独的时序表存运动检测原始数据，还是只存判定后的事件
- [ ] 保留策略的触发方式：定时任务 vs 每次写入后检查
