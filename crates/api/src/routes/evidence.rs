use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use chrono::DateTime;
use serde::{Deserialize, Serialize};

use db::{CaptureRepo, RecognitionRepo};

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
    pub crop_image_rel_path: String,
    pub captured_at: i64,
    pub created_at: i64,
}

impl From<db::entity::capture::Model> for CaptureDto {
    fn from(m: db::entity::capture::Model) -> Self {
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
    pub registered_photo_path: String,
    pub recognized_at: i64,
    pub created_at: i64,
}

impl From<db::entity::recognition::Model> for RecognitionDto {
    fn from(m: db::entity::recognition::Model) -> Self {
        Self {
            id: m.id,
            recognition_id: m.recognition_id,
            camera_id: m.camera_id,
            gallery_id: m.gallery_id,
            subject_id: m.subject_id,
            subject_name: m.subject_name,
            similarity: m.similarity,
            field_crop_path: m.field_crop_path,
            registered_photo_path: m.registered_photo_path,
            recognized_at: m.recognized_at.timestamp_millis(),
            created_at: m.created_at.timestamp_millis(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct EvidenceQuery {
    pub camera_id: Option<String>,
    pub target_label: Option<String>,
    pub start_time: Option<i64>,
    pub end_time: Option<i64>,
    #[serde(default = "default_limit")]
    pub limit: u64,
    #[serde(default)]
    pub offset: u64,
}

fn default_limit() -> u64 {
    20
}

/// 受保护的证据数据接口路由
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/captures", get(list_captures))
        .route("/recognitions", get(list_recognitions))
}

/// 证据图片提供路由（支持 Header 或 ?token= 校验）
pub fn image_router() -> Router<AppState> {
    Router::new().route("/{*path}", get(serve_evidence_image))
}

async fn list_captures(
    State(state): State<AppState>,
    Query(params): Query<EvidenceQuery>,
) -> Result<ApiResponse<Vec<CaptureDto>>, ApiError> {
    let start_utc = params.start_time.and_then(DateTime::from_timestamp_millis);
    let end_utc = params.end_time.and_then(DateTime::from_timestamp_millis);

    let list = CaptureRepo::list_filtered(
        &state.db,
        params.camera_id.as_deref(),
        params.target_label.as_deref(),
        start_utc,
        end_utc,
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
    let list = RecognitionRepo::list_recent(
        &state.db,
        params.camera_id.as_deref(),
        params.limit,
        params.offset,
    )
    .await?;
    let dtos = list.into_iter().map(RecognitionDto::from).collect();
    Ok(ApiResponse::success(dtos))
}

/// 安全提供证据图片（受权鉴保护，防止越权与路径穿越）
async fn serve_evidence_image(
    State(state): State<AppState>,
    _user: AuthUser,
    headers: HeaderMap,
    Path(path): Path<String>,
) -> Result<Response, StatusCode> {
    let clean_path = path.trim_start_matches('/').trim_start_matches('\\');
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

    tracing::info!(
        camera_id = %camera_id,
        user = %_user.username,
        client_ip = %client_ip,
        path = %clean_path,
        "证据图片访问"
    );

    // 严防路径穿越与空路径
    if clean_path.is_empty() || clean_path.contains("..") {
        return Err(StatusCode::FORBIDDEN);
    }

    let base_dir = state.pipeline.snapshot_engine().base_evidence_dir();
    let full_path = base_dir.join(clean_path);

    // 校验 base_dir 存在性
    if !base_dir.exists() {
        return Err(StatusCode::NOT_FOUND);
    }

    let canonical_base = match tokio::fs::canonicalize(base_dir).await {
        Ok(p) => p,
        Err(_) => return Err(StatusCode::NOT_FOUND),
    };
    let canonical_target = match tokio::fs::canonicalize(&full_path).await {
        Ok(p) => p,
        Err(_) => return Err(StatusCode::NOT_FOUND),
    };

    if !canonical_target.starts_with(&canonical_base) {
        return Err(StatusCode::FORBIDDEN);
    }

    match tokio::fs::read(&canonical_target).await {
        Ok(bytes) => {
            let mime = if clean_path.ends_with(".png") {
                "image/png"
            } else {
                "image/jpeg"
            };

            let res = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime)
                .header(header::CACHE_CONTROL, "private, max-age=86400")
                .body(Body::from(bytes))
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            Ok(res)
        }
        Err(_) => Err(StatusCode::NOT_FOUND),
    }
}
