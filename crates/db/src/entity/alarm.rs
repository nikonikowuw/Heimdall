use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "alarm_records")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    #[sea_orm(unique, column_type = "Text")]
    pub event_id: String,
    #[sea_orm(column_type = "Text")]
    pub camera_id: String,
    pub alarm_type_id: String,
    pub occurred_at: DateTimeUtc,
    pub target_label: String,
    pub confidence: f32,
    pub track_id: i64,
    #[sea_orm(column_type = "Text")]
    pub bbox_json: String,
    pub image_id: String,
    pub image_rel_path: String,
    pub crop_image_id: String,
    pub crop_image_rel_path: String,
    pub rule_type: String,
    pub severity: String,
    pub status: String,
    pub handled_at: Option<DateTimeUtc>,
    pub created_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::camera::Entity",
        from = "Column::CameraId",
        to = "super::camera::Column::CameraId"
    )]
    Camera,
}

impl Related<super::camera::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Camera.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
