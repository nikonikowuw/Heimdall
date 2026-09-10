use std::sync::Arc;

use infer::package::AlgoRegistry;
use pipeline::{InstanceLaunchConfig, StartCameraPipelineParams};
use types::{CodecType, MotionGateConfig, TransportPolicy};

use crate::error::ApiError;

/// Resolve the algorithm used by a task update.
///
/// Explicit bindings are kept verbatim after trimming. A disabled task keeps its
/// existing binding, while an enabled task may select only a package that is
/// both registered for runtime use and present in the database.
pub async fn resolve_algorithm_id(
    db: &db::DatabaseConnection,
    registry: &Arc<AlgoRegistry>,
    camera_id: &str,
    requested_algorithm_id: &str,
    desired_enabled: bool,
) -> Result<String, ApiError> {
    let requested = requested_algorithm_id.trim();
    if !requested.is_empty() {
        return Ok(requested.to_string());
    }

    let existing_instances = db::AlgorithmInstanceRepo::list_by_camera_id(db, camera_id).await?;
    if !desired_enabled {
        return Ok(existing_instances
            .first()
            .map(|inst| inst.algorithm_id.clone())
            .unwrap_or_default());
    }

    let mut candidates = Vec::new();
    if registry.contains("general_detection").await {
        candidates.push("general_detection".to_string());
    }

    let mut manifests = registry.list().await;
    manifests.sort_by(|left, right| left.algorithm_id.cmp(&right.algorithm_id));
    candidates.extend(
        manifests
            .into_iter()
            .map(|manifest| manifest.algorithm_id)
            .filter(|algorithm_id| algorithm_id != "general_detection"),
    );

    for algorithm_id in candidates {
        if db::AlgorithmRepo::find_by_algorithm_id(db, &algorithm_id)
            .await?
            .is_some()
        {
            return Ok(algorithm_id);
        }
    }

    Err(ApiError::BadRequest("未配置可用分析算法包".to_string()))
}

/// Convert the persisted camera stream configuration into coordinator input with multi-algorithm instances.
pub fn build_start_params(
    camera_id: &str,
    camera: &db::entity::camera::Model,
    instances: Vec<InstanceLaunchConfig>,
    motion_gate: Option<&MotionGateConfig>,
) -> StartCameraPipelineParams {
    let main_rtsp_url = camera.rtsp_url.trim().to_string();
    let sub_rtsp_url = if !camera.sub_rtsp_url.trim().is_empty() {
        camera.sub_rtsp_url.trim().to_string()
    } else if let Some(deduced) = media::deduce_primary_sub_stream(&main_rtsp_url) {
        deduced
    } else {
        main_rtsp_url.clone()
    };

    let codec = match camera.last_codec.to_lowercase() {
        value if value.contains("265") || value.contains("hevc") => CodecType::H265,
        _ => CodecType::H264,
    };

    StartCameraPipelineParams {
        camera_id: camera_id.to_string(),
        main_rtsp_url,
        main_codec: codec,
        sub_rtsp_url,
        sub_codec: codec,
        transport_policy: TransportPolicy::Auto,
        instances,
        motion_gate_enabled: motion_gate.map(|config| config.enabled).unwrap_or(true),
    }
}

/// Helper for single-instance pipeline parameters.
pub fn build_single_start_params(
    camera_id: &str,
    camera: &db::entity::camera::Model,
    algorithm_id: String,
    algo_params: serde_json::Value,
    analysis_fps: i32,
    motion_gate: Option<&MotionGateConfig>,
) -> StartCameraPipelineParams {
    let target_fps = if analysis_fps > 0 {
        analysis_fps as u32
    } else {
        10
    };
    build_start_params(
        camera_id,
        camera,
        vec![InstanceLaunchConfig {
            algorithm_id,
            algo_params,
            target_fps,
        }],
        motion_gate,
    )
}
