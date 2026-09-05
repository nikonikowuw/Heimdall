use std::path::Path;

use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};

use crate::error::DbError;

refinery::embed_migrations!("src/migration/migrations");

/// 执行嵌入式版本化数据库迁移（基于 Refinery）
pub fn run_migrations(db_path: &Path) -> Result<(), DbError> {
    if let Some(parent) = db_path.parent() {
        if !parent.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(parent);
        }
    }

    let mut conn = rusqlite::Connection::open(db_path)
        .map_err(|e| DbError::Migration(format!("打开 SQLite 数据库文件失败: {e}")))?;

    // 必开 PRAGMA 配置
    conn.execute_batch(
        r#"
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;
        PRAGMA busy_timeout = 5000;
        PRAGMA foreign_keys = ON;
        "#,
    )
    .map_err(|e| DbError::Migration(format!("配置 SQLite PRAGMA 失败: {e}")))?;

    migrations::runner()
        .run(&mut conn)
        .map_err(|e| DbError::Migration(format!("执行 Refinery 版本迁移失败: {e}")))?;

    tracing::info!(db_path = %db_path.display(), "Refinery 版本化数据库迁移已成功执行");
    Ok(())
}

/// 重置并从头执行版本化迁移（清理旧库及 WAL/SHM 临时文件后重建）
pub fn reset_database(db_path: &Path) -> Result<(), DbError> {
    if db_path.exists() {
        let _ = std::fs::remove_file(db_path);
    }
    let path_str = db_path.to_string_lossy();
    let wal_path = std::path::PathBuf::from(format!("{path_str}-wal"));
    let shm_path = std::path::PathBuf::from(format!("{path_str}-shm"));
    if wal_path.exists() {
        let _ = std::fs::remove_file(&wal_path);
    }
    if shm_path.exists() {
        let _ = std::fs::remove_file(&shm_path);
    }

    run_migrations(db_path)
}

/// 在已有 SeaORM 连接（如内存数据库 :memory:）上按顺序应用所有嵌入式迁移
pub async fn run_migrations_on_seaorm(db: &DatabaseConnection) -> Result<(), DbError> {
    // 确保 refinery_schema_history 迁移记录表存在
    db.execute(Statement::from_string(
        sea_orm::DatabaseBackend::Sqlite,
        r#"
        CREATE TABLE IF NOT EXISTS refinery_schema_history (
            version INTEGER PRIMARY KEY,
            name TEXT,
            applied_on TEXT,
            checksum TEXT
        );
        "#
        .to_string(),
    ))
    .await?;

    let runner = migrations::runner();
    let mut migrations = runner.get_migrations().to_vec();
    migrations.sort_by_key(|m| m.version());

    for migration in migrations {
        let version = migration.version() as i64;
        let check_stmt = Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            format!("SELECT COUNT(*) FROM refinery_schema_history WHERE version = {version};"),
        );
        let query_res = db.query_one(check_stmt).await?;
        let count: i64 = query_res
            .and_then(|row| row.try_get_by_index(0).ok())
            .unwrap_or(0);

        if count > 0 {
            continue; // 已应用过该版本，幂等跳过
        }

        if let Some(sql) = migration.sql() {
            // 将批处理按分号拆分为单个 SQL 语句依次执行，任何执行错误立即 fail-fast 返回
            for stmt in sql.split(';') {
                let trimmed = stmt.trim();
                if !trimmed.is_empty() {
                    db.execute(Statement::from_string(
                        sea_orm::DatabaseBackend::Sqlite,
                        trimmed.to_string(),
                    ))
                    .await?;
                }
            }
        }

        let name = migration.name();
        let record_stmt = Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            format!(
                "INSERT INTO refinery_schema_history (version, name, applied_on, checksum) VALUES ({version}, '{name}', CURRENT_TIMESTAMP, '');"
            ),
        );
        db.execute(record_stmt).await?;
    }
    Ok(())
}

/// 初始化纯净的内存测试数据库并自动跑完所有版本迁移
pub async fn init_test_db() -> Result<DatabaseConnection, DbError> {
    let db = crate::connection::init_db(":memory:").await?;
    run_migrations_on_seaorm(&db).await?;
    Ok(db)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_refinery_migrations_on_file() {
        let temp_dir = std::env::temp_dir();
        let db_file = temp_dir.join("test_argus_migration.db");
        if db_file.exists() {
            let _ = std::fs::remove_file(&db_file);
        }

        run_migrations(&db_file).unwrap();

        // 验证表是否存在
        let conn = rusqlite::Connection::open(&db_file).unwrap();
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap();
        let tables: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        assert!(tables.contains(&"admin_users".to_string()));
        assert!(tables.contains(&"system_configs".to_string()));
        assert!(tables.contains(&"cameras".to_string()));
        assert!(tables.contains(&"refinery_schema_history".to_string()));

        let _ = std::fs::remove_file(&db_file);
    }

    #[tokio::test]
    async fn test_init_test_db_lifecycle() {
        let db = init_test_db().await.unwrap();
        // 再次跑一次迁移应安全
        run_migrations_on_seaorm(&db).await.unwrap();
    }
}
