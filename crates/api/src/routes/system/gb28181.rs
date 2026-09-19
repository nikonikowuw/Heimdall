use std::time::Duration;

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::Json;
use axum::Router;
use db::entity::camera::ActiveModel as CameraActiveModel;
use sea_orm::Set;
use types::{
    BatchImportGbChannelsRequest, BatchImportGbChannelsResponse, DiscoveredDevice,
    Gb28181ConfigResponse, Gb28181DeviceDto, Gb28181ServerHealth, SysGb28181Config,
    UpdateGb28181ConfigRequest,
};

use crate::error::ApiError;
use crate::response::ApiResponse;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/gb28181/config", get(get_config).put(update_config))
        .route("/gb28181/devices", get(list_devices))
        .route("/gb28181/devices/{device_id}/sync", post(sync_catalog))
        .route("/gb28181/channels/import", post(import_channels))
        .route("/discovery/scan", get(scan_discovery))
}

/// `GET /api/v1/system/gb28181/config`
/// 获取本机 SIP 服务配置与运行健康指标
pub async fn get_config(
    State(state): State<AppState>,
) -> Result<ApiResponse<Gb28181ConfigResponse>, ApiError> {
    let cfg = db::SysGb28181ConfigRepo::get(&state.db)
        .await
        .map_err(|e| ApiError::Internal(format!("获取 GB28181 配置失败: {e}")))?;

    let sip_server = state.gb28181_sip_server.clone();
    let (_, online_devs) = sip_server.device_counts();

    let total_devices = db::Gb28181DeviceRepo::list_devices_with_channels(&state.db)
        .await
        .map(|d| d.len())
        .unwrap_or(0);
    let total_devices_count = total_devices.max(online_devs);
    let active_streams_count = state.stream_hub.gb28181_active_streams();

    let health = Gb28181ServerHealth {
        running: true,
        sip_port: cfg.sip_port,
        transport: "udp+tcp".to_string(),
        online_devices_count: online_devs,
        total_devices_count,
        active_streams_count,
    };

    Ok(ApiResponse::success(Gb28181ConfigResponse {
        config: cfg,
        health,
    }))
}

/// `PUT /api/v1/system/gb28181/config`
/// 动态更新本机 SIP 服务配置
pub async fn update_config(
    State(state): State<AppState>,
    Json(req): Json<UpdateGb28181ConfigRequest>,
) -> Result<ApiResponse<SysGb28181Config>, ApiError> {
    let updated = db::SysGb28181ConfigRepo::update(&state.db, req)
        .await
        .map_err(|e| ApiError::Internal(format!("更新 GB28181 配置失败: {e}")))?;

    // 热重载内存中的 SIP 服务器配置
    state.gb28181_sip_server.update_config(updated.clone());

    Ok(ApiResponse::success(updated))
}

/// `GET /api/v1/system/gb28181/devices`
/// 列出所有向本平台注册的 GB28181 设备树及下属通道
pub async fn list_devices(
    State(state): State<AppState>,
) -> Result<ApiResponse<Vec<Gb28181DeviceDto>>, ApiError> {
    let devices = db::Gb28181DeviceRepo::list_devices_with_channels(&state.db)
        .await
        .map_err(|e| ApiError::Internal(format!("查询 GB28181 设备列表失败: {e}")))?;

    Ok(ApiResponse::success(devices))
}

/// `POST /api/v1/system/gb28181/devices/{device_id}/sync`
/// 主动触发针对某设备的 Catalog 目录树同步
pub async fn sync_catalog(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
) -> Result<ApiResponse<()>, ApiError> {
    let _dev = db::Gb28181DeviceRepo::find_by_device_id(&state.db, &device_id)
        .await
        .map_err(|e| ApiError::Internal(format!("查询目标设备失败: {e}")))?
        .ok_or_else(|| ApiError::NotFound("国标设备未注册或不存在".to_string()))?;

    state
        .gb28181_sip_server
        .sync_device_catalog(&device_id)
        .await
        .map_err(|e| ApiError::NotFound(e.to_string()))?;

    Ok(ApiResponse::success(()))
}

/// `POST /api/v1/system/gb28181/channels/import`
/// 批量将国标通道导入纳管至 cameras 表
pub async fn import_channels(
    State(state): State<AppState>,
    Json(req): Json<BatchImportGbChannelsRequest>,
) -> Result<ApiResponse<BatchImportGbChannelsResponse>, ApiError> {
    let mut imported_count = 0;
    let mut camera_ids = Vec::new();

    for item in req.channels {
        let clean_dev = item.device_id.trim();
        let clean_ch = item.channel_id.trim();
        if clean_dev.is_empty() || clean_ch.is_empty() {
            continue;
        }

        let cid = format!("gb_{clean_dev}_{clean_ch}");
        let name = item.name.unwrap_or_else(|| format!("GB-{clean_ch}"));
        let stream_mode = item.stream_mode.unwrap_or_else(|| "auto".to_string());
        let rtsp_url = format!("gb28181://{clean_dev}/{clean_ch}");

        // 检查是否已存在
        if db::CameraRepo::find_by_camera_id(&state.db, &cid)
            .await
            .map_err(|e| ApiError::Internal(format!("查询已有摄像头失败: {e}")))?
            .is_some()
        {
            camera_ids.push(cid);
            continue;
        }

        let active = CameraActiveModel {
            camera_id: Set(cid.clone()),
            name: Set(name),
            protocol: Set("gb28181".to_string()),
            rtsp_url: Set(rtsp_url),
            sub_rtsp_url: Set("".to_string()),
            stream_mode: Set(stream_mode),
            remark: Set("国标 GB28181 自动批量纳管通道".to_string()),
            last_probe_status: Set("healthy".to_string()),
            gb28181_device_id: Set(Some(clean_dev.to_string())),
            gb28181_channel_id: Set(Some(clean_ch.to_string())),
            ..Default::default()
        };

        if db::CameraRepo::insert(&state.db, active).await.is_ok() {
            imported_count += 1;
            camera_ids.push(cid);
        }
    }

    Ok(ApiResponse::success(BatchImportGbChannelsResponse {
        imported_count,
        camera_ids,
    }))
}

/// `GET /api/v1/system/discovery/scan`
/// 触发局域网摄像头主动嗅探 (ONVIF WS-Discovery)
pub async fn scan_discovery(
    State(_state): State<AppState>,
) -> Result<ApiResponse<Vec<DiscoveredDevice>>, ApiError> {
    let timeout = Duration::from_secs(3);
    let devices = media::gb28181::scan_lan_cameras(timeout)
        .await
        .map_err(|e| ApiError::Internal(format!("局域网组播扫描失败: {e}")))?;

    Ok(ApiResponse::success(devices))
}
