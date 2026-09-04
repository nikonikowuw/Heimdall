use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, Set,
};

use crate::entity::oplog::{ActiveModel, Column, Entity, Model};
use crate::error::DbError;

#[derive(Debug)]
pub struct OplogRepo;

impl OplogRepo {
    pub async fn list_recent(
        db: &DatabaseConnection,
        module: Option<&str>,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<Model>, DbError> {
        let mut query = Entity::find().order_by_desc(Column::CreatedAt);
        if let Some(m) = module {
            query = query.filter(Column::Module.eq(m));
        }
        query
            .limit(limit)
            .offset(offset)
            .all(db)
            .await
            .map_err(DbError::from)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn record(
        db: &DatabaseConnection,
        username: &str,
        module: &str,
        action: &str,
        method: &str,
        path: &str,
        query: &str,
        body: &str,
        status_code: i32,
        duration_ms: i64,
        ip: &str,
        user_agent: &str,
    ) -> Result<Model, DbError> {
        let active = ActiveModel {
            id: sea_orm::ActiveValue::NotSet,
            username: Set(username.to_string()),
            module: Set(module.to_string()),
            action: Set(action.to_string()),
            method: Set(method.to_string()),
            path: Set(path.to_string()),
            query: Set(query.to_string()),
            body: Set(body.to_string()),
            status_code: Set(status_code),
            duration_ms: Set(duration_ms),
            ip: Set(ip.to_string()),
            user_agent: Set(user_agent.to_string()),
            created_at: Set(chrono::Utc::now()),
        };
        active.insert(db).await.map_err(DbError::from)
    }
}
