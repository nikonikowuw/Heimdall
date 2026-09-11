use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "cameras")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    #[sea_orm(unique, column_type = "Text")]
    pub camera_id: String,
    pub name: String,
    pub protocol: String,
    #[sea_orm(column_type = "Text")]
    pub rtsp_url: String,
    #[sea_orm(column_type = "Text")]
    pub sub_rtsp_url: String,
    #[sea_orm(column_type = "Text")]
    pub stream_mode: String,
    pub remark: String,
    pub last_probe_status: String,
    pub last_probe_at: Option<DateTimeUtc>,
    pub last_probe_error_code: String,
    pub last_success_at: Option<DateTimeUtc>,
    pub last_codec: String,
    pub last_width: i32,
    pub last_height: i32,
    pub last_fps: f64,
    pub gb28181_device_id: Option<String>,
    pub gb28181_channel_id: Option<String>,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
