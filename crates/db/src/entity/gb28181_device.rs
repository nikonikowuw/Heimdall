use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "gb28181_devices")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub device_id: String,
    pub name: String,
    pub ip_addr: String,
    pub sip_port: i32,
    pub transport: String,
    pub status: String,
    pub channel_count: i32,
    pub last_keepalive_ms: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::gb28181_channel::Entity")]
    Channels,
}

impl Related<super::gb28181_channel::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Channels.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
