use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};

use crate::error::DbError;

/// 创建核心数据表（如不存在）及执行轻量级增量字段迁移
pub async fn create_tables_if_not_exist(db: &DatabaseConnection) -> Result<(), DbError> {
    let ddl_statements = [
        r#"
        CREATE TABLE IF NOT EXISTS cameras (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            camera_id TEXT NOT NULL UNIQUE,
            name TEXT NOT NULL,
            protocol TEXT NOT NULL DEFAULT 'rtsp',
            rtsp_url TEXT NOT NULL,
            sub_rtsp_url TEXT NOT NULL DEFAULT '',
            remark TEXT NOT NULL DEFAULT '',
            last_probe_status TEXT NOT NULL DEFAULT 'never',
            last_probe_at DATETIME,
            last_probe_error_code TEXT NOT NULL DEFAULT '',
            last_success_at DATETIME,
            last_codec TEXT NOT NULL DEFAULT '',
            last_width INTEGER NOT NULL DEFAULT 0,
            last_height INTEGER NOT NULL DEFAULT 0,
            last_fps REAL NOT NULL DEFAULT 0.0,
            gb28181_device_id TEXT,
            gb28181_channel_id TEXT,
            created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        "#,
        r#"
        CREATE TABLE IF NOT EXISTS analysis_tasks (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            camera_id TEXT NOT NULL UNIQUE,
            name TEXT NOT NULL,
            desired_enabled INTEGER NOT NULL DEFAULT 0,
            actual_status INTEGER NOT NULL DEFAULT 0,
            rules_json TEXT NOT NULL DEFAULT '[]',
            motion_gate_json TEXT NOT NULL DEFAULT '{}',
            created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY(camera_id) REFERENCES cameras(camera_id) ON DELETE CASCADE
        );
        "#,
        r#"
        CREATE TABLE IF NOT EXISTS alarm_records (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_id TEXT NOT NULL UNIQUE,
            camera_id TEXT NOT NULL,
            alarm_type_id TEXT NOT NULL,
            occurred_at DATETIME NOT NULL,
            target_label TEXT NOT NULL,
            confidence REAL NOT NULL DEFAULT 0.0,
            track_id INTEGER NOT NULL DEFAULT 0,
            bbox_json TEXT NOT NULL DEFAULT '[]',
            image_id TEXT NOT NULL DEFAULT '',
            image_rel_path TEXT NOT NULL DEFAULT '',
            created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        "#,
        r#"
        CREATE INDEX IF NOT EXISTS idx_alarm_records_camera_time ON alarm_records(camera_id, occurred_at DESC);
        "#,
        r#"
        CREATE TABLE IF NOT EXISTS operation_logs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            username TEXT NOT NULL,
            module TEXT NOT NULL,
            action TEXT NOT NULL,
            method TEXT NOT NULL,
            path TEXT NOT NULL,
            query TEXT NOT NULL DEFAULT '',
            body TEXT NOT NULL DEFAULT '',
            status_code INTEGER NOT NULL,
            duration_ms INTEGER NOT NULL,
            ip TEXT NOT NULL,
            user_agent TEXT NOT NULL,
            created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        "#,
        r#"
        CREATE INDEX IF NOT EXISTS idx_operation_logs_module_time ON operation_logs(module, created_at DESC);
        "#,
        r#"
        CREATE TABLE IF NOT EXISTS admin_users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            username TEXT NOT NULL UNIQUE,
            password_hash TEXT NOT NULL,
            token_invalid_before INTEGER NOT NULL DEFAULT 0,
            created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
            updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        "#,
        r#"
        CREATE TABLE IF NOT EXISTS system_configs (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
        "#,
    ];

    for ddl in ddl_statements {
        db.execute(Statement::from_string(
            sea_orm::DatabaseBackend::Sqlite,
            ddl.to_string(),
        ))
        .await?;
    }

    // 增量字段无损迁移（针对已有历史数据库文件自动对齐字段）
    let migrations = [
        (
            "cameras",
            "last_probe_error_code",
            "TEXT NOT NULL DEFAULT ''",
        ),
        ("cameras", "last_success_at", "DATETIME"),
        ("cameras", "gb28181_device_id", "TEXT"),
        ("cameras", "gb28181_channel_id", "TEXT"),
    ];

    for (table, col, def) in migrations {
        let sql = format!("ALTER TABLE {table} ADD COLUMN {col} {def};");
        let _ = db
            .execute(Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                sql,
            ))
            .await;
    }

    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_tables_and_incremental_migration() {
        let db = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
        create_tables_if_not_exist(&db).await.unwrap();

        // 再次执行应幂等安全
        create_tables_if_not_exist(&db).await.unwrap();
    }
}
