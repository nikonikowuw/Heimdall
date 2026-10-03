use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection, Statement};

use crate::error::DbError;

/// 初始化 SQLite 数据库连接并配置高并发低延迟 WAL PRAGMA
pub async fn init_db(db_path: &str) -> Result<DatabaseConnection, DbError> {
    let url = if db_path == ":memory:" || db_path == "sqlite::memory:" {
        "sqlite::memory:".to_string()
    } else if db_path.starts_with("sqlite:") {
        db_path.to_string()
    } else {
        format!("sqlite://{db_path}?mode=rwc")
    };
    let mut opt = ConnectOptions::new(&url);
    opt.max_connections(4)
        .min_connections(1)
        .sqlx_logging(false);

    let db = Database::connect(opt)
        .await
        .map_err(|source| DbError::Connection { url, source })?;

    // 必开 PRAGMA 配置：WAL 模式、NORMAL 同步、busy_timeout、外键约束、查询计划统计信息
    //
    // PRAGMA optimize=0x10012 的 bitmask 语义（bundled SQLite 3.46.0）：
    //   0x10000 —— 让本连接从未查询过的表也进入检查。并非无条件必需：表上存在
    //              缺 stat1 的索引时（条件 4b）已会被分析；不可替代的是无索引表；
    //   0x10    —— 以有界的 analysis_limit 执行 ANALYZE（内部取 SQLITE_DEFAULT_OPTIMIZE_LIMIT
    //              = 2000 行），把首次引导的耗时限制在百毫秒级；
    //   0x2     —— 对可能受益的表运行 ANALYZE（默认位）。
    //
    // 0x10 位不可省：缺它时 `nLimit = 0`，ANALYZE 不受行数限制，冷启动会完整扫描所有
    // 索引（实测 50 万告警 + 20 万抓拍 + 20 万识别：0x10002 = 302ms，0x10012 = 40ms）。
    // 注意 0x2 只是「运行 ANALYZE」，与 skip-scan 无关：跳扫由优化器按 sqlite_stat1
    // 自行决策，没有任何 bit 用于开关它。
    //
    // 缺统计信息时，复合索引首列未被约束的查询无法走跳扫（跳扫硬性要求索引已有 sqlite_stat1），
    // 会退化为全表扫：
    // alarm_records 仅有的 (camera_id, occurred_at DESC) 在“全部通道 + 时间窗”形态下
    // 正是这种情况（实测 200 万行 209ms -> 2ms）。
    //
    // 详见 [数据库规范](../../../.trellis/spec/db/backend/database-guidelines.md#连接与迁移)。
    let pragmas = [
        "PRAGMA journal_mode = WAL;",
        "PRAGMA synchronous = NORMAL;",
        "PRAGMA busy_timeout = 5000;",
        "PRAGMA foreign_keys = ON;",
        "PRAGMA optimize=0x10012;",
    ];

    for pragma in pragmas {
        db.execute(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            pragma.to_string(),
        ))
        .await?;
    }

    Ok(db)
}
