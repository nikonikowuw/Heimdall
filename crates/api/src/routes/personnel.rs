use axum::extract::{Multipart, Path as AxumPath, Query, State};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use types::{PersonnelDetailDto, PersonnelItemDto, PersonnelStatsDto, UpdatePersonnelRequest};

use crate::error::ApiError;
use crate::personnel_service::PersonnelService;
use crate::response::ApiResponse;
use crate::state::AppState;

/// 分页查询参数
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonnelQuery {
    pub keyword: Option<String>,
    #[serde(default = "default_limit")]
    pub limit: u64,
    #[serde(default)]
    pub offset: u64,
}

fn default_limit() -> u64 {
    20
}

/// 人员列表响应
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonnelListResponse {
    pub items: Vec<PersonnelItemDto>,
    pub total: u64,
}

/// 组装人员底库路由
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_personnel).post(create_personnel))
        .route("/stats", get(get_personnel_stats))
        .route(
            "/{id}",
            get(get_personnel_detail)
                .put(update_personnel)
                .delete(delete_personnel),
        )
        .route("/{id}/faces", post(add_personnel_faces))
        .route("/{id}/faces/{face_id}", delete(delete_personnel_face))
        .route(
            "/{id}/faces/{face_id}/primary",
            put(set_primary_personnel_face),
        )
}

/// 分页查询在册人员列表
async fn list_personnel(
    State(state): State<AppState>,
    Query(query): Query<PersonnelQuery>,
) -> Result<ApiResponse<PersonnelListResponse>, ApiError> {
    let svc = PersonnelService::from_state(&state);
    let (items, total) = svc
        .list(query.keyword.as_deref(), query.limit, query.offset)
        .await?;
    Ok(ApiResponse::success(PersonnelListResponse { items, total }))
}

/// 获取全局底库统计数据
async fn get_personnel_stats(
    State(state): State<AppState>,
) -> Result<ApiResponse<PersonnelStatsDto>, ApiError> {
    let svc = PersonnelService::from_state(&state);
    let stats = svc.get_stats().await?;
    Ok(ApiResponse::success(stats))
}

/// 获取单个人员详细信息（含所有人脸样本）
async fn get_personnel_detail(
    State(state): State<AppState>,
    AxumPath(subject_id): AxumPath<String>,
) -> Result<ApiResponse<PersonnelDetailDto>, ApiError> {
    let svc = PersonnelService::from_state(&state);
    let detail = svc.get_detail(&subject_id).await?;
    Ok(ApiResponse::success(detail))
}

/// 一步式原子录入人员与 1~5 张人脸照片
async fn create_personnel(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<ApiResponse<PersonnelDetailDto>, ApiError> {
    let mut name = String::new();
    let mut subject_id = None;
    let mut id_card = String::new();
    let mut remark = String::new();
    let mut raw_images: Vec<Vec<u8>> = Vec::new();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("解析 Multipart 失败: {e}")))?
    {
        let field_name = field.name().unwrap_or_default().to_string();
        match field_name.as_str() {
            "name" => {
                name = field
                    .text()
                    .await
                    .map_err(|e| ApiError::BadRequest(e.to_string()))?;
            }
            "subjectId" | "subject_id" => {
                let val = field
                    .text()
                    .await
                    .map_err(|e| ApiError::BadRequest(e.to_string()))?;
                subject_id = Some(val);
            }
            "idCard" | "id_card" => {
                id_card = field
                    .text()
                    .await
                    .map_err(|e| ApiError::BadRequest(e.to_string()))?;
            }
            "remark" => {
                remark = field
                    .text()
                    .await
                    .map_err(|e| ApiError::BadRequest(e.to_string()))?;
            }
            "image" | "images" | "photo" | "photos" | "file" | "files" => {
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|e| ApiError::BadRequest(e.to_string()))?;
                if !bytes.is_empty() {
                    raw_images.push(bytes.to_vec());
                }
            }
            _ => {}
        }
    }

    let svc = PersonnelService::from_state(&state);
    let detail = svc
        .create(name, subject_id, id_card, remark, raw_images)
        .await?;
    Ok(ApiResponse::success(detail))
}

/// 更新人员基本信息
async fn update_personnel(
    State(state): State<AppState>,
    AxumPath(subject_id): AxumPath<String>,
    Json(req): Json<UpdatePersonnelRequest>,
) -> Result<ApiResponse<PersonnelDetailDto>, ApiError> {
    let svc = PersonnelService::from_state(&state);
    let detail = svc.update(&subject_id, req).await?;
    Ok(ApiResponse::success(detail))
}

/// 物理删除人员及其关联的所有样本照与数据库记录
async fn delete_personnel(
    State(state): State<AppState>,
    AxumPath(subject_id): AxumPath<String>,
) -> Result<ApiResponse<serde_json::Value>, ApiError> {
    let svc = PersonnelService::from_state(&state);
    svc.delete(&subject_id).await?;
    Ok(ApiResponse::success(serde_json::json!({
        "subjectId": subject_id,
        "deleted": true
    })))
}

/// 追加 1~N 张人脸照片（上限 5 张）
async fn add_personnel_faces(
    State(state): State<AppState>,
    AxumPath(subject_id): AxumPath<String>,
    mut multipart: Multipart,
) -> Result<ApiResponse<PersonnelDetailDto>, ApiError> {
    let mut raw_images = Vec::new();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(e.to_string()))?
    {
        let name = field.name().unwrap_or_default().to_string();
        if matches!(
            name.as_str(),
            "image" | "images" | "photo" | "photos" | "file" | "files"
        ) {
            let bytes = field
                .bytes()
                .await
                .map_err(|e| ApiError::BadRequest(e.to_string()))?;
            if !bytes.is_empty() {
                raw_images.push(bytes.to_vec());
            }
        }
    }

    let svc = PersonnelService::from_state(&state);
    let detail = svc.add_faces(&subject_id, raw_images).await?;
    Ok(ApiResponse::success(detail))
}

/// 删除单张人脸特征样本（至少保留 1 张）
async fn delete_personnel_face(
    State(state): State<AppState>,
    AxumPath((subject_id, face_id)): AxumPath<(String, String)>,
) -> Result<ApiResponse<PersonnelDetailDto>, ApiError> {
    let svc = PersonnelService::from_state(&state);
    let detail = svc.delete_face(&subject_id, &face_id).await?;
    Ok(ApiResponse::success(detail))
}

/// 设置某张人脸样本为主头像
async fn set_primary_personnel_face(
    State(state): State<AppState>,
    AxumPath((subject_id, face_id)): AxumPath<(String, String)>,
) -> Result<ApiResponse<PersonnelDetailDto>, ApiError> {
    let svc = PersonnelService::from_state(&state);
    let detail = svc.set_primary_face(&subject_id, &face_id).await?;
    Ok(ApiResponse::success(detail))
}
