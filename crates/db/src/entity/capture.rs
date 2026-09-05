use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "capture_records")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    #[sea_orm(unique, column_type = "Text")]
    pub capture_id: String,
    #[sea_orm(column_type = "Text")]
    pub camera_id: String,
    pub track_id: i64,
    pub target_label: String,
    pub confidence: f32,
    pub quality_score: f32,
    #[sea_orm(column_type = "Text")]
    pub bbox_json: String,
    pub image_id: String,
    pub image_rel_path: String,
    pub crop_image_id: String,
    pub crop_image_rel_path: String,
    pub captured_at: DateTimeUtc,
    pub created_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
