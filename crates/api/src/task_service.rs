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

fn parse_codec_str(codec_str: &str) -> CodecType {
    match codec_str.to_lowercase() {
        value if value.contains("265") || value.contains("hevc") => CodecType::H265,
        _ => CodecType::H264,
    }
}

fn resolve_candidate_sub_url(configured_trimmed: &str, main_trimmed: &str) -> Option<String> {
    if !configured_trimmed.is_empty() {
        Some(configured_trimmed.to_string())
    } else {
        media::deduce_primary_sub_stream(main_trimmed)
    }
}

/// Convert the persisted camera stream configuration into coordinator input with multi-algorithm instances (synchronous without probe).
pub fn build_start_params(
    camera_id: &str,
    camera: &db::entity::camera::Model,
    instances: Vec<InstanceLaunchConfig>,
    motion_gate: Option<&MotionGateConfig>,
) -> StartCameraPipelineParams {
    let main_rtsp_url = camera.rtsp_url.trim().to_string();
    let stream_mode = types::StreamMode::from_str_loose(&camera.stream_mode);
    let main_codec = parse_codec_str(&camera.last_codec);
    let (sub_rtsp_url, sub_codec) = match stream_mode {
        types::StreamMode::Main => (main_rtsp_url.clone(), main_codec),
        types::StreamMode::Sub | types::StreamMode::Auto => {
            let sub_url = resolve_candidate_sub_url(camera.sub_rtsp_url.trim(), &main_rtsp_url)
                .unwrap_or_else(|| main_rtsp_url.clone());
            (sub_url, main_codec)
        }
    };

    StartCameraPipelineParams {
        camera_id: camera_id.to_string(),
        main_rtsp_url,
        main_codec,
        sub_rtsp_url,
        sub_codec,
        transport_policy: TransportPolicy::Auto,
        instances,
        motion_gate_enabled: motion_gate.map(|config| config.enabled).unwrap_or(true),
    }
}

/// 自适应探测并决议适用的分析流 URL 与对应编解码格式：
/// - StreamMode::Main: 显式指定主码流，直接向算法包下发主码流原生帧；算法包自行完成模型所需预处理；
/// - StreamMode::Sub: 显式指定子码流，使用配置或推导的子码流（超高路数并发低功耗），同时探活获取子流真实编码；
/// - StreamMode::Auto: 自动探测协商，探活成功用子流及其实际编码，探活失败或无子流自适应回退到主流。
pub async fn resolve_effective_sub_stream(
    camera_id: &str,
    main_rtsp_url: &str,
    main_codec: CodecType,
    configured_sub_url: &str,
    stream_mode: types::StreamMode,
    probe_timeout: std::time::Duration,
) -> (String, CodecType) {
    let main_trimmed = main_rtsp_url.trim();
    let configured_trimmed = configured_sub_url.trim();

    match stream_mode {
        types::StreamMode::Main => {
            tracing::info!(
                camera_id = %camera_id,
                "用户指定主码流分析模式 (StreamMode::Main)，常驻硬解主码流并将原生帧直接下发至算法包"
            );
            (main_trimmed.to_string(), main_codec)
        }
        types::StreamMode::Sub => {
            let candidate = resolve_candidate_sub_url(configured_trimmed, main_trimmed);

            if let Some(candidate_url) = candidate {
                let sub_codec = match media::StreamProber::probe(&candidate_url, probe_timeout)
                    .await
                {
                    Ok(info) => {
                        let detected = parse_codec_str(&info.codec);
                        tracing::info!(
                            camera_id = %camera_id,
                            sub_url = %media::mask_rtsp_url(&candidate_url),
                            codec = %info.codec,
                            "用户指定子码流分析模式 (StreamMode::Sub)，检测到子码流编码格式为 {:?}",
                            detected
                        );
                        detected
                    }
                    Err(_) => main_codec,
                };
                (candidate_url, sub_codec)
            } else {
                tracing::warn!(
                    camera_id = %camera_id,
                    "用户指定子码流分析模式但未配置且无法推导，降级回退为主码流"
                );
                (main_trimmed.to_string(), main_codec)
            }
        }
        types::StreamMode::Auto => {
            if !configured_trimmed.is_empty() && configured_trimmed == main_trimmed {
                return (main_trimmed.to_string(), main_codec);
            }

            let candidate = resolve_candidate_sub_url(configured_trimmed, main_trimmed);

            if let Some(candidate_url) = candidate {
                if candidate_url == main_trimmed {
                    return (main_trimmed.to_string(), main_codec);
                }

                match media::StreamProber::probe(&candidate_url, probe_timeout).await {
                    Ok(info) => {
                        let detected_codec = parse_codec_str(&info.codec);
                        tracing::info!(
                            camera_id = %camera_id,
                            sub_url = %media::mask_rtsp_url(&candidate_url),
                            codec = %info.codec,
                            width = info.width,
                            height = info.height,
                            "子码流探活校验成功，全自动启用双流高能效分析管线 (子流编码: {:?})",
                            detected_codec
                        );
                        (candidate_url, detected_codec)
                    }
                    Err(err) => {
                        tracing::warn!(
                            camera_id = %camera_id,
                            candidate_sub_url = %media::mask_rtsp_url(&candidate_url),
                            error = %err,
                            "候选子码流不可用 (未开启/404/超时)，自适应降级使用主码流进行常驻分析"
                        );
                        (main_trimmed.to_string(), main_codec)
                    }
                }
            } else {
                tracing::info!(
                    camera_id = %camera_id,
                    "未匹配到子码流推导规则，自适应使用主码流进行常驻分析"
                );
                (main_trimmed.to_string(), main_codec)
            }
        }
    }
}

/// 异步构造启动参数：包含子码流可用性探活与自适应降级
pub async fn build_start_params_async(
    camera_id: &str,
    camera: &db::entity::camera::Model,
    instances: Vec<InstanceLaunchConfig>,
    motion_gate: Option<&MotionGateConfig>,
) -> StartCameraPipelineParams {
    let main_rtsp_url = camera.rtsp_url.trim().to_string();
    let stream_mode = types::StreamMode::from_str_loose(&camera.stream_mode);
    let main_codec = parse_codec_str(&camera.last_codec);
    let (sub_rtsp_url, sub_codec) = resolve_effective_sub_stream(
        camera_id,
        &main_rtsp_url,
        main_codec,
        &camera.sub_rtsp_url,
        stream_mode,
        std::time::Duration::from_millis(1500),
    )
    .await;

    StartCameraPipelineParams {
        camera_id: camera_id.to_string(),
        main_rtsp_url,
        main_codec,
        sub_rtsp_url,
        sub_codec,
        transport_policy: TransportPolicy::Auto,
        instances,
        motion_gate_enabled: motion_gate.map(|config| config.enabled).unwrap_or(true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_camera(rtsp: &str, sub_rtsp: &str) -> db::entity::camera::Model {
        db::entity::camera::Model {
            id: 1,
            camera_id: "test_cam".to_string(),
            name: "Test Cam".to_string(),
            protocol: "rtsp".to_string(),
            rtsp_url: rtsp.to_string(),
            sub_rtsp_url: sub_rtsp.to_string(),
            stream_mode: "auto".to_string(),
            remark: "".to_string(),
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
        }
    }

    #[tokio::test]
    async fn test_single_stream_fallback_when_sub_rtsp_url_unreachable() {
        // 用户未配置 sub_rtsp_url，主码流为海康 101，自动推导为 102
        // 但由于 127.0.0.1:28554 并不存在真实子码流服务，build_start_params_async 必须自动探活并降级回退到 101
        let cam = mock_camera("rtsp://127.0.0.1:28554/Streaming/Channels/101", "");
        let params = build_start_params_async("test_cam", &cam, vec![], None).await;
        assert_eq!(params.main_rtsp_url, params.sub_rtsp_url);
        assert!(params.is_main_stream_analysis());
        assert_eq!(
            params.sub_rtsp_url,
            "rtsp://127.0.0.1:28554/Streaming/Channels/101"
        );
    }

    #[test]
    fn test_dual_stream_preserved_when_sub_rtsp_url_is_provided() {
        let cam = mock_camera(
            "rtsp://admin:12345@192.168.1.64:554/Streaming/Channels/101",
            "rtsp://admin:12345@192.168.1.64:554/Streaming/Channels/102",
        );
        let params = build_start_params("test_cam", &cam, vec![], None);
        assert_ne!(params.main_rtsp_url, params.sub_rtsp_url);
        assert!(!params.is_main_stream_analysis());
        assert_eq!(
            params.sub_rtsp_url,
            "rtsp://admin:12345@192.168.1.64:554/Streaming/Channels/102"
        );
    }

    #[tokio::test]
    async fn test_resolve_effective_sub_stream_auto_fallback_when_sub_unreachable() {
        // 使用一个本地未开启的端口，模拟推导出的子码流无法连通的情况
        let main_url = "rtsp://127.0.0.1:28554/Streaming/Channels/101";
        let (resolved_url, resolved_codec) = resolve_effective_sub_stream(
            "test_cam",
            main_url,
            CodecType::H265,
            "",
            types::StreamMode::Auto,
            std::time::Duration::from_millis(50),
        )
        .await;

        // 探测推导出的 102 失败后，系统必须自适应回退到主码流 101 与主流 codec，保证分析管线正常启动
        assert_eq!(resolved_url, main_url);
        assert_eq!(resolved_codec, CodecType::H265);
    }

    #[tokio::test]
    async fn test_user_explicit_main_stream_mode_forces_main_stream() {
        // 即使配置了子码流，只要 stream_mode 为 Main，系统必须强制使用主码流进行高质量分析
        let mut cam = mock_camera(
            "rtsp://admin:12345@192.168.1.64:554/Streaming/Channels/101",
            "rtsp://admin:12345@192.168.1.64:554/Streaming/Channels/102",
        );
        cam.stream_mode = "main".to_string();

        let params = build_start_params_async("test_cam", &cam, vec![], None).await;
        assert_eq!(params.main_rtsp_url, params.sub_rtsp_url);
        assert!(params.is_main_stream_analysis());
        assert_eq!(
            params.sub_rtsp_url,
            "rtsp://admin:12345@192.168.1.64:554/Streaming/Channels/101"
        );
    }
}
