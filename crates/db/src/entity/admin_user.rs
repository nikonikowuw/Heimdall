use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "admin_users")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    #[sea_orm(unique, column_type = "Text")]
    pub username: String,
    #[sea_orm(column_type = "Text")]
    pub password_hash: String,
    pub token_invalid_before: i64,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

impl From<Model> for types::AdminUser {
    fn from(model: Model) -> Self {
        Self {
            id: model.id,
            username: model.username,
            password_hash: model.password_hash,
            token_invalid_before: model.token_invalid_before,
            created_at: model.created_at.timestamp_millis(),
            updated_at: model.updated_at.timestamp_millis(),
        }
    }
}
