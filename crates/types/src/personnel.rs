use serde::{Deserialize, Serialize};

/// 人员列表展示 DTO
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PersonnelItemDto {
    pub id: i64,
    pub subject_id: String,
    pub name: String,
    pub id_card: String,
    pub remark: String,
    pub primary_photo_path: String,
    pub face_count: u32,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 人脸特征样本 DTO
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GalleryFaceDto {
    pub id: i64,
    pub face_id: String,
    pub subject_id: String,
    pub photo_rel_path: String,
    pub aligned_rel_path: String,
    pub quality_score: f32,
    pub detection_score: f32,
    pub is_primary: bool,
    pub created_at: i64,
}

/// 人员详情 DTO（包含所有人脸特征样本）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PersonnelDetailDto {
    pub id: i64,
    pub subject_id: String,
    pub name: String,
    pub id_card: String,
    pub remark: String,
    pub primary_photo_path: String,
    pub faces: Vec<GalleryFaceDto>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// 更新人员基本信息请求
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct UpdatePersonnelRequest {
    pub name: Option<String>,
    pub id_card: Option<String>,
    pub remark: Option<String>,
}

/// 人员底库全局统计数据
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PersonnelStatsDto {
    pub total_personnel: u64,
    pub total_faces: u64,
    pub algo_ready: bool,
}

/// 1:N 人脸特征检索比对命中结果
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FaceMatchResult {
    pub subject_id: String,
    pub subject_name: String,
    pub face_id: String,
    pub photo_rel_path: String,
    pub similarity: f32,
}
