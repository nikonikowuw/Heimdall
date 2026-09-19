use axum::body::Body;
use axum::extract::{Json, Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use chrono::DateTime;
use serde::{Deserialize, Serialize};

use db::{CaptureRepo, RecognitionRepo};

use crate::capture_service::{
    isolate_gallery_evidence_photo, normalize_evidence_relative_path, parse_evidence_origin,
};
use crate::error::ApiError;
use crate::middleware::AuthUser;
use crate::response::ApiResponse;
use crate::state::AppState;

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureDto {
    pub id: i64,
    pub capture_id: String,
    pub camera_id: String,
    pub track_id: i64,
    pub target_label: String,
    pub confidence: f32,
    pub quality_score: f32,
    pub bbox_json: String,
    pub image_id: String,
    pub image_rel_path: String,
    pub crop_image_id: String,
    /// 人脸特写相对路径；空串 = 本次未产出（背身/低头）或迁移前遗留行。
    pub crop_image_rel_path: String,
    pub body_crop_image_id: String,
    /// 人体特写相对路径（人工复核看衣着的主体证据）；空串语义同上。
    pub body_crop_image_rel_path: String,
    /// 证据图产生路径；历史记录未标注时为 `null`。
    pub image_source: Option<types::EvidenceImageSource>,
    /// 证据图所属码流；历史记录未标注时为 `null`。
    pub image_stream: Option<types::EvidenceImageStream>,
    /// 证据帧的可比 PTS（检测轴）；不可比或未知时为 `null`。
    pub image_pts_ms: Option<i64>,
    /// 匹配所用融合模板的参与帧数（仅新版算法包 sidecar 上报）。
    pub fused_count: Option<i64>,
    /// 匹配所用融合模板的质量加权均值（语义同上）。
    pub template_quality: Option<f32>,
    pub captured_at: i64,
    pub created_at: i64,
}

impl From<db::entity::capture::Model> for CaptureDto {
    fn from(m: db::entity::capture::Model) -> Self {
        let (image_source, image_stream, image_pts_ms) =
            parse_evidence_origin(&m.image_source, &m.image_stream, m.image_pts_ms);
        Self {
            id: m.id,
            capture_id: m.capture_id,
            camera_id: m.camera_id,
            track_id: m.track_id,
            target_label: m.target_label,
            confidence: m.confidence,
            quality_score: m.quality_score,
            bbox_json: m.bbox_json,
            image_id: m.image_id,
            image_rel_path: m.image_rel_path,
            crop_image_id: m.crop_image_id,
            crop_image_rel_path: m.crop_image_rel_path,
            body_crop_image_id: m.body_crop_image_id,
            body_crop_image_rel_path: m.body_crop_image_rel_path,
            image_source,
            image_stream,
            image_pts_ms,
            fused_count: m.fused_count,
            template_quality: m.template_quality,
            captured_at: m.captured_at.timestamp_millis(),
            created_at: m.created_at.timestamp_millis(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecognitionDto {
    pub id: i64,
    pub recognition_id: String,
    pub camera_id: String,
    pub gallery_id: String,
    pub subject_id: String,
    pub subject_name: String,
    pub similarity: f32,
    pub field_crop_path: String,
    pub field_image_path: Option<String>,
    pub field_bbox_json: Option<String>,
    pub registered_photo_path: String,
    /// 证据图产生路径；历史记录未标注时为 `null`。
    pub image_source: Option<types::EvidenceImageSource>,
    /// 证据图所属码流；历史记录未标注时为 `null`。
    pub image_stream: Option<types::EvidenceImageStream>,
    /// 证据帧的可比 PTS（检测轴）；不可比或未知时为 `null`。
    pub image_pts_ms: Option<i64>,
    /// 1:N 比对所用融合模板的参与帧数（仅新版算法包 sidecar 上报）。
    pub fused_count: Option<i64>,
    /// 1:N 比对所用融合模板的质量加权均值（语义同上）。
    pub template_quality: Option<f32>,
    pub status: String,
    pub candidates: Vec<types::FaceCandidateItem>,
    pub reviewer_id: Option<String>,
    pub reviewed_at: Option<i64>,
    pub recognized_at: i64,
    pub created_at: i64,
}

impl From<db::entity::recognition::Model> for RecognitionDto {
    fn from(m: db::entity::recognition::Model) -> Self {
        let candidates = m
            .candidates_json
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();
        let field_image_path = Some(m.field_image_path).filter(|s| !s.trim().is_empty());
        let field_bbox_json = Some(m.field_bbox_json).filter(|s| !s.trim().is_empty());
        let (image_source, image_stream, image_pts_ms) =
            parse_evidence_origin(&m.image_source, &m.image_stream, m.image_pts_ms);
        Self {
            id: m.id,
            recognition_id: m.recognition_id,
            camera_id: m.camera_id,
            gallery_id: m.gallery_id,
            subject_id: m.subject_id,
            subject_name: m.subject_name,
            similarity: m.similarity,
            field_crop_path: m.field_crop_path,
            field_image_path,
            field_bbox_json,
            registered_photo_path: m.registered_photo_path,
            image_source,
            image_stream,
            image_pts_ms,
            fused_count: m.fused_count,
            template_quality: m.template_quality,
            status: m.status,
            candidates,
            reviewer_id: m.reviewer_id,
            reviewed_at: m.reviewed_at.map(|t| t.timestamp_millis()),
            recognized_at: m.recognized_at.timestamp_millis(),
            created_at: m.created_at.timestamp_millis(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct EvidenceQuery {
    pub camera_id: Option<String>,
    pub target_label: Option<String>,
    /// 轨道过滤：定位"同一个人的一次通行"的全部结算记录。仅抓拍列表支持。
    pub track_id: Option<i64>,
    pub status: Option<String>,
    pub start_time: Option<i64>,
    pub end_time: Option<i64>,
    #[serde(default = "default_limit")]
    pub limit: u64,
    #[serde(default)]
    pub offset: u64,
}

impl EvidenceQuery {
    pub fn time_range(&self) -> (Option<DateTime<chrono::Utc>>, Option<DateTime<chrono::Utc>>) {
        (
            self.start_time.and_then(DateTime::from_timestamp_millis),
            self.end_time.and_then(DateTime::from_timestamp_millis),
        )
    }

    pub fn to_capture_filter(&self) -> db::CaptureFilter<'_> {
        let (start_time, end_time) = self.time_range();
        db::CaptureFilter {
            camera_id: self.camera_id.as_deref(),
            target_label: self.target_label.as_deref(),
            track_id: self.track_id,
            start_time,
            end_time,
        }
    }
}

fn default_limit() -> u64 {
    20
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceCountDto {
    pub total: u64,
}

/// 受保护的证据数据接口路由
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/captures", get(list_captures))
        .route("/captures/count", get(count_captures))
        .route("/recognitions", get(list_recognitions))
        .route("/recognitions/count", get(count_recognitions))
        .route("/recognitions/{recognition_id}", get(get_recognition))
        .route(
            "/recognitions/{recognition_id}/review",
            post(review_recognition),
        )
}

/// 证据图片提供路由（支持 Header 或 ?token= 校验）
pub fn image_router() -> Router<AppState> {
    Router::new().route("/{*path}", get(serve_evidence_image))
}

async fn count_captures(
    State(state): State<AppState>,
    Query(params): Query<EvidenceQuery>,
) -> Result<ApiResponse<EvidenceCountDto>, ApiError> {
    let total = CaptureRepo::count_filtered(&state.db, params.to_capture_filter()).await?;
    Ok(ApiResponse::success(EvidenceCountDto { total }))
}

async fn count_recognitions(
    State(state): State<AppState>,
    Query(params): Query<EvidenceQuery>,
) -> Result<ApiResponse<EvidenceCountDto>, ApiError> {
    let (start_utc, end_utc) = params.time_range();
    let total = RecognitionRepo::count_filtered(
        &state.db,
        params.camera_id.as_deref(),
        params.status.as_deref(),
        start_utc,
        end_utc,
    )
    .await?;
    Ok(ApiResponse::success(EvidenceCountDto { total }))
}

async fn list_captures(
    State(state): State<AppState>,
    Query(params): Query<EvidenceQuery>,
) -> Result<ApiResponse<Vec<CaptureDto>>, ApiError> {
    let list = CaptureRepo::list_filtered(
        &state.db,
        params.to_capture_filter(),
        params.limit,
        params.offset,
    )
    .await?;
    let dtos = list.into_iter().map(CaptureDto::from).collect();
    Ok(ApiResponse::success(dtos))
}

async fn list_recognitions(
    State(state): State<AppState>,
    Query(params): Query<EvidenceQuery>,
) -> Result<ApiResponse<Vec<RecognitionDto>>, ApiError> {
    let (start_utc, end_utc) = params.time_range();
    let list = RecognitionRepo::list_filtered(
        &state.db,
        params.camera_id.as_deref(),
        params.status.as_deref(),
        start_utc,
        end_utc,
        params.limit,
        params.offset,
    )
    .await?;
    let dtos = list.into_iter().map(RecognitionDto::from).collect();
    Ok(ApiResponse::success(dtos))
}

async fn get_recognition(
    State(state): State<AppState>,
    Path(recognition_id): Path<String>,
) -> Result<ApiResponse<RecognitionDto>, ApiError> {
    let rec = RecognitionRepo::find_by_recognition_id(&state.db, &recognition_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("识别记录不存在".to_string()))?;
    Ok(ApiResponse::success(RecognitionDto::from(rec)))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRecognitionRequest {
    pub status: String,
    pub subject_id: Option<String>,
    pub subject_name: Option<String>,
    pub photo_rel_path: Option<String>,
    pub similarity: Option<f32>,
}

async fn review_recognition(
    State(state): State<AppState>,
    user: AuthUser,
    Path(recognition_id): Path<String>,
    Json(req): Json<ReviewRecognitionRequest>,
) -> Result<ApiResponse<RecognitionDto>, ApiError> {
    let status_enum = req
        .status
        .parse::<types::RecognitionStatus>()
        .map_err(|_| ApiError::BadRequest("状态必须为 confirmed 或 rejected".to_string()))?;
    if status_enum == types::RecognitionStatus::PendingReview {
        return Err(ApiError::BadRequest(
            "审核结果状态必须为 confirmed 或 rejected".to_string(),
        ));
    }

    // 若核验指定了候选底库照片，执行安全证据隔离复制并更新记录引用
    let updated_photo_rel = match req.photo_rel_path.as_deref() {
        Some(photo) => {
            let base_dir = state.pipeline.snapshot_engine().base_evidence_dir();
            Some(
                isolate_gallery_evidence_photo(base_dir, photo, &recognition_id)
                    .await
                    .unwrap_or_else(|| photo.to_string()),
            )
        }
        None => None,
    };

    let updated = RecognitionRepo::update_review_status(
        &state.db,
        db::UpdateRecognitionReviewParams {
            recognition_id: &recognition_id,
            status: status_enum.as_str(),
            reviewer_id: Some(&user.username),
            selected_subject_id: req.subject_id.as_deref(),
            selected_subject_name: req.subject_name.as_deref(),
            selected_photo_path: updated_photo_rel.as_deref(),
            selected_similarity: req.similarity,
        },
    )
    .await?
    .ok_or_else(|| ApiError::NotFound("识别记录不存在".to_string()))?;

    let dto = RecognitionDto::from(updated);

    // 广播对账状态变更事件，通知全网监控终端
    let _ = state
        .event_broadcaster
        .send(crate::state::WsBroadcastEvent {
            topic: types::TOPIC_RECOGNITION_STATUS_CHANGED.to_string(),
            payload: serde_json::json!({
                "recognitionId": dto.recognition_id,
                "status": dto.status,
                "reviewerId": dto.reviewer_id,
                "reviewedAt": dto.reviewed_at,
                "subjectId": dto.subject_id,
                "subjectName": dto.subject_name,
                "similarity": dto.similarity,
                "registeredPhotoPath": dto.registered_photo_path,
            }),
            timestamp: chrono::Utc::now().timestamp_millis(),
        });

    Ok(ApiResponse::success(dto))
}

/// 安全提供证据图片（受权鉴保护，防止越权与路径穿越）
async fn serve_evidence_image(
    State(state): State<AppState>,
    _user: AuthUser,
    headers: HeaderMap,
    Path(path): Path<String>,
) -> Result<Response, StatusCode> {
    let clean_path = match normalize_evidence_relative_path(&path) {
        Some(c) => c,
        None => return Err(StatusCode::FORBIDDEN),
    };
    let camera_id = clean_path.split(['/', '\\']).next().unwrap_or("unknown");
    let client_ip = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(str::trim)
        .or_else(|| {
            headers
                .get("x-real-ip")
                .and_then(|v| v.to_str().ok())
                .map(str::trim)
        })
        .unwrap_or("unknown");

    tracing::debug!(
        camera_id = %camera_id,
        user = %_user.username,
        client_ip = %client_ip,
        path = %clean_path,
        "证据图片访问"
    );

    // 严防路径穿越与空路径 (normalize_evidence_relative_path 已校验)
    if clean_path.is_empty() {
        return Err(StatusCode::FORBIDDEN);
    }

    let engine = state.pipeline.snapshot_engine();
    let base_dir = engine.base_evidence_dir();
    let full_path = base_dir.join(clean_path);

    // 优先复用 SnapshotEngine 启动时已规范化的 base_dir 句柄，避免每次高频图片请求都对根目录重复执行系统调用
    let fallback_canonical_base;
    let canonical_base: &std::path::Path = match engine.canonical_base_evidence_dir() {
        Some(p) => p,
        None => {
            if !base_dir.exists() {
                return Err(StatusCode::NOT_FOUND);
            }
            fallback_canonical_base = match tokio::fs::canonicalize(base_dir).await {
                Ok(p) => p,
                Err(_) => return Err(StatusCode::NOT_FOUND),
            };
            &fallback_canonical_base
        }
    };
    let canonical_target = match tokio::fs::canonicalize(&full_path).await {
        Ok(p) => p,
        Err(_) => return Err(StatusCode::NOT_FOUND),
    };

    if !canonical_target.starts_with(canonical_base) {
        return Err(StatusCode::FORBIDDEN);
    }

    let metadata = match tokio::fs::metadata(&canonical_target).await {
        Ok(m) => m,
        Err(_) => return Err(StatusCode::NOT_FOUND),
    };

    if !metadata.is_file() {
        return Err(StatusCode::NOT_FOUND);
    }

    let file_size = metadata.len();
    let mtime_nanos = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let etag = format!("\"{:x}-{:x}\"", file_size, mtime_nanos);

    if let Some(if_none_match) = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
    {
        if if_none_match == etag {
            return Response::builder()
                .status(StatusCode::NOT_MODIFIED)
                .header(header::ETAG, etag)
                .header(header::CACHE_CONTROL, "private, max-age=86400")
                .body(Body::empty())
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

    let file = match tokio::fs::File::open(&canonical_target).await {
        Ok(f) => f,
        Err(_) => return Err(StatusCode::NOT_FOUND),
    };
    let stream = tokio_util::io::ReaderStream::with_capacity(file, 64 * 1024);
    let body = Body::from_stream(stream);

    let mime = if clean_path.ends_with(".png") {
        "image/png"
    } else {
        "image/jpeg"
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime)
        .header(header::CONTENT_LENGTH, file_size)
        .header(header::ETAG, etag)
        .header(header::CACHE_CONTROL, "private, max-age=86400")
        .body(body)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}
