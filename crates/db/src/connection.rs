use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection, Statement};

use crate::error::DbError;

/// 初始化 SQLite 数据库连接并配置高并发低延迟 WAL PRAGMA
pub async fn init_db(db_path: &str) -> Result<DatabaseConnection, DbError> {
    let url = format!("sqlite://{db_path}?mode=rwc");
    let mut opt = ConnectOptions::new(&url);
    opt.max_connections(4)
        .min_connections(1)
        .sqlx_logging(false);

    let db = Database::connect(opt)
        .await
        .map_err(|source| DbError::Connection { url, source })?;

    // 必开 PRAGMA 配置：WAL 模式、NORMAL 同步、busy_timeout、外键约束
    let pragmas = [
        "PRAGMA journal_mode = WAL;",
        "PRAGMA synchronous = NORMAL;",
        "PRAGMA busy_timeout = 5000;",
        "PRAGMA foreign_keys = ON;",
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
