use std::time::Duration;

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use sea_orm::Set;
use types::{Camera, CodecType, CreateCameraRequest, ProbeResult, UpdateCameraRequest};

use crate::camera_probe::CameraProbeService;
use crate::error::ApiError;
use crate::response::ApiResponse;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_cameras).post(create_camera))
        .route("/deduce-substream", post(deduce_substream))
        .route(
            "/{cameraId}",
            get(get_camera).put(update_camera).delete(delete_camera),
        )
        .route("/{cameraId}/probe", post(probe_camera_manual))
}

/// 实体转领域模型 DTO
fn model_to_camera_dto(m: db::entity::camera::Model) -> Camera {
    Camera {
        id: m.id,
        camera_id: m.camera_id,
        name: m.name,
        protocol: m.protocol,
        rtsp_url: m.rtsp_url,
        sub_rtsp_url: m.sub_rtsp_url,
        stream_mode: types::StreamMode::from_str_loose(&m.stream_mode),
        remark: m.remark,
        transport_policy: types::TransportPolicy::Auto,
        last_probe_status: types::ProbeStatus::from_str_loose(&m.last_probe_status),
        last_probe_at: m.last_probe_at.map(|dt| dt.timestamp_millis()),
        last_probe_error_code: m.last_probe_error_code,
        last_success_at: m.last_success_at.map(|dt| dt.timestamp_millis()),
        last_codec: m.last_codec,
        last_width: m.last_width as u32,
        last_height: m.last_height as u32,
        last_fps: m.last_fps,
        gb28181_device_id: m.gb28181_device_id,
        gb28181_channel_id: m.gb28181_channel_id,
        created_at: m.created_at.timestamp_millis(),
        updated_at: m.updated_at.timestamp_millis(),
    }
}

/// 获取所有摄像头视频源列表
async fn list_cameras(State(state): State<AppState>) -> Result<ApiResponse<Vec<Camera>>, ApiError> {
    let list = db::CameraRepo::list_all(&state.db).await?;
    let dtos = list.into_iter().map(model_to_camera_dto).collect();
    Ok(ApiResponse::success(dtos))
}

/// 获取单路摄像头视频源详情
async fn get_camera(
    State(state): State<AppState>,
    Path(camera_id): Path<String>,
) -> Result<ApiResponse<Camera>, ApiError> {
    let camera = db::CameraRepo::find_by_camera_id(&state.db, &camera_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("摄像头未找到: {camera_id}")))?;
    Ok(ApiResponse::success(model_to_camera_dto(camera)))
}

/// 新增摄像头视频源并异步触发首次探活
async fn create_camera(
    State(state): State<AppState>,
    Json(req): Json<CreateCameraRequest>,
) -> Result<ApiResponse<Camera>, ApiError> {
    let name = req.name.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest("摄像头名称不能为空".to_string()));
    }
    let rtsp_url = req.rtsp_url.trim();
    if rtsp_url.is_empty() {
        return Err(ApiError::BadRequest("RTSP 地址不能为空".to_string()));
    }

    let camera_id = uuid::Uuid::new_v4().to_string();
    let protocol = req
        .protocol
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| "rtsp".to_string());

    // 若请求显式提供了 sub_rtsp_url（包含传空字符串表示明确使用单码流模式），则遵从用户设定；
    // 仅在完全未提供该字段 (None) 时才尝试自动推导候选子码流
    let sub_rtsp_url = match req.sub_rtsp_url {
        Some(ref s) => s.trim().to_string(),
        None => media::deduce_primary_sub_stream(rtsp_url).unwrap_or_default(),
    };

    let stream_mode = req
        .stream_mode
        .unwrap_or(types::StreamMode::Auto)
        .as_str()
        .to_string();

    let active_model = db::entity::camera::ActiveModel {
        camera_id: Set(camera_id.clone()),
        name: Set(name.to_string()),
        protocol: Set(protocol),
        rtsp_url: Set(rtsp_url.to_string()),
        sub_rtsp_url: Set(sub_rtsp_url),
        stream_mode: Set(stream_mode),
        remark: Set(req.remark.unwrap_or_default()),
        last_probe_status: Set("never".to_string()),
        last_probe_at: Set(None),
        last_probe_error_code: Set(String::new()),
        last_success_at: Set(None),
        last_codec: Set(String::new()),
        last_width: Set(0),
        last_height: Set(0),
        last_fps: Set(0.0),
        gb28181_device_id: Set(req.gb28181_device_id),
        gb28181_channel_id: Set(req.gb28181_channel_id),
        created_at: Set(chrono::Utc::now()),
        updated_at: Set(chrono::Utc::now()),
        ..Default::default()
    };

    let inserted = db::CameraRepo::insert(&state.db, active_model).await?;

    // 异步触发一次轻量探活
    CameraProbeService::spawn_probe_and_broadcast(
        state.db.clone(),
        state.event_broadcaster.clone(),
        camera_id,
        rtsp_url.to_string(),
    );

    Ok(ApiResponse::success(model_to_camera_dto(inserted)))
}

/// 修改摄像头基础配置
async fn update_camera(
    State(state): State<AppState>,
    Path(camera_id): Path<String>,
    Json(req): Json<UpdateCameraRequest>,
) -> Result<ApiResponse<Camera>, ApiError> {
    let camera = db::CameraRepo::find_by_camera_id(&state.db, &camera_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("摄像头未找到: {camera_id}")))?;

    let mut active: db::entity::camera::ActiveModel = camera.clone().into();
    let mut rtsp_url_changed = false;
    let mut new_url = String::new();

    if let Some(name) = req.name {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            active.name = Set(trimmed.to_string());
        }
    }
    if let Some(url) = req.rtsp_url {
        let trimmed = url.trim();
        if !trimmed.is_empty() && trimmed != camera.rtsp_url {
            active.rtsp_url = Set(trimmed.to_string());
            rtsp_url_changed = true;
            new_url = trimmed.to_string();
        }
    }
    let mut stream_mode_changed = false;
    let mut sub_rtsp_changed = false;

    if let Some(sub_url) = req.sub_rtsp_url {
        let trimmed = sub_url.trim();
        if trimmed != camera.sub_rtsp_url {
            active.sub_rtsp_url = Set(trimmed.to_string());
            sub_rtsp_changed = true;
        }
    }
    if let Some(mode) = req.stream_mode {
        if mode.as_str() != camera.stream_mode {
            active.stream_mode = Set(mode.as_str().to_string());
            stream_mode_changed = true;
        }
    }
    if let Some(remark) = req.remark {
        active.remark = Set(remark);
    }
    if let Some(dev_id) = req.gb28181_device_id {
        active.gb28181_device_id = Set(Some(dev_id));
    }
    if let Some(ch_id) = req.gb28181_channel_id {
        active.gb28181_channel_id = Set(Some(ch_id));
    }
    active.updated_at = Set(chrono::Utc::now());

    let updated = sea_orm::ActiveModelTrait::update(active, &state.db)
        .await
        .map_err(db::DbError::from)?;

    // 若 RTSP 地址变更，移除旧流会话并异步重新探活
    if rtsp_url_changed {
        state.stream_hub.remove_session(&camera_id).await;
        CameraProbeService::spawn_probe_and_broadcast(
            state.db.clone(),
            state.event_broadcaster.clone(),
            camera_id.clone(),
            new_url,
        );
    }

    // 若 RTSP 地址或码流模式变更，联动同步已启用的 AI 分析管线
    if rtsp_url_changed || stream_mode_changed || sub_rtsp_changed {
        if let Err(err) = crate::routes::task::sync_pipeline_for_camera(&state, &camera_id).await {
            tracing::warn!(
                camera_id = %camera_id,
                error = %err,
                "摄像头码流配置变更后同步分析管线失败"
            );
        }
    }

    Ok(ApiResponse::success(model_to_camera_dto(updated)))
}

/// 删除摄像头视频源
async fn delete_camera(
    State(state): State<AppState>,
    Path(camera_id): Path<String>,
) -> Result<ApiResponse<()>, ApiError> {
    // 先确认资源存在，避免对不存在摄像头触发运行时副作用。
    db::CameraRepo::find_by_camera_id(&state.db, &camera_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("摄像头未找到: {camera_id}")))?;

    // 摄像头删除必须先停止 coordinator 管理的完整双流运行时。
    state
        .task_coordinator
        .stop_camera_pipeline(&camera_id)
        .await
        .map_err(ApiError::Coordinator)?;

    if let Err(err) = state.pipeline.stop_task(&camera_id).await {
        if !matches!(err, pipeline::PipelineError::PipelineNotFound { .. }) {
            return Err(ApiError::Pipeline(err));
        }
    }
    state
        .task_coordinator
        .set_camera_rules(&camera_id, Vec::new())
        .await;
    state.pipeline.set_ai_active(&camera_id, false).await;
    state.stream_hub.remove_session(&camera_id).await;

    // 任务及其主算法实例不能随着摄像头删除而成为孤儿记录。
    db::TaskRepo::delete_task_and_instance(&state.db, &camera_id).await?;
    let rows = db::CameraRepo::delete_by_camera_id(&state.db, &camera_id).await?;
    if rows == 0 {
        return Err(ApiError::NotFound(format!("摄像头未找到: {camera_id}")));
    }

    Ok(ApiResponse::success(()))
}

/// 手动触发单次探活
async fn probe_camera_manual(
    State(state): State<AppState>,
    Path(camera_id): Path<String>,
) -> Result<ApiResponse<ProbeResult>, ApiError> {
    let camera = db::CameraRepo::find_by_camera_id(&state.db, &camera_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("摄像头未找到: {camera_id}")))?;

    tracing::info!(camera_id = %camera_id, rtsp_url = %media::mask_rtsp_url(&camera.rtsp_url), "收到手动探活请求");

    // 1. 若当前摄像头已有活跃流会话，优先复用已缓存的关键帧参数集 (0 额外网络开销)
    let cached_info = {
        let mut found = None;
        for key in [
            &format!("{camera_id}:main"),
            &format!("{camera_id}:sub"),
            &camera.rtsp_url,
        ] {
            if let Some(cache) = state.stream_hub.get_keyframe_cache(key).await {
                if let Some(sps) = &cache.sps {
                    let info = match cache.codec {
                        Some(CodecType::H265) => media::sps::parse_h265_sps(sps).ok(),
                        Some(CodecType::H264) => media::sps::parse_h264_sps(sps).ok(),
                        _ => None,
                    };
                    if let Some(sps_info) = info {
                        if sps_info.width > 0 && sps_info.height > 0 {
                            found = Some(media::StreamInfo {
                                codec: sps_info.codec,
                                width: sps_info.width,
                                height: sps_info.height,
                                fps: sps_info.fps,
                            });
                            break;
                        }
                    }
                }
            }
        }
        found
    };

    let probe_res = match cached_info {
        Some(info) => Ok(info),
        None => media::StreamProber::probe(&camera.rtsp_url, Duration::from_secs(5)).await,
    };

    match probe_res {
        Ok(probe_info) => {
            tracing::info!(
                camera_id = %camera_id,
                codec = %probe_info.codec,
                width = probe_info.width,
                height = probe_info.height,
                fps = probe_info.fps,
                "手动探活成功 -> 标记为 healthy"
            );
            CameraProbeService::broadcast_probe_update(
                &state.db,
                &state.event_broadcaster,
                &camera_id,
                db::ProbeUpdateParams {
                    status: "healthy",
                    codec: &probe_info.codec,
                    width: probe_info.width as i32,
                    height: probe_info.height as i32,
                    fps: probe_info.fps,
                    error_code: "",
                },
            )
            .await;

            Ok(ApiResponse::success(ProbeResult {
                codec: probe_info.codec,
                width: probe_info.width,
                height: probe_info.height,
                fps: probe_info.fps,
            }))
        }
        Err(e) => {
            let err_str = e.to_string();
            tracing::warn!(camera_id = %camera_id, error = %err_str, "手动探活失败 -> 标记为 failed");
            CameraProbeService::broadcast_probe_update(
                &state.db,
                &state.event_broadcaster,
                &camera_id,
                db::ProbeUpdateParams {
                    status: "failed",
                    codec: "",
                    width: 0,
                    height: 0,
                    fps: 0.0,
                    error_code: &err_str,
                },
            )
            .await;

            Err(ApiError::Media(e))
        }
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeduceSubStreamRequest {
    pub rtsp_url: String,
}

/// 自动推导子码流候选地址
async fn deduce_substream(
    Json(req): Json<DeduceSubStreamRequest>,
) -> ApiResponse<Vec<media::SubStreamCandidate>> {
    let list = media::deduce_sub_stream(&req.rtsp_url);
    ApiResponse::success(list)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    async fn setup_test_app() -> (axum::Router, AppState, String) {
        let db = db::init_test_db().await.unwrap();
        let pipeline = std::sync::Arc::new(pipeline::PipelineManager::new());
        let state = AppState::new(db, pipeline);
        crate::sync_auth_state(&state).await;

        // 初始化管理员并获得 token
        let password_hash =
            crate::crypto::hash_password_async("adminPassword123".to_string()).await;
        db::AdminUserRepo::create_admin(&state.db, "admin", &password_hash)
            .await
            .unwrap();
        state
            .is_initialized
            .store(true, std::sync::atomic::Ordering::Relaxed);

        let claims = types::AuthClaims {
            sub: "admin".to_string(),
            iat: chrono::Utc::now().timestamp_millis(),
            exp: chrono::Utc::now().timestamp_millis() + 86400000,
        };
        let token = crate::crypto::generate_jwt(&claims, &state.get_jwt_secret()).unwrap();

        let app = crate::create_app(state.clone());
        (app, state, token)
    }

    #[test]
    fn test_model_to_camera_dto() {
        let model = db::entity::camera::Model {
            id: 1,
            camera_id: "cam-101".to_string(),
            name: "Test Cam".to_string(),
            protocol: "rtsp".to_string(),
            rtsp_url: "rtsp://127.0.0.1:8554/live".to_string(),
            sub_rtsp_url: "".to_string(),
            stream_mode: "auto".to_string(),
            remark: "Entrance".to_string(),
            last_probe_status: "healthy".to_string(),
            last_probe_at: None,
            last_probe_error_code: "".to_string(),
            last_success_at: None,
            last_codec: "h264".to_string(),
            last_width: 1920,
            last_height: 1080,
            last_fps: 25.0,
            gb28181_device_id: None,
            gb28181_channel_id: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let dto = model_to_camera_dto(model);
        assert_eq!(dto.camera_id, "cam-101");
        assert_eq!(dto.last_probe_status, types::ProbeStatus::Healthy);
        assert_eq!(dto.last_width, 1920);
    }

    #[tokio::test]
    async fn test_camera_crud_lifecycle() {
        let (app, state, token) = setup_test_app().await;

        // 1. 获取列表为空
        let req = Request::builder()
            .uri("/api/v1/cameras")
            .method("GET")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let list_res: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(list_res["code"], 0);
        assert_eq!(list_res["data"].as_array().unwrap().len(), 0);

        // 2. 创建摄像头
        let create_body = serde_json::json!({
            "name": "East Gate Camera",
            "rtspUrl": "rtsp://192.168.1.200:554/live/ch0",
            "remark": "Main entrance"
        });
        let req = Request::builder()
            .uri("/api/v1/cameras")
            .method("POST")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&create_body).unwrap()))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let created_res: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(created_res["code"], 0);
        let cam_id = created_res["data"]["cameraId"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(created_res["data"]["name"], "East Gate Camera");

        // 3. 查询单条摄像头
        let req = Request::builder()
            .uri(format!("/api/v1/cameras/{cam_id}"))
            .method("GET")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let get_res: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(get_res["data"]["cameraId"], cam_id);

        // 4. 更新摄像头
        let update_body = serde_json::json!({
            "name": "East Gate Camera Updated",
            "remark": "Updated remark"
        });
        let req = Request::builder()
            .uri(format!("/api/v1/cameras/{cam_id}"))
            .method("PUT")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&update_body).unwrap()))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let update_res: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(update_res["data"]["name"], "East Gate Camera Updated");
        assert_eq!(update_res["data"]["remark"], "Updated remark");

        // 5. 校验审计日志中已记录 create 与 update 操作。
        // 审计写入在 middleware 中异步执行，只等待有限时间避免测试依赖调度时序。
        let mut logs = Vec::new();
        for _ in 0..100 {
            logs = db::OplogRepo::list_recent(&state.db, Some("camera"), 10, 0)
                .await
                .unwrap();
            let has_create = logs
                .iter()
                .any(|log| log.action == "create" && log.module == "camera");
            let has_update = logs
                .iter()
                .any(|log| log.action == "update" && log.module == "camera");
            if has_create && has_update {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        assert!(logs
            .iter()
            .any(|log| log.action == "create" && log.module == "camera"));
        assert!(logs
            .iter()
            .any(|log| log.action == "update" && log.module == "camera"));

        // 6. 删除摄像头
        let req = Request::builder()
            .uri(format!("/api/v1/cameras/{cam_id}"))
            .method("DELETE")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        // 7. 再次查询为 404
        let req = Request::builder()
            .uri(format!("/api/v1/cameras/{cam_id}"))
            .method("GET")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_camera_explicit_empty_sub_stream_preserved_for_single_stream_mode() {
        let (app, _state, token) = setup_test_app().await;

        // 包含海康匹配模式的 URL，但用户明确显式传空字符串表示无子码流（单码流）
        let create_body = serde_json::json!({
            "name": "Hik Single Stream Camera",
            "rtspUrl": "rtsp://admin:12345@192.168.1.64:554/Streaming/Channels/101",
            "subRtspUrl": ""
        });
        let req = Request::builder()
            .uri("/api/v1/cameras")
            .method("POST")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&create_body).unwrap()))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let created_res: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(created_res["code"], 0);
        // 验证用户显式设置的空字符串得到尊重，未被推导覆盖
        assert_eq!(created_res["data"]["subRtspUrl"], "");

        // 仅在未提供 subRtspUrl 字段时，才自动推导建议
        let create_body_auto = serde_json::json!({
            "name": "Hik Auto Deduce Camera",
            "rtspUrl": "rtsp://admin:12345@192.168.1.64:554/Streaming/Channels/101"
        });
        let req_auto = Request::builder()
            .uri("/api/v1/cameras")
            .method("POST")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&create_body_auto).unwrap()))
            .unwrap();
        let res_auto = app.clone().oneshot(req_auto).await.unwrap();
        assert_eq!(res_auto.status(), StatusCode::OK);
        let bytes_auto = axum::body::to_bytes(res_auto.into_body(), usize::MAX)
            .await
            .unwrap();
        let created_res_auto: serde_json::Value = serde_json::from_slice(&bytes_auto).unwrap();
        assert_eq!(
            created_res_auto["data"]["subRtspUrl"],
            "rtsp://admin:12345@192.168.1.64:554/Streaming/Channels/102"
        );
    }

    #[tokio::test]
    async fn test_camera_update_stream_mode_invokes_pipeline_sync() {
        let (app, state, token) = setup_test_app().await;

        // 1. 创建摄像头
        let cam_model = db::entity::camera::ActiveModel {
            id: sea_orm::ActiveValue::NotSet,
            camera_id: sea_orm::ActiveValue::Set("CAM-SYNC-TEST".to_string()),
            name: sea_orm::ActiveValue::Set("Sync Cam".to_string()),
            protocol: sea_orm::ActiveValue::Set("rtsp".to_string()),
            rtsp_url: sea_orm::ActiveValue::Set("rtsp://127.0.0.1:8554/live".to_string()),
            sub_rtsp_url: sea_orm::ActiveValue::Set("rtsp://127.0.0.1:8554/sub".to_string()),
            stream_mode: sea_orm::ActiveValue::Set("auto".to_string()),
            remark: sea_orm::ActiveValue::Set("".to_string()),
            last_probe_status: sea_orm::ActiveValue::Set("healthy".to_string()),
            last_probe_at: sea_orm::ActiveValue::Set(None),
            last_probe_error_code: sea_orm::ActiveValue::Set("".to_string()),
            last_success_at: sea_orm::ActiveValue::Set(None),
            last_codec: sea_orm::ActiveValue::Set("h264".to_string()),
            last_width: sea_orm::ActiveValue::Set(1920),
            last_height: sea_orm::ActiveValue::Set(1080),
            last_fps: sea_orm::ActiveValue::Set(25.0),
            gb28181_device_id: sea_orm::ActiveValue::Set(None),
            gb28181_channel_id: sea_orm::ActiveValue::Set(None),
            created_at: sea_orm::ActiveValue::Set(chrono::Utc::now()),
            updated_at: sea_orm::ActiveValue::Set(chrono::Utc::now()),
        };
        db::CameraRepo::insert(&state.db, cam_model).await.unwrap();

        // 2. 更新摄像头 stream_mode 为 main
        let update_body = serde_json::json!({
            "streamMode": "main"
        });
        let req = Request::builder()
            .uri("/api/v1/cameras/CAM-SYNC-TEST")
            .method("PUT")
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&update_body).unwrap()))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);

        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["code"], 0);
        assert_eq!(json["data"]["streamMode"], "main");

        // 3. 验证数据库中 stream_mode 已成功变为 main
        let reloaded = db::CameraRepo::find_by_camera_id(&state.db, "CAM-SYNC-TEST")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reloaded.stream_mode, "main");
    }
}
