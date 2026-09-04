use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "operation_logs")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub username: String,
    pub module: String,
    pub action: String,
    pub method: String,
    pub path: String,
    #[sea_orm(column_type = "Text")]
    pub query: String,
    #[sea_orm(column_type = "Text")]
    pub body: String,
    pub status_code: i32,
    pub duration_ms: i64,
    pub ip: String,
    pub user_agent: String,
    pub created_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
