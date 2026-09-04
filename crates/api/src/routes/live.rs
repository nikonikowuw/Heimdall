use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use bytes::Bytes;
use media::{FlvMuxer, FlvStreamPipeline};
use serde::Deserialize;
use types::{CodecType, StreamKey, StreamType, TransportPolicy};

use crate::middleware::auth::AuthUser;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct LiveQuery {
    pub stream: Option<String>, // "main" | "sub"
    pub token: Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/{camera_id}", get(handle_http_flv))
        .route("/{camera_id}/flv", get(handle_http_flv))
        .route("/{camera_id}/ws", get(handle_ws_flv))
}

/// 辅助函数：根据请求参数解析摄像头并订阅目标码流
async fn resolve_camera_and_subscribe(
    state: &AppState,
    camera_id: &str,
    stream_type: Option<&str>,
) -> Result<
    (
        db::entity::camera::Model,
        StreamKey,
        tokio::sync::broadcast::Receiver<std::sync::Arc<types::EncodedPacket>>,
    ),
    StatusCode,
> {
    let clean_camera_id = camera_id
        .strip_suffix(".flv")
        .unwrap_or(camera_id)
        .to_string();
    let camera = match db::CameraRepo::find_by_camera_id(&state.db, &clean_camera_id).await {
        Ok(Some(cam)) => cam,
        Ok(None) => return Err(StatusCode::NOT_FOUND),
        Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
    };

    let stream_kind = StreamType::from_str_loose(stream_type.unwrap_or("main"));
    let stream_key = StreamKey::new(&clean_camera_id, stream_kind);

    let target_url = if stream_kind == StreamType::Sub {
        if !camera.sub_rtsp_url.trim().is_empty() {
            camera.sub_rtsp_url.clone()
        } else {
            media::sub_stream::deduce_sub_stream(&camera.rtsp_url)
                .into_iter()
                .next()
                .map(|c| c.sub_url)
                .unwrap_or_else(|| camera.rtsp_url.clone())
        }
    } else {
        camera.rtsp_url.clone()
    };

    let str_key = stream_key.as_str_key();
    let packet_rx = match state
        .stream_hub
        .subscribe(&str_key, &target_url, TransportPolicy::Auto)
        .await
    {
        Ok(rx) => rx,
        Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR),
    };

    Ok((camera, stream_key, packet_rx))
}

/// 处理 HTTP-FLV 实时流拉取 (分块传输 Chunked Transfer)
async fn handle_http_flv(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(camera_id): Path<String>,
    Query(query): Query<LiveQuery>,
) -> Response {
    let (camera, stream_key, mut packet_rx) =
        match resolve_camera_and_subscribe(&state, &camera_id, query.stream.as_deref()).await {
            Ok(res) => res,
            Err(status) => return status.into_response(),
        };

    let stream_hub = state.stream_hub.clone();
    let str_key = stream_key.as_str_key();
    let str_key_clone = str_key.clone();
    let mut shutdown_rx = state.shutdown_tx.subscribe();

    tracing::info!(
        camera_id = %camera.camera_id,
        stream_key = %str_key,
        "启动 HTTP-FLV 实时流传输 (带 Sequence Header 优先与时间戳单调滤波)"
    );

    let fallback_codec = if camera.last_codec.to_lowercase() == "h265" {
        CodecType::H265
    } else {
        CodecType::H264
    };

    let stream = async_stream::stream! {
        // ① 发送 13 字节 FLV Header
        yield Ok::<Bytes, std::convert::Infallible>(FlvMuxer::flv_header());

        let mut pipeline = FlvStreamPipeline::new();

        // ② 尝试注入缓存中的 Sequence Header 与完整 GOP 关键帧序列
        if let Some(cache) = stream_hub.get_keyframe_cache(&str_key_clone).await {
            let init_tags = pipeline.inject_cache(&cache, fallback_codec);
            for tag in init_tags {
                yield Ok(tag);
            }
        }

        // ③ 实时消费并封装 NALU 为 FLV Video Tag
        loop {
            let pkt = tokio::select! {
                _ = shutdown_rx.recv() => {
                    tracing::info!(stream_key = %str_key_clone, "HTTP-FLV 收到服务停机信号，主动终止流传输");
                    break;
                }
                res = packet_rx.recv() => {
                    match res {
                        Ok(p) => p,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                            tracing::warn!(
                                stream_key = %str_key_clone,
                                skipped,
                                "HTTP-FLV 消费端处理落后，跳过残片帧并等待下一个关键帧重新对齐"
                            );
                            pipeline.handle_lagged();
                            continue;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            };

            for tag in pipeline.process_packet(&pkt) {
                yield Ok(tag);
            }
        }

        // 客户端断开连接，自动注销订阅
        stream_hub.unsubscribe(&str_key_clone).await;
        tracing::info!(camera_id = %camera.camera_id, "HTTP-FLV 客户端已断开，释放流媒体订阅");
    };

    let body = axum::body::Body::from_stream(stream);

    let mut resp_headers = HeaderMap::new();
    resp_headers.insert(
        header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("video/x-flv"),
    );
    resp_headers.insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-cache, no-store, must-revalidate"),
    );
    resp_headers.insert(
        header::CONNECTION,
        axum::http::HeaderValue::from_static("keep-alive"),
    );
    resp_headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        axum::http::HeaderValue::from_static("*"),
    );

    (StatusCode::OK, resp_headers, body).into_response()
}

/// 处理 WS-FLV (WebSocket-FLV) 实时流拉取
async fn handle_ws_flv(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(camera_id): Path<String>,
    Query(query): Query<LiveQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    let (camera, stream_key, packet_rx) =
        match resolve_camera_and_subscribe(&state, &camera_id, query.stream.as_deref()).await {
            Ok(res) => res,
            Err(status) => return status.into_response(),
        };

    let str_key = stream_key.as_str_key();

    tracing::info!(
        camera_id = %camera.camera_id,
        stream_key = %str_key,
        "启动 WS-FLV WebSocket 实时流通道"
    );

    ws.on_upgrade(move |socket| serve_ws_flv(socket, state, str_key, camera, packet_rx))
}

async fn serve_ws_flv(
    mut socket: WebSocket,
    state: AppState,
    stream_key: String,
    camera: db::entity::camera::Model,
    mut packet_rx: tokio::sync::broadcast::Receiver<std::sync::Arc<types::EncodedPacket>>,
) {
    let mut shutdown_rx = state.shutdown_tx.subscribe();
    let stream_hub = state.stream_hub.clone();

    // ① 发送 13 字节 FLV Header
    let flv_header = FlvMuxer::flv_header();
    if socket.send(Message::Binary(flv_header)).await.is_err() {
        stream_hub.unsubscribe(&stream_key).await;
        return;
    }

    let fallback_codec = if camera.last_codec.to_lowercase() == "h265" {
        CodecType::H265
    } else {
        CodecType::H264
    };

    let mut pipeline = FlvStreamPipeline::new();

    // ② 尝试注入缓存中的 Sequence Header 与完整 GOP 序列
    if let Some(cache) = stream_hub.get_keyframe_cache(&stream_key).await {
        for tag in pipeline.inject_cache(&cache, fallback_codec) {
            if socket.send(Message::Binary(tag)).await.is_err() {
                stream_hub.unsubscribe(&stream_key).await;
                return;
            }
        }
    }

    // ③ 实时消费并封装 NALU 为 FLV Video Tag 发送至 WebSocket
    loop {
        let pkt = tokio::select! {
            _ = shutdown_rx.recv() => {
                let _ = socket.send(Message::Close(None)).await;
                break;
            }
            res = packet_rx.recv() => {
                match res {
                    Ok(p) => p,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(
                            stream_key = %stream_key,
                            skipped,
                            "WS-FLV 消费端处理落后，跳过残片帧并等待下一个关键帧重新对齐"
                        );
                        pipeline.handle_lagged();
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        };

        for tag in pipeline.process_packet(&pkt) {
            if socket.send(Message::Binary(tag)).await.is_err() {
                break;
            }
        }
    }

    stream_hub.unsubscribe(&stream_key).await;
    tracing::info!(stream_key = %stream_key, "WS-FLV 客户端已断开，释放流媒体订阅");
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_live_flv_nonexistent_camera() {
        let db = db::init_test_db().await.unwrap();
        let pipeline = std::sync::Arc::new(pipeline::PipelineManager::new());
        let state = AppState::new(db, pipeline);

        let res = resolve_camera_and_subscribe(&state, "non-existent-uuid", None).await;
        assert!(res.is_err());
        assert_eq!(res.unwrap_err(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_broadcast_lagged_resilience_logic() {
        let mut pipeline = FlvStreamPipeline::new();
        pipeline.handle_lagged();
        assert!(!pipeline.has_first_keyframe);
    }
}
