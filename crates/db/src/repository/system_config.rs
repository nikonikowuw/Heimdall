use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};

use crate::error::DbError;

#[derive(Debug)]
pub struct SystemConfigRepo;

impl SystemConfigRepo {
    /// 查询指定 key 的系统配置值
    pub async fn get(db: &DatabaseConnection, key: &str) -> Result<Option<String>, DbError> {
        let stmt = Statement::from_sql_and_values(
            db.get_database_backend(),
            "SELECT value FROM system_configs WHERE key = ?",
            [key.into()],
        );
        let res = db.query_one(stmt).await.map_err(DbError::from)?;
        match res {
            Some(row) => {
                let val: String = row.try_get("", "value")?;
                Ok(Some(val))
            }
            None => Ok(None),
        }
    }

    /// 设置系统配置键值
    pub async fn set(db: &DatabaseConnection, key: &str, value: &str) -> Result<(), DbError> {
        let stmt = Statement::from_sql_and_values(
            db.get_database_backend(),
            "INSERT INTO system_configs (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [key.into(), value.into()],
        );
        db.execute(stmt).await.map_err(DbError::from)?;
        Ok(())
    }

    /// 获取配置值，若不存在则调用生成函数落库并返回
    pub async fn get_or_set_with<F>(
        db: &DatabaseConnection,
        key: &str,
        default_fn: F,
    ) -> Result<String, DbError>
    where
        F: FnOnce() -> String,
    {
        if let Some(val) = Self::get(db, key).await? {
            return Ok(val);
        }
        let new_val = default_fn();
        Self::set(db, key, &new_val).await?;
        Ok(new_val)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_system_config_lifecycle() {
        let db = crate::init_test_db().await.unwrap();

        assert_eq!(SystemConfigRepo::get(&db, "test_key").await.unwrap(), None);

        let val1 = SystemConfigRepo::get_or_set_with(&db, "test_key", || "initial_val".to_string())
            .await
            .unwrap();
        assert_eq!(val1, "initial_val");

        let val2 = SystemConfigRepo::get_or_set_with(&db, "test_key", || "new_val".to_string())
            .await
            .unwrap();
        assert_eq!(val2, "initial_val");

        SystemConfigRepo::set(&db, "test_key", "updated_val")
            .await
            .unwrap();
        assert_eq!(
            SystemConfigRepo::get(&db, "test_key").await.unwrap(),
            Some("updated_val".to_string())
        );
    }
}
