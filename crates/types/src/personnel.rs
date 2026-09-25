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

/// 批量导入中单个候选人员的失败归因分类
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportFailureKind {
    /// 上传照片无法解码或转码
    Transcode,
    /// 未在照片中检出有效人脸
    NoFace,
    /// 人脸质量分低于录入门禁
    QualityLow,
    /// 与其它主体底库样本高度相似
    Clash,
    /// 人员编号或姓名与在册档案冲突
    Conflict,
    /// 归档内缺少可用照片
    NoPhoto,
    /// 其它未归类的服务端错误
    Internal,
}

impl ImportFailureKind {
    /// 用于日志与前端筛选的稳定标识
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Transcode => "transcode",
            Self::NoFace => "no_face",
            Self::QualityLow => "quality_low",
            Self::Clash => "clash",
            Self::Conflict => "conflict",
            Self::NoPhoto => "no_photo",
            Self::Internal => "internal",
        }
    }
}

/// 批量导入单个候选人员的失败明细
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ImportFailureDetail {
    /// 候选姓名；约定推断模式解析失败时可能为空串
    pub name: String,
    /// 候选编号；未分配编号时为空串
    pub subject_id: String,
    /// 失败归因分类
    pub kind: ImportFailureKind,
    /// 人类可读的失败原因
    pub reason: String,
    /// 因超出 5 张上限而被忽略的照片数量
    pub skipped_photos: u32,
}

/// 批量导入任务运行状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ImportTaskStatus {
    #[default]
    Idle,
    Running,
    Completed,
    /// 全部候选均失败（终态，需人工介入）
    Failed,
    /// 管理员主动中止
    Cancelled,
}

impl ImportTaskStatus {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

/// 批量导入任务实时进度与终态报告 DTO
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PersonnelImportProgressDto {
    /// 任务唯一标识；空闲状态为空串
    pub task_id: String,
    pub status: ImportTaskStatus,
    /// 归档内解析出的候选人员总数
    pub total: u64,
    /// 已处理候选人数
    pub processed: u64,
    pub succeeded: u64,
    pub failed: u64,
    /// 正在处理的候选姓名
    pub current_name: Option<String>,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    /// 失败明细，按处理顺序累积
    pub failures: Vec<ImportFailureDetail>,
    /// 归档解析与任务级错误（非单候选失败）
    pub error_message: Option<String>,
}

impl PersonnelImportProgressDto {
    /// 任务是否处于运行中状态
    pub fn is_running(&self) -> bool {
        self.status == ImportTaskStatus::Running
    }
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

    #[test]
    fn test_import_task_status_serialization_is_snake_case() {
        assert_eq!(
            serde_json::to_string(&ImportTaskStatus::Cancelled).unwrap(),
            "\"cancelled\""
        );
        assert_eq!(
            serde_json::to_string(&ImportTaskStatus::Idle).unwrap(),
            "\"idle\""
        );
    }

    #[test]
    fn test_import_progress_running_predicate_only_matches_running() {
        let mut progress = PersonnelImportProgressDto::default();
        assert!(!progress.is_running());
        progress.status = ImportTaskStatus::Running;
        assert!(progress.is_running());
        progress.status = ImportTaskStatus::Completed;
        assert!(!progress.is_running());
    }

    #[test]
    fn test_import_failure_kind_is_stable_for_frontend_filtering() {
        assert_eq!(ImportFailureKind::NoFace.as_str(), "no_face");
        assert_eq!(ImportFailureKind::QualityLow.as_str(), "quality_low");
        assert_eq!(ImportFailureKind::Clash.as_str(), "clash");
    }
}
