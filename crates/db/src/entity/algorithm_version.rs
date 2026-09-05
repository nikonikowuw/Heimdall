use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "algorithm_versions")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    pub algorithm_id: String,
    pub version: String,
    pub platform_id: String,
    pub min_adapter_version: String,
    pub package_root: String,
    #[sea_orm(column_type = "Text")]
    pub fps_tiers: String,
    #[sea_orm(column_type = "Text")]
    pub config_schema: String,
    #[sea_orm(column_type = "Text")]
    pub manifest_raw: String,
    pub package_size_bytes: i64,
    pub is_active: bool,
    pub is_builtin: bool,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
