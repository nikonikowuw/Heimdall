use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "capture_records")]
pub struct Model {
    #[sea_orm(primary_key)]
    pub id: i64,
    #[sea_orm(unique, column_type = "Text")]
    pub capture_id: String,
    #[sea_orm(column_type = "Text")]
    pub camera_id: String,
    pub track_id: i64,
    pub target_label: String,
    pub confidence: f32,
    pub quality_score: f32,
    #[sea_orm(column_type = "Text")]
    pub bbox_json: String,
    pub image_id: String,
    pub image_rel_path: String,
    pub crop_image_id: String,
    pub crop_image_rel_path: String,
    /// 人体特写（抓拍记录恒产出）；空串 = 未产出或迁移前遗留行。
    #[sea_orm(column_type = "Text")]
    pub body_crop_image_id: String,
    /// 人体特写相对路径；空串 = 未产出或迁移前遗留行。
    #[sea_orm(column_type = "Text")]
    pub body_crop_image_rel_path: String,
    /// 证据图产生路径（`peak_candidate` / `targeted`）；空串 = 未标注。
    #[sea_orm(column_type = "Text")]
    pub image_source: String,
    /// 证据图所属码流（`main` / `sub`）；空串 = 未标注。
    #[sea_orm(column_type = "Text")]
    pub image_stream: String,
    /// 证据图帧 PTS（仅与检测轴同轴时记录，不同轴或未知为 0）。
    pub image_pts_ms: i64,
    /// 匹配所用融合模板的参与帧数；无 sidecar 的旧包与未上报帧为 NULL。
    pub fused_count: Option<i64>,
    /// 匹配所用融合模板的质量加权均值；语义同上。
    pub template_quality: Option<f32>,
    pub captured_at: DateTimeUtc,
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
