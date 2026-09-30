use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// 录像-事件关联记录
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "recording_events")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    #[sea_orm(column_type = "Text")]
    pub recording_id: String,
    /// 事件类型：alarm / recognition
    #[sea_orm(column_type = "Text")]
    pub event_type: String,
    /// 事件 ID（引用 alarm_records.event_id 或 recognition_records.event_id）
    #[sea_orm(column_type = "Text")]
    pub event_id: String,
    /// 事件发生时间（UTC 毫秒）
    pub event_time: i64,
    /// 事件在录像文件内的时间偏移（毫秒）
    pub offset_ms: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::recording::Entity",
        from = "Column::RecordingId",
        to = "super::recording::Column::RecordingId"
    )]
    Recording,
}

impl Related<super::recording::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Recording.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
