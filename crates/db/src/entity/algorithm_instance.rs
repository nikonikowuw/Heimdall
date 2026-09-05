use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "algorithm_instances")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    #[sea_orm(unique, column_type = "Text")]
    pub instance_id: String,
    pub camera_id: String,
    pub algorithm_id: String,
    pub analysis_fps: i32,
    #[sea_orm(column_type = "Text")]
    pub params_json: String,
    #[sea_orm(column_type = "Text")]
    pub rules_json: String,
    #[sea_orm(column_type = "Text")]
    pub motion_gate_json: String,
    pub enabled: bool,
    pub actual_status: i32,
    pub status_message: String,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
