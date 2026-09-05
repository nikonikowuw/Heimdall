use sea_orm::DatabaseConnection;

use crate::error::DbError;

/// 创建核心数据表并应用所有版本化数据库迁移（统一由 Refinery 单一真实源驱动）
pub async fn create_tables_if_not_exist(db: &DatabaseConnection) -> Result<(), DbError> {
    crate::migration::run_migrations_on_seaorm(db).await
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
