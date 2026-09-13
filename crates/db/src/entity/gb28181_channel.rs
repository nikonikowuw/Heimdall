use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "gb28181_channels")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub device_id: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub channel_id: String,
    pub name: String,
    pub manufacturer: String,
    pub model: String,
    pub status: String,
    pub parent_id: String,
    pub sub_stream_supported: i32,
    pub last_seen_ms: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::gb28181_device::Entity",
        from = "Column::DeviceId",
        to = "super::gb28181_device::Column::DeviceId"
    )]
    Device,
}

impl Related<super::gb28181_device::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Device.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
