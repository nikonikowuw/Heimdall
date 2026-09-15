use serde::{Deserialize, Serialize};

/// 人脸特征重新提取单项失败明细
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReextractFaceFailureDetail {
    pub face_id: String,
    pub subject_id: String,
    pub reason: String,
}

/// 人脸特征重新提取任务运行状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReextractTaskStatus {
    #[default]
    Idle,
    Running,
    Completed,
    Failed,
}

impl ReextractTaskStatus {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

/// 人脸特征重新提取任务实时进度与状态 DTO
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReextractProgressDto {
    pub status: ReextractTaskStatus,
    pub total: u64,
    pub processed: u64,
    pub succeeded: u64,
    pub failed: u64,
    pub current_face_id: Option<String>,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub failures: Vec<ReextractFaceFailureDetail>,
    pub error_message: Option<String>,
}

/// 人脸特征重新提取任务执行报告（针对单人或终态）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ReextractFaceFeaturesReportDto {
    pub total: u64,
    pub succeeded: u64,
    pub failed: u64,
    pub failures: Vec<ReextractFaceFailureDetail>,
}

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

/// 1:N 人脸特征检索 Top-K 候选人明细项
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FaceCandidateItem {
    pub rank: usize,
    pub subject_id: String,
    pub subject_name: String,
    pub face_id: String,
    pub photo_rel_path: String,
    pub similarity: f32,
}

/// 识别对账审核状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RecognitionStatus {
    /// 高置信自动确认放行
    #[default]
    Confirmed,
    /// 位于疑似区间，等待人工核验
    PendingReview,
    /// 人工复核已驳回或标记为陌生人
    Rejected,
}

impl RecognitionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::PendingReview => "pending_review",
            Self::Rejected => "rejected",
        }
    }
}

impl std::str::FromStr for RecognitionStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "confirmed" => Ok(Self::Confirmed),
            "pending_review" | "pending" => Ok(Self::PendingReview),
            "rejected" => Ok(Self::Rejected),
            other => Err(format!("未知的识别状态: {other}")),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_recognition_status_from_str() {
        assert_eq!(
            "confirmed".parse::<RecognitionStatus>().unwrap(),
            RecognitionStatus::Confirmed
        );
        assert_eq!(
            "pending_review".parse::<RecognitionStatus>().unwrap(),
            RecognitionStatus::PendingReview
        );
        assert_eq!(
            "pending".parse::<RecognitionStatus>().unwrap(),
            RecognitionStatus::PendingReview
        );
        assert_eq!(
            "rejected".parse::<RecognitionStatus>().unwrap(),
            RecognitionStatus::Rejected
        );
        assert!("invalid_status".parse::<RecognitionStatus>().is_err());
    }
}
