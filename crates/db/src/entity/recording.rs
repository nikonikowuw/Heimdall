use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// 事件录像片段
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "recordings")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    #[sea_orm(unique, column_type = "Text")]
    pub recording_id: String,
    #[sea_orm(column_type = "Text")]
    pub camera_id: String,
    #[sea_orm(column_type = "Text")]
    pub file_path: String,
    /// 录像起始时间（UTC 毫秒）
    pub start_time: i64,
    /// 录像结束时间（UTC 毫秒），录制中为 None
    pub end_time: Option<i64>,
    /// 录像总时长（毫秒），录制中为 None
    pub duration_ms: Option<i64>,
    /// 文件大小（字节），闭合后更新
    pub file_size: Option<i64>,
    /// 编码格式：h264 / h265
    #[sea_orm(column_type = "Text")]
    pub codec: String,
    /// 状态：recording / completed / truncated
    #[sea_orm(column_type = "Text")]
    pub status: String,
    /// 创建时间（UTC 毫秒）
    pub created_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(
        belongs_to = "super::camera::Entity",
        from = "Column::CameraId",
        to = "super::camera::Column::CameraId"
    )]
    Camera,
    #[sea_orm(has_many = "super::recording_event::Entity")]
    RecordingEvents,
}

impl Related<super::camera::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Camera.def()
    }
}

impl Related<super::recording_event::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::RecordingEvents.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
