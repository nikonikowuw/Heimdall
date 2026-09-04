use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    Set,
};

use crate::entity::admin_user::{ActiveModel, Column, Entity, Model};
use crate::error::DbError;

#[derive(Debug)]
pub struct AdminUserRepo;

impl AdminUserRepo {
    /// 统计系统中已有的管理员数量
    pub async fn count(db: &DatabaseConnection) -> Result<u64, DbError> {
        Entity::find().count(db).await.map_err(DbError::from)
    }

    /// 查询系统是否已经初始化了管理员
    pub async fn is_initialized(db: &DatabaseConnection) -> Result<bool, DbError> {
        let count = Self::count(db).await?;
        Ok(count > 0)
    }

    /// 根据用户名精确查询管理员信息
    pub async fn find_by_username(
        db: &DatabaseConnection,
        username: &str,
    ) -> Result<Option<Model>, DbError> {
        Entity::find()
            .filter(Column::Username.eq(username))
            .one(db)
            .await
            .map_err(DbError::from)
    }

    /// 获取系统中第一个管理员（单用户模式）
    pub async fn get_first_admin(db: &DatabaseConnection) -> Result<Option<Model>, DbError> {
        Entity::find().one(db).await.map_err(DbError::from)
    }

    /// 创建新的管理员记录
    pub async fn create_admin(
        db: &DatabaseConnection,
        username: &str,
        password_hash: &str,
    ) -> Result<Model, DbError> {
        let now = chrono::Utc::now();
        let active = ActiveModel {
            username: Set(username.to_string()),
            password_hash: Set(password_hash.to_string()),
            token_invalid_before: Set(0),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        active.insert(db).await.map_err(DbError::from)
    }

    /// 环境变量静默初始化（仅在表中尚无任何管理员时执行）
    /// 返回 Ok(true) 表示新建成功，Ok(false) 表示已存在管理员跳过初始化
    pub async fn ensure_silent_admin(
        db: &DatabaseConnection,
        username: &str,
        password_hash: &str,
    ) -> Result<bool, DbError> {
        if Self::is_initialized(db).await? {
            return Ok(false);
        }
        Self::create_admin(db, username, password_hash).await?;
        Ok(true)
    }

    /// 内部通用凭证与失效时间戳更新函数
    async fn update_credentials_internal(
        db: &DatabaseConnection,
        username: &str,
        new_password_hash: Option<&str>,
        token_invalid_before: i64,
    ) -> Result<(), DbError> {
        if let Some(user) = Self::find_by_username(db, username).await? {
            let mut active: ActiveModel = user.into();
            if let Some(hash) = new_password_hash {
                active.password_hash = Set(hash.to_string());
            }
            active.token_invalid_before = Set(token_invalid_before);
            active.updated_at = Set(chrono::Utc::now());
            active.update(db).await?;
        }
        Ok(())
    }

    /// 更新管理员登录密码并使此前签发的所有 Token 立即失效
    pub async fn update_password(
        db: &DatabaseConnection,
        username: &str,
        new_password_hash: &str,
        now_ms: i64,
    ) -> Result<(), DbError> {
        Self::update_credentials_internal(db, username, Some(new_password_hash), now_ms).await
    }

    /// 废除当前管理员此前签发的所有 Token
    pub async fn invalidate_tokens(
        db: &DatabaseConnection,
        username: &str,
        now_ms: i64,
    ) -> Result<(), DbError> {
        Self::update_credentials_internal(db, username, None, now_ms).await
    }

    /// 获取管理员 token_invalid_before 时间戳
    pub async fn get_token_invalid_before(
        db: &DatabaseConnection,
        username: &str,
    ) -> Result<Option<i64>, DbError> {
        if let Some(user) = Self::find_by_username(db, username).await? {
            Ok(Some(user.token_invalid_before))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_admin_user_repository_flow() {
        let db = crate::init_test_db()
            .await
            .expect("init in-memory db failed");

        // 1. 初始状态为未初始化
        assert!(!AdminUserRepo::is_initialized(&db).await.unwrap());
        assert_eq!(AdminUserRepo::count(&db).await.unwrap(), 0);

        // 2. 第一次静默初始化成功
        let created = AdminUserRepo::ensure_silent_admin(&db, "admin", "hash123")
            .await
            .unwrap();
        assert!(created);
        assert!(AdminUserRepo::is_initialized(&db).await.unwrap());
        assert_eq!(AdminUserRepo::count(&db).await.unwrap(), 1);

        // 3. 第二次静默初始化跳过
        let created_again = AdminUserRepo::ensure_silent_admin(&db, "admin", "hash456")
            .await
            .unwrap();
        assert!(!created_again);

        // 4. 查询
        let user = AdminUserRepo::find_by_username(&db, "admin").await.unwrap();
        assert!(user.is_some());
        let u = user.unwrap();
        assert_eq!(u.username, "admin");
        assert_eq!(u.password_hash, "hash123");
        assert_eq!(u.token_invalid_before, 0);

        // 5. 更新密码与失效时间戳
        let now_ms = 1747584000000;
        AdminUserRepo::update_password(&db, "admin", "new_hash_999", now_ms)
            .await
            .unwrap();
        let u2 = AdminUserRepo::find_by_username(&db, "admin")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(u2.password_hash, "new_hash_999");
        assert_eq!(u2.token_invalid_before, now_ms);

        // 6. 登出/失效 token
        let logout_ms = 1747585000000;
        AdminUserRepo::invalidate_tokens(&db, "admin", logout_ms)
            .await
            .unwrap();
        let token_invalid = AdminUserRepo::get_token_invalid_before(&db, "admin")
            .await
            .unwrap();
        assert_eq!(token_invalid, Some(logout_ms));
    }
}
