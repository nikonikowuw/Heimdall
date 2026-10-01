//! 事件录像查询与播放接口
//!
//! - 元数据查询：`GET /recordings`、`GET /recordings/{id}`
//! - 事件反查：`GET /recordings/by-event/{eventType}/{eventId}`
//! - 文件播放：`GET /recordings/{id}/file`（支持 HTTP Range，浏览器 `<video>` 直接 seek）
//! - 手动删除：`DELETE /recordings/{id}`
//!
//! 文件服务复用存储内的 `recordings/{camera_id}/{date}/...` 相对路径，
//! 并提供与证据图片一致的 `canonicalize` 路径穿越防护。

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde::{Deserialize, Serialize};
use tower::ServiceExt;
use tower_http::services::ServeFile;

use crate::error::ApiError;
use crate::middleware::AuthUser;
use crate::response::ApiResponse;
use crate::state::AppState;

const DEFAULT_LIMIT: u64 = 20;
const MAX_LIMIT: u64 = 100;

/// 录像元数据 DTO
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingDto {
    pub id: i64,
    pub recording_id: String,
    pub camera_id: String,
    pub start_time: i64,
    pub end_time: Option<i64>,
    pub duration_ms: Option<i64>,
    pub file_size: Option<i64>,
    pub codec: String,
    pub status: String,
    pub created_at: i64,
}

impl From<db::entity::recording::Model> for RecordingDto {
    fn from(m: db::entity::recording::Model) -> Self {
        Self {
            id: m.id,
            recording_id: m.recording_id,
            camera_id: m.camera_id,
            start_time: m.start_time,
            end_time: m.end_time,
            duration_ms: m.duration_ms,
            file_size: m.file_size,
            codec: m.codec,
            status: m.status,
            created_at: m.created_at,
        }
    }
}

/// 事件关联信息（含回放 seek 偏移）
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingEventDto {
    pub event_type: String,
    pub event_id: String,
    pub event_time: i64,
    /// 事件在录像文件内的时间偏移（毫秒），前端据此 `currentTime = offset/1000`
    pub offset_ms: i64,
}

impl From<db::entity::recording_event::Model> for RecordingEventDto {
    fn from(m: db::entity::recording_event::Model) -> Self {
        Self {
            event_type: m.event_type,
            event_id: m.event_id,
            event_time: m.event_time,
            offset_ms: m.offset_ms,
        }
    }
}

/// 录像详情（元数据 + 关联事件）
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingDetailDto {
    #[serde(flatten)]
    pub recording: RecordingDto,
    pub events: Vec<RecordingEventDto>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingQuery {
    pub camera_id: Option<String>,
    pub status: Option<String>,
    pub start_time: Option<i64>,
    pub end_time: Option<i64>,
    #[serde(default = "default_limit")]
    pub limit: u64,
    #[serde(default)]
    pub offset: u64,
}

fn default_limit() -> u64 {
    DEFAULT_LIMIT
}

/// 录像查询路由（挂载于 `/api/v1/recordings`）
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_recordings))
        .route("/by-event/{event_type}/{event_id}", get(find_by_event))
        .route(
            "/{recording_id}",
            get(get_recording).delete(delete_recording),
        )
        .route("/{recording_id}/file", get(serve_recording_file))
}

async fn list_recordings(
    State(state): State<AppState>,
    Query(params): Query<RecordingQuery>,
) -> Result<ApiResponse<Vec<RecordingDto>>, ApiError> {
    let limit = params.limit.clamp(1, MAX_LIMIT);
    let list = db::RecordingRepo::list_filtered(
        &state.db,
        db::RecordingFilter {
            camera_id: params.camera_id.as_deref(),
            status: params.status.as_deref(),
            start_time_ms: params.start_time,
            end_time_ms: params.end_time,
        },
        limit,
        params.offset,
    )
    .await?;
    Ok(ApiResponse::success(
        list.into_iter().map(RecordingDto::from).collect(),
    ))
}

async fn get_recording(
    State(state): State<AppState>,
    Path(recording_id): Path<String>,
) -> Result<ApiResponse<RecordingDetailDto>, ApiError> {
    let rec = db::RecordingRepo::find_by_id(&state.db, &recording_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("录像不存在".to_string()))?;
    let events = db::RecordingRepo::list_events(&state.db, &recording_id).await?;
    Ok(ApiResponse::success(RecordingDetailDto {
        recording: RecordingDto::from(rec),
        events: events.into_iter().map(RecordingEventDto::from).collect(),
    }))
}

/// 通过事件反查关联录像（告警/识别详情页回放入口）
async fn find_by_event(
    State(state): State<AppState>,
    Path((event_type, event_id)): Path<(String, String)>,
) -> Result<ApiResponse<Option<RecordingDetailDto>>, ApiError> {
    // 事件类型白名单，避免任意字符串进入查询
    if event_type != "alarm" && event_type != "recognition" {
        return Err(ApiError::BadRequest(
            "eventType 必须为 alarm 或 recognition".to_string(),
        ));
    }

    let found = db::RecordingRepo::find_by_event(&state.db, &event_type, &event_id).await?;
    let Some((rec, _event)) = found else {
        return Ok(ApiResponse::success(None));
    };
    let events = db::RecordingRepo::list_events(&state.db, &rec.recording_id).await?;
    Ok(ApiResponse::success(Some(RecordingDetailDto {
        recording: RecordingDto::from(rec),
        events: events.into_iter().map(RecordingEventDto::from).collect(),
    })))
}

async fn delete_recording(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(recording_id): Path<String>,
) -> Result<ApiResponse<serde_json::Value>, ApiError> {
    let rec = db::RecordingRepo::find_by_id(&state.db, &recording_id)
        .await?
        .ok_or_else(|| ApiError::NotFound("录像不存在".to_string()))?;

    // 先删物理文件，再删 DB 记录：图销案销，避免孤儿
    let file_path = resolve_recording_path(&state.base_evidence_dir(), &rec.file_path)
        .map_err(|_| ApiError::BadRequest("录像路径非法".to_string()))?;
    if file_path.exists() {
        if let Err(error) = std::fs::remove_file(&file_path) {
            tracing::warn!(
                recording_id = %recording_id,
                path = %file_path.display(),
                %error,
                "删除录像物理文件失败，仍继续清理数据库记录"
            );
        }
    } else {
        tracing::warn!(
            recording_id = %recording_id,
            path = %file_path.display(),
            "录像物理文件已不存在，仅清理数据库记录"
        );
    }

    db::RecordingRepo::delete_by_id(&state.db, &recording_id).await?;
    Ok(ApiResponse::success(serde_json::json!({ "deleted": true })))
}

/// 安全提供录像文件（支持 HTTP Range，浏览器可直接 `<video>` 播放与 seek）
async fn serve_recording_file(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(recording_id): Path<String>,
    request: axum::extract::Request,
) -> Response {
    let rec = match db::RecordingRepo::find_by_id(&state.db, &recording_id).await {
        Ok(Some(rec)) => rec,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            tracing::error!(recording_id = %recording_id, %error, "查询录像记录失败");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let full_path = match resolve_recording_path(&state.base_evidence_dir(), &rec.file_path) {
        Ok(path) => path,
        Err(_) => return StatusCode::FORBIDDEN.into_response(),
    };

    // 强制要求文件位于证据存储根内（canonicalize 解析符号链接后再比对）
    let evidence_root = state.base_evidence_dir();
    let canonical_root = match tokio::fs::canonicalize(&evidence_root).await {
        Ok(root) => root,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let canonical_file = match tokio::fs::canonicalize(&full_path).await {
        Ok(path) => path,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    if !canonical_file.starts_with(&canonical_root) {
        tracing::warn!(
            recording_id = %recording_id,
            path = %canonical_file.display(),
            "录像文件路径越界，拒绝访问"
        );
        return StatusCode::FORBIDDEN.into_response();
    }

    // ServeFile 原生处理 Range / If-Modified-Since / 206 Partial Content，
    // MIME 由文件扩展名推导（.mp4 → video/mp4）
    let service = ServeFile::new(&canonical_file);
    match service.oneshot(request).await {
        Ok(response) => response.into_response(),
        Err(error) => {
            tracing::debug!(%error, "录像文件服务响应失败");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// 把数据库中的录像相对路径解析为绝对路径，并拒绝任何路径穿越成分。
///
/// DB 中存储的是相对于录像根目录的路径（`{camera_id}/{date}/{file}.mp4`）。
fn resolve_recording_path(root: &std::path::Path, stored: &str) -> Result<std::path::PathBuf, ()> {
    let candidate = std::path::Path::new(stored);

    // 允许绝对路径直接落库（早期实现），但必须仍在 root 内
    let relative = if candidate.is_absolute() {
        candidate.strip_prefix(root).map_err(|_| ())?.to_path_buf()
    } else {
        candidate.to_path_buf()
    };

    // 拒绝 `..` / 根组件 / 前缀组件
    for component in relative.components() {
        match component {
            std::path::Component::Normal(_) => {}
            _ => return Err(()),
        }
    }

    Ok(root.join(relative))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_relative_path() {
        let root = std::path::Path::new("/data");
        let resolved =
            resolve_recording_path(root, "cam-01/2025-07-15/083052_abcdef12.mp4").unwrap();
        assert_eq!(
            resolved,
            std::path::PathBuf::from("/data/cam-01/2025-07-15/083052_abcdef12.mp4")
        );
    }

    #[test]
    fn test_resolve_rejects_traversal() {
        let root = std::path::Path::new("/data");
        assert!(resolve_recording_path(root, "../../etc/passwd").is_err());
        assert!(resolve_recording_path(root, "cam/../../../etc/passwd").is_err());
        assert!(resolve_recording_path(root, "/etc/passwd").is_err());
    }

    #[test]
    fn test_resolve_accepts_absolute_inside_root() {
        let root = std::path::Path::new("/data");

        // 绝对路径且在 root 内 → 正常解析（兼容早期实现落库的绝对路径）
        let inside = resolve_recording_path(root, "/data/recordings/cam/2025-07-15/a.mp4").unwrap();
        assert_eq!(
            inside,
            std::path::PathBuf::from("/data/recordings/cam/2025-07-15/a.mp4")
        );

        // 绝对路径但在 root 外 → 拒绝
        assert!(resolve_recording_path(root, "/etc/passwd").is_err());
    }
}
