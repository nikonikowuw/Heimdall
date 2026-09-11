use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "gallery_faces")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    #[sea_orm(unique, column_type = "Text")]
    pub face_id: String,
    #[sea_orm(column_type = "Text")]
    pub subject_id: String,
    pub photo_rel_path: String,
    pub aligned_rel_path: String,
    pub feature_vector: Vec<u8>,
    pub quality_score: f32,
    pub detection_score: f32,
    pub is_primary: i32,
    pub created_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::personnel::Entity",
        from = "Column::SubjectId",
        to = "super::personnel::Column::SubjectId"
    )]
    Personnel,
}

impl Related<super::personnel::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Personnel.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}

impl From<Model> for types::GalleryFaceDto {
    fn from(f: Model) -> Self {
        Self {
            id: f.id,
            face_id: f.face_id,
            subject_id: f.subject_id,
            photo_rel_path: f.photo_rel_path,
            aligned_rel_path: f.aligned_rel_path,
            quality_score: f.quality_score,
            detection_score: f.detection_score,
            is_primary: f.is_primary != 0,
            created_at: f.created_at.timestamp_millis(),
        }
    }
}
