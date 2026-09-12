use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};

use crate::error::ApiError;
use crate::response::ApiResponse;
use crate::state::AppState;

/// 挂载快照系统配置路由
pub fn router() -> Router<AppState> {
    Router::new().route(
        "/snapshot/config",
        get(get_snapshot_config).put(update_snapshot_config),
    )
}

/// `GET /api/v1/system/snapshot/config`
pub async fn get_snapshot_config(
    State(state): State<AppState>,
) -> Result<ApiResponse<types::SnapshotSystemConfig>, ApiError> {
    let config = state
        .snapshot_config
        .get()
        .await
        .map_err(ApiError::Internal)?;
    Ok(ApiResponse::success(config))
}

/// `PUT /api/v1/system/snapshot/config`
pub async fn update_snapshot_config(
    State(state): State<AppState>,
    Json(new_config): Json<types::SnapshotSystemConfig>,
) -> Result<ApiResponse<types::SnapshotSystemConfig>, ApiError> {
    new_config.validate().map_err(ApiError::BadRequest)?;
    let config = state
        .snapshot_config
        .update(new_config)
        .await
        .map_err(ApiError::Internal)?;
    Ok(ApiResponse::success(config))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use std::sync::Arc;
    use tower::ServiceExt;

    #[test]
    fn test_snapshot_system_config_validation() {
        let mut cfg = types::SnapshotSystemConfig::default();
        assert!(cfg.validate().is_ok());

        cfg.main_stream_panoramic_quality = 0;
        assert!(cfg.validate().is_err());

        cfg.main_stream_panoramic_quality = 101;
        assert!(cfg.validate().is_err());

        cfg.main_stream_panoramic_quality = 90;
        cfg.crop_padding_ratio = 0.6;
        assert!(cfg.validate().is_err());
    }

    #[tokio::test]
    async fn test_snapshot_config_api_rejects_invalid_persisted_value() {
        let db = db::init_test_db().await.expect("初始化测试数据库失败");
        db::SystemConfigRepo::set(
            &db,
            "snapshot_config",
            r#"{"mainStreamPanoramicQuality":0,"mainStreamCropQuality":95,"subStreamPanoramicQuality":80,"subStreamCropQuality":85,"cropPaddingRatio":0.1}"#,
        )
        .await
        .expect("写入非法快照配置失败");
        let state = AppState::new(db, Arc::new(pipeline::PipelineManager::new()));
        let response = router()
            .with_state(state)
            .oneshot(
                Request::builder()
                    .uri("/snapshot/config")
                    .body(Body::empty())
                    .expect("构造 GET 请求失败"),
            )
            .await
            .expect("执行 GET 请求失败");

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn test_snapshot_config_api_flow() {
        let db = db::init_test_db().await.expect("初始化测试数据库失败");
        let pipeline = Arc::new(pipeline::PipelineManager::new());
        let state = AppState::new(db, pipeline);
        let app = router().with_state(state);

        // 1. GET 默认配置
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/snapshot/config")
                    .body(Body::empty())
                    .expect("构造 GET 请求失败"),
            )
            .await
            .expect("执行 GET 请求失败");

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("读取 GET 响应体失败");
        let resp: serde_json::Value = serde_json::from_slice(&body).expect("反序列化 GET 响应失败");
        assert_eq!(resp["code"], 0);
        assert_eq!(
            resp["data"]["mainStreamPanoramicQuality"]
                .as_u64()
                .expect("质量字段应存在"),
            90
        );

        // 2. PUT 更新配置
        let new_cfg = types::SnapshotSystemConfig {
            main_stream_panoramic_quality: 85,
            main_stream_crop_quality: 92,
            sub_stream_panoramic_quality: 75,
            sub_stream_crop_quality: 80,
            crop_padding_ratio: 0.15,
        };
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/snapshot/config")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&new_cfg).expect("序列化请求体失败"),
                    ))
                    .expect("构造 PUT 请求失败"),
            )
            .await
            .expect("执行 PUT 请求失败");

        assert_eq!(response.status(), StatusCode::OK);

        // 3. 再次 GET 确认持久化与读取
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/snapshot/config")
                    .body(Body::empty())
                    .expect("构造确认 GET 请求失败"),
            )
            .await
            .expect("执行确认 GET 请求失败");

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("读取确认 GET 响应体失败");
        let resp: serde_json::Value =
            serde_json::from_slice(&body).expect("反序列化确认 GET 响应失败");
        assert_eq!(
            resp["data"]["mainStreamPanoramicQuality"]
                .as_u64()
                .expect("质量字段应存在"),
            85
        );
        assert_eq!(
            resp["data"]["subStreamPanoramicQuality"]
                .as_u64()
                .expect("质量字段应存在"),
            75
        );
        assert!(
            (resp["data"]["cropPaddingRatio"]
                .as_f64()
                .expect("裁剪比例应存在")
                - 0.15)
                .abs()
                < 1e-4
        );
    }
}
