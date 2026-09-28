use axum::extract::{DefaultBodyLimit, Multipart, Path as AxumPath, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use types::{
    PersonnelDetailDto, PersonnelImportProgressDto, PersonnelItemDto, PersonnelStatsDto,
    ReextractProgressDto, UpdatePersonnelRequest,
};

use crate::error::ApiError;
use crate::personnel_import::manager::{MAX_IMPORT_ARCHIVE_BYTES, MAX_IMPORT_REQUEST_BYTES};
use crate::personnel_import::ImportTaskContext;
use crate::personnel_limits::{
    MAX_PERSONNEL_MULTIPART_BYTES, MAX_PERSONNEL_PHOTOS_PER_PERSON, MAX_PERSONNEL_PHOTO_BYTES,
};
use crate::personnel_maintenance::MaintenanceTaskKind;
use crate::personnel_reextract::{ReextractScope, ReextractTaskDependencies};
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

const MAX_PERSONNEL_PAGE_SIZE: u64 = 100;
const MAX_PERSONNEL_KEYWORD_CHARS: usize = 64;

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
        .route(
            "/",
            get(list_personnel)
                .post(create_personnel)
                .layer(DefaultBodyLimit::max(MAX_PERSONNEL_MULTIPART_BYTES)),
        )
        .route("/stats", get(get_personnel_stats))
        .route(
            "/import",
            post(start_personnel_import).layer(DefaultBodyLimit::max(MAX_IMPORT_REQUEST_BYTES)),
        )
        .route("/import/status", get(get_personnel_import_status))
        .route("/import/cancel", post(cancel_personnel_import))
        .route("/import/template", get(download_import_template))
        .route("/reextract", post(start_reextract_all_faces))
        .route("/reextract/status", get(get_reextract_status))
        .route(
            "/{id}",
            get(get_personnel_detail)
                .put(update_personnel)
                .delete(delete_personnel),
        )
        .route(
            "/{id}/faces",
            post(add_personnel_faces).layer(DefaultBodyLimit::max(MAX_PERSONNEL_MULTIPART_BYTES)),
        )
        .route("/{id}/faces/{face_id}", delete(delete_personnel_face))
        .route(
            "/{id}/faces/{face_id}/primary",
            put(set_primary_personnel_face),
        )
        .route("/{id}/reextract", post(reextract_single_personnel_faces))
}

/// 批量导入任务启动响应（202 Accepted 语义由业务码 + 运行态表达）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonnelImportAcceptedResponse {
    pub task_id: String,
    pub total: u64,
    pub progress: PersonnelImportProgressDto,
}

/// 分页查询在册人员列表
async fn list_personnel(
    State(state): State<AppState>,
    Query(query): Query<PersonnelQuery>,
) -> Result<ApiResponse<PersonnelListResponse>, ApiError> {
    let keyword = query
        .keyword
        .as_deref()
        .map(str::trim)
        .filter(|keyword| !keyword.is_empty());
    if keyword.is_some_and(|value| value.chars().count() > MAX_PERSONNEL_KEYWORD_CHARS) {
        return Err(ApiError::BadRequest(format!(
            "keyword 长度不能超过 {MAX_PERSONNEL_KEYWORD_CHARS} 个字符"
        )));
    }
    let svc = PersonnelService::from_state(&state);
    let (items, total) = svc
        .list(
            keyword,
            query.limit.clamp(1, MAX_PERSONNEL_PAGE_SIZE),
            query.offset,
        )
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
    let mut photo_fields = 0usize;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(map_personnel_multipart_error)?
    {
        let field_name = field.name().unwrap_or_default().to_string();
        if is_personnel_photo_field(&field_name) {
            append_personnel_photo_field(field, &mut photo_fields, &mut raw_images).await?;
            continue;
        }
        match field_name.as_str() {
            "name" => {
                name = read_personnel_text_field(field).await?;
            }
            "subjectId" | "subject_id" => {
                subject_id = Some(read_personnel_text_field(field).await?);
            }
            "idCard" | "id_card" => {
                id_card = read_personnel_text_field(field).await?;
            }
            "remark" => {
                remark = read_personnel_text_field(field).await?;
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
    let mut photo_fields = 0usize;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(map_personnel_multipart_error)?
    {
        let name = field.name().unwrap_or_default().to_string();
        if is_personnel_photo_field(&name) {
            append_personnel_photo_field(field, &mut photo_fields, &mut raw_images).await?;
        }
    }

    let svc = PersonnelService::from_state(&state);
    let detail = svc.add_faces(&subject_id, raw_images).await?;
    Ok(ApiResponse::success(detail))
}

fn is_personnel_photo_field(name: &str) -> bool {
    matches!(
        name,
        "image" | "images" | "photo" | "photos" | "file" | "files"
    )
}

async fn append_personnel_photo_field(
    field: axum::extract::multipart::Field<'_>,
    photo_fields: &mut usize,
    raw_images: &mut Vec<Vec<u8>>,
) -> Result<(), ApiError> {
    *photo_fields += 1;
    if *photo_fields > MAX_PERSONNEL_PHOTOS_PER_PERSON {
        return Err(ApiError::BadRequest(format!(
            "单个人员最多支持上传 {MAX_PERSONNEL_PHOTOS_PER_PERSON} 张人脸照片"
        )));
    }

    let bytes = read_personnel_photo_field(field).await?;
    if !bytes.is_empty() {
        raw_images.push(bytes);
    }
    Ok(())
}

async fn read_personnel_text_field(
    field: axum::extract::multipart::Field<'_>,
) -> Result<String, ApiError> {
    let bytes = field.bytes().await.map_err(map_personnel_multipart_error)?;
    std::str::from_utf8(&bytes)
        .map(str::to_owned)
        .map_err(|_| ApiError::BadRequest("人员信息字段不是有效 UTF-8".to_string()))
}

async fn read_personnel_photo_field(
    field: axum::extract::multipart::Field<'_>,
) -> Result<Vec<u8>, ApiError> {
    let bytes = field.bytes().await.map_err(map_personnel_multipart_error)?;
    if bytes.len() > MAX_PERSONNEL_PHOTO_BYTES {
        return Err(ApiError::BadRequest(format!(
            "单张人脸照片不能超过 {} MiB",
            MAX_PERSONNEL_PHOTO_BYTES / (1024 * 1024)
        )));
    }
    Ok(bytes.to_vec())
}

fn map_personnel_multipart_error(error: axum::extract::multipart::MultipartError) -> ApiError {
    if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        ApiError::BadRequest(format!(
            "人员照片表单总大小不能超过 {} MiB",
            MAX_PERSONNEL_MULTIPART_BYTES / (1024 * 1024)
        ))
    } else {
        ApiError::BadRequest("人员上传表单格式无效".to_string())
    }
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

/// 启动全量底库人脸特征后台异步重新提取任务
async fn start_reextract_all_faces(
    State(state): State<AppState>,
) -> Result<ApiResponse<ReextractProgressDto>, ApiError> {
    let evidence_base_dir = state.base_evidence_dir();
    let maintenance_guard = state
        .maintenance_gate
        .acquire(MaintenanceTaskKind::Reextract)?;
    let initial_progress = state
        .reextract_manager
        .start_task(
            ReextractScope::AllFaces,
            ReextractTaskDependencies {
                db: state.db.clone(),
                evidence_base_dir,
                algo_registry: state.algo_registry.clone(),
                gallery_index: state.gallery_index.clone(),
                maintenance_guard,
                event_broadcaster: state.event_broadcaster.clone(),
            },
        )
        .await?;
    Ok(ApiResponse::success(initial_progress))
}

/// 查询后台人脸特征重新提取任务的实时进度与状态
async fn get_reextract_status(
    State(state): State<AppState>,
) -> Result<ApiResponse<ReextractProgressDto>, ApiError> {
    let progress = state.reextract_manager.get_progress().await;
    Ok(ApiResponse::success(progress))
}

/// 针对单个人员重新提取其所有人脸样本特征
///
/// 走与全量重提相同的后台任务模型：单体在 RK3568 这类单核 NPU 上逐个样本串行提取，
/// 挂在一个 HTTP 请求里既会与常驻视频流推理争抢时间片，也会因超时/客户端失联
/// 丢掉整批进度。调用方改为读 `/reextract/status` 或订阅 WS 终态主题。
async fn reextract_single_personnel_faces(
    State(state): State<AppState>,
    AxumPath(subject_id): AxumPath<String>,
) -> Result<ApiResponse<ReextractProgressDto>, ApiError> {
    let evidence_base_dir = state.base_evidence_dir();
    let maintenance_guard = state
        .maintenance_gate
        .acquire(MaintenanceTaskKind::Reextract)?;
    let initial_progress = state
        .reextract_manager
        .start_task(
            ReextractScope::Subject(subject_id.trim().to_string()),
            ReextractTaskDependencies {
                db: state.db.clone(),
                evidence_base_dir,
                algo_registry: state.algo_registry.clone(),
                gallery_index: state.gallery_index.clone(),
                maintenance_guard,
                event_broadcaster: state.event_broadcaster.clone(),
            },
        )
        .await?;
    Ok(ApiResponse::success(initial_progress))
}

// ============================================================================
// 人员批量建档与导入
// ============================================================================

/// 归档流式落盘的临时文件上限说明
fn import_size_limit_error() -> ApiError {
    let max_mb = MAX_IMPORT_ARCHIVE_BYTES / (1024 * 1024);
    ApiError::BadRequest(format!(
        "导入归档体积超出上限 ({max_mb} MB)，请拆分后分批导入"
    ))
}

/// 上传归档并启动人员批量导入后台任务
///
/// 归档先流式落盘到临时文件（不占内存），再在专用阻塞线程内解压解析；
/// 解析通过后立即启动串行后台 Worker 并返回初始进度，客户端据 taskId 轮询或订阅 WS。
async fn start_personnel_import(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Response, ApiError> {
    // 先占闸门再解压：被拒绝的请求不做任何解压/解析开销，也不留下临时沙箱
    let maintenance_guard = state
        .maintenance_gate
        .acquire(MaintenanceTaskKind::Import)?;

    let mut archive_guard: Option<crate::algo::archive::TempFileGuard> = None;
    let mut archive_name: Option<String> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(map_import_multipart_error)?
    {
        let name = field.name().unwrap_or_default().to_string();
        if !matches!(name.as_str(), "file" | "archive" | "package") {
            continue;
        }
        if archive_guard.is_some() {
            return Err(ApiError::BadRequest(
                "每次请求只能上传一个人员导入归档".to_string(),
            ));
        }

        if let Some(file_name) = field.file_name() {
            archive_name = Some(file_name.to_string());
        }
        archive_guard = Some(write_import_archive_to_temp(field).await?);
    }

    let Some(archive_guard) = archive_guard else {
        return Err(ApiError::BadRequest(
            "未找到有效的导入归档文件流".to_string(),
        ));
    };

    let task_id = uuid::Uuid::now_v7().to_string();
    // 解析阶段结束即回收上传归档（守卫析构），后续只依赖已解压沙箱内容
    let (sandbox, candidates) = state
        .import_manager
        .prepare_candidates(
            archive_guard.path().to_path_buf(),
            archive_name,
            task_id.clone(),
        )
        .await?;
    drop(archive_guard);

    if candidates.is_empty() {
        return Err(ApiError::BadRequest(
            "导入包中未解析出任何有效人员记录".to_string(),
        ));
    }

    let total = candidates.len() as u64;
    let evidence_base_dir = state.base_evidence_dir();

    let progress = state
        .import_manager
        .start_task(
            task_id.clone(),
            sandbox,
            candidates,
            ImportTaskContext {
                db: state.db.clone(),
                evidence_base_dir,
                algo_registry: state.algo_registry.clone(),
                gallery_index: state.gallery_index.clone(),
                maintenance_guard,
                event_broadcaster: state.event_broadcaster.clone(),
            },
        )
        .await?;

    Ok((
        StatusCode::ACCEPTED,
        ApiResponse::success(PersonnelImportAcceptedResponse {
            task_id,
            total,
            progress,
        }),
    )
        .into_response())
}

/// 查询当前或最近一次导入任务的进度与报告
async fn get_personnel_import_status(
    State(state): State<AppState>,
) -> Result<ApiResponse<PersonnelImportProgressDto>, ApiError> {
    Ok(ApiResponse::success(
        state.import_manager.get_progress().await,
    ))
}

/// 中止正在执行的人员批量导入任务
async fn cancel_personnel_import(
    State(state): State<AppState>,
) -> Result<ApiResponse<PersonnelImportProgressDto>, ApiError> {
    let progress = state.import_manager.request_cancel().await?;
    Ok(ApiResponse::success(progress))
}

/// 下载标准人员清单模板
///
/// 带 UTF-8 BOM，避免 Windows Excel 直接打开时中文乱码；示例行覆盖多图引用写法。
async fn download_import_template() -> Result<Response, ApiError> {
    const TEMPLATE: &str = "name,subjectId,idCard,remark,photos\n张三,EMP001,110101199001010011,安保部,zhang_01.jpg;zhang_02.jpg\n李四,,,巡检组,\n";
    let mut body = Vec::with_capacity(TEMPLATE.len() + 3);
    body.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    body.extend_from_slice(TEMPLATE.as_bytes());

    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"personnel_import_template.csv\"",
            ),
        ],
        body,
    )
        .into_response())
}

/// 流式写入导入归档到临时文件，超限立即中止并返回 400
///
/// 返回 `TempFileGuard` 而非裸路径：调用方负责在解析结束后释放临时归档，
/// 错误路径下守卫随作用域析构，不留下 .part 残留。
async fn write_import_archive_to_temp(
    mut field: axum::extract::multipart::Field<'_>,
) -> Result<crate::algo::archive::TempFileGuard, ApiError> {
    use tokio::io::AsyncWriteExt;

    let path = std::env::temp_dir().join(format!(
        "heimdall_personnel_import_{}.part",
        uuid::Uuid::now_v7().simple()
    ));
    let guard = crate::algo::archive::TempFileGuard::new(path.clone());

    let result = async {
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await
            .map_err(|err| ApiError::Internal(format!("创建导入临时归档失败: {err}")))?;
        let mut total = 0usize;

        while let Some(chunk) = field.chunk().await.map_err(map_import_multipart_error)? {
            total = total
                .checked_add(chunk.len())
                .ok_or_else(import_size_limit_error)?;
            if total > MAX_IMPORT_ARCHIVE_BYTES {
                return Err(import_size_limit_error());
            }
            file.write_all(&chunk)
                .await
                .map_err(|err| ApiError::Internal(format!("写入导入临时归档失败: {err}")))?;
        }

        file.flush()
            .await
            .map_err(|err| ApiError::Internal(format!("刷新导入临时归档失败: {err}")))?;

        if total == 0 {
            return Err(ApiError::BadRequest("导入归档为空".to_string()));
        }
        Ok(())
    }
    .await;

    match result {
        Ok(()) => Ok(guard),
        Err(err) => Err(err),
    }
}

/// 归一化 Multipart 读取错误，识别体积超限
fn map_import_multipart_error(error: axum::extract::multipart::MultipartError) -> ApiError {
    if error.status() == axum::http::StatusCode::PAYLOAD_TOO_LARGE {
        import_size_limit_error()
    } else {
        ApiError::BadRequest(format!("解析导入归档失败: {error}"))
    }
}
