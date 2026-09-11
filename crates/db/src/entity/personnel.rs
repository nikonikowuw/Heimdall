use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "personnel")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    #[sea_orm(unique, column_type = "Text")]
    pub subject_id: String,
    pub name: String,
    pub id_card: String,
    pub remark: String,
    pub primary_photo_path: String,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::gallery_face::Entity")]
    GalleryFaces,
}

impl Related<super::gallery_face::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::GalleryFaces.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
