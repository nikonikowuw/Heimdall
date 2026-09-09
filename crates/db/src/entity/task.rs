use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "analysis_tasks")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    #[sea_orm(unique, column_type = "Text")]
    pub camera_id: String,
    pub name: String,
    pub desired_enabled: bool,
    pub actual_status: i32,
    pub status_message: String,
    pub algorithm_id: String,
    pub analysis_fps: i32,
    #[sea_orm(column_type = "Text")]
    pub algo_params_json: String,
    #[sea_orm(column_type = "Text")]
    pub rules_json: String,
    #[sea_orm(column_type = "Text")]
    pub motion_gate_json: String,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
