use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "recognition_records")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    #[sea_orm(unique, column_type = "Text")]
    pub recognition_id: String,
    #[sea_orm(column_type = "Text")]
    pub camera_id: String,
    pub gallery_id: String,
    pub subject_id: String,
    pub subject_name: String,
    pub similarity: f32,
    pub field_crop_path: String,
    pub registered_photo_path: String,
    pub recognized_at: DateTimeUtc,
    pub created_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
