use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "operational_logs")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub ts_ms: i64,
    pub level: String,
    pub event: String,
    #[sea_orm(column_type = "Text", default_value = "")]
    pub target: String,
    #[sea_orm(column_type = "Text")]
    pub message: String,
    pub camera_id: Option<String>,
    #[sea_orm(column_type = "Text")]
    pub extra_json: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
