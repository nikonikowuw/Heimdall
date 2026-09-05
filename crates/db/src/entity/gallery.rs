use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "galleries")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    #[sea_orm(column_type = "Text")]
    pub gallery_id: String,
    #[sea_orm(unique, column_type = "Text")]
    pub subject_id: String,
    pub subject_name: String,
    pub subject_type: String,
    pub id_card: String,
    pub plate_number: String,
    #[sea_orm(nullable)]
    pub feature_vector: Option<Vec<u8>>,
    pub photo_rel_path: String,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
