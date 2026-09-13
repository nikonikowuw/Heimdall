use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "sys_gb28181_config")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: i64,
    pub sip_id: String,
    pub sip_domain: String,
    pub sip_port: i32,
    pub sip_password: String,
    pub rtp_port_range_start: i32,
    pub rtp_port_range_end: i32,
    pub auto_catalog_sync: i32,
    pub heartbeat_timeout_sec: i32,
    pub updated_at_ms: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

impl From<Model> for types::SysGb28181Config {
    fn from(m: Model) -> Self {
        Self {
            sip_id: m.sip_id,
            sip_domain: m.sip_domain,
            sip_port: m.sip_port as u16,
            sip_password: m.sip_password,
            rtp_port_range_start: m.rtp_port_range_start as u16,
            rtp_port_range_end: m.rtp_port_range_end as u16,
            auto_catalog_sync: m.auto_catalog_sync != 0,
            heartbeat_timeout_sec: m.heartbeat_timeout_sec as u32,
            updated_at_ms: m.updated_at_ms,
        }
    }
}
