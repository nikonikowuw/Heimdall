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

    // 迁移完成后立即补齐查询计划统计信息。
    //
    // 必须放在迁移之后：全新库在建表前执行 optimize 无事可做，统计信息就不会生成。
    // 此处用 rusqlite 独立连接，不复用 init_db 的 sea-orm 连接池，故 pragma 必须两处各一份。
    // bitmask 见 `connection.rs::init_db`；0x10 位必须保留，否则冷启动会无界 ANALYZE。
    conn.execute_batch("PRAGMA optimize=0x10012;")
        .map_err(|e| DbError::Migration(format!("引导 SQLite 统计信息失败: {e}")))?;

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
        let db_file = temp_dir.join("test_heimdall_migration.db");
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

    /// AC1 / R1.1 + R1.2：全新库必须经 `init_db` 后已引导查询计划统计信息。
    ///
    /// 走的是**真实链路**：`run_migrations` 建表 → 写入数据 → `init_db` 的 pragma 序列。
    /// 断言不能只看表存在：全新空库里 `sys_gb28181_config`（无索引表）
    /// 就能让 `sqlite_stat1` 出现，但业务表一条统计也没有。
    /// 因此这里**只断言有数据时的业务表确实获得统计条目**。
    #[tokio::test]
    async fn test_init_db_bootstraps_query_planner_statistics() {
        let temp_dir = std::env::temp_dir();
        let db_file = temp_dir.join("test_heimdall_stats_bootstrap.db");
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", db_file.display()));
        }

        run_migrations(&db_file).unwrap();

        // 写入 200 行，让 alarm_records 的索引有值得统计的量。
        // 迁移后的空库：`sqlite_stat1` 可能已由无索引表（`sys_gb28181_config`）触发创建，
        // 但业务表一条统计也没有——这正是不能只断言「表存在」的原因。
        {
            let conn = rusqlite::Connection::open(&db_file).unwrap();
            let alarm_stats_before: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_stat1 WHERE tbl = 'alarm_records';",
                    [],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            assert_eq!(
                alarm_stats_before, 0,
                "空库阶段 alarm_records 不应有统计条目（无数据可统计）"
            );

            conn.execute_batch(
                r#"
                BEGIN;
                WITH RECURSIVE seq(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM seq WHERE i < 200)
                INSERT INTO alarm_records (event_id, camera_id, alarm_type_id, occurred_at, target_label)
                SELECT 'EVT-' || i, 'cam_01', 'intrusion', '2026-10-02T10:00:00+00:00', 'person' FROM seq;
                COMMIT;
                "#,
            )
            .unwrap();
        }

        // 真实链路：init_db 执行含 optimize 的 pragma 序列。
        let db = crate::connection::init_db(db_file.to_str().unwrap())
            .await
            .expect("init_db");
        drop(db);

        let conn = rusqlite::Connection::open(&db_file).unwrap();
        let stat1_rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM sqlite_stat1;", [], |row| row.get(0))
            .unwrap();
        assert!(
            stat1_rows > 0,
            "AC1：init_db 后 sqlite_stat1 必须非空，实际 {stat1_rows} 行"
        );

        let alarm_stats: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_stat1 WHERE tbl = 'alarm_records';",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            alarm_stats > 0,
            "AC1：有数据的 alarm_records 必须获得统计条目，否则计划仍可能退化；\
             实际 {alarm_stats} 行"
        );

        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", db_file.display()));
        }
    }

    /// AC5：迁移必须登记到 `refinery_schema_history`，且重复执行幂等。
    #[test]
    fn test_refinery_records_all_versions_and_is_idempotent() {
        let temp_dir = std::env::temp_dir();
        let db_file = temp_dir.join("test_heimdall_refinery_history.db");
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", db_file.display()));
        }

        run_migrations(&db_file).unwrap();

        {
            let conn = rusqlite::Connection::open(&db_file).unwrap();
            let max_version: i64 = conn
                .query_row(
                    "SELECT MAX(version) FROM refinery_schema_history;",
                    [],
                    |row| row.get(0),
                )
                .unwrap();

            // 嵌入式迁移的最后一个版本
            let expected = migrations::runner()
                .get_migrations()
                .iter()
                .map(|m| m.version() as i64)
                .max()
                .expect("at least one migration");
            assert_eq!(
                max_version, expected,
                "refinery 应登记到最新版本 {expected}"
            );

            // V23 / V24 必须都在历史表中
            for version in [23i64, 24] {
                let found: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM refinery_schema_history WHERE version = ?1;",
                        [version],
                        |row| row.get(0),
                    )
                    .unwrap();
                assert_eq!(found, 1, "refinery_schema_history 缺少 V{version}");
            }
        }

        // 幂等：再跑一次不报错、版本数不变
        run_migrations(&db_file).unwrap();
        {
            let conn = rusqlite::Connection::open(&db_file).unwrap();
            let total: i64 = conn
                .query_row("SELECT COUNT(*) FROM refinery_schema_history;", [], |row| {
                    row.get(0)
                })
                .unwrap();
            let distinct: i64 = conn
                .query_row(
                    "SELECT COUNT(DISTINCT version) FROM refinery_schema_history;",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(total, distinct, "重复迁移不得重复登记版本");
        }

        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", db_file.display()));
        }
    }
}
