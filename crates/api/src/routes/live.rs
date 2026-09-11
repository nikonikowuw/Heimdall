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
    pub format: Option<String>, // "flv" | "webcodecs"
}

/// 客户端预览会话 RAII 守护者，确保在任何断开、异常中止或被 Drop 场景下安全注销订阅并扣减按需预览计数
struct PreviewSessionGuard {
    stream_hub: std::sync::Arc<media::StreamHub>,
    pipeline: std::sync::Arc<pipeline::PipelineManager>,
    stream_key: String,
    camera_id: String,
    protocol: &'static str,
}

impl Drop for PreviewSessionGuard {
    fn drop(&mut self) {
        let hub = self.stream_hub.clone();
        let pipe = self.pipeline.clone();
        let key = self.stream_key.clone();
        let cid = self.camera_id.clone();
        let proto = self.protocol;
        tokio::spawn(async move {
            hub.unsubscribe(&key).await;
            pipe.decrement_preview(&cid).await;
            tracing::info!(camera_id = %cid, stream_key = %key, protocol = proto, "实时流客户端已断开，释放订阅与预览引用");
        });
    }
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/{camera_id}", get(handle_http_flv))
        .route("/{camera_id}/flv", get(handle_http_flv))
        .route("/{camera_id}/ws", get(handle_ws_live))
        .route("/{camera_id}/webcodecs", get(handle_ws_webcodecs))
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

    let stream_mode = types::StreamMode::from_str_loose(&camera.stream_mode);
    let requested_kind = StreamType::from_str_loose(stream_type.unwrap_or("main"));
    // 若客户端请求子码流，但在主码流模式且未配置子码流时，自适应回退为主码流
    let stream_kind = if requested_kind == StreamType::Sub
        && stream_mode == types::StreamMode::Main
        && camera.sub_rtsp_url.trim().is_empty()
    {
        StreamType::Main
    } else {
        requested_kind
    };
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
    let pipeline_mgr = state.pipeline.clone();
    let str_key = stream_key.as_str_key();
    let str_key_clone = str_key.clone();
    let mut shutdown_rx = state.shutdown_tx.subscribe();
    let is_main_stream = stream_key.stream_type == StreamType::Main;
    let cam_id_for_stream = camera.camera_id.clone();

    // 活跃预览计数增加 (按需激活相关资源)
    pipeline_mgr.increment_preview(&camera.camera_id).await;

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
        let _guard = PreviewSessionGuard {
            stream_hub: stream_hub.clone(),
            pipeline: pipeline_mgr.clone(),
            stream_key: str_key_clone.clone(),
            camera_id: cam_id_for_stream.clone(),
            protocol: "HTTP-FLV",
        };

        // ① 发送 13 字节 FLV Header
        yield Ok::<Bytes, std::convert::Infallible>(FlvMuxer::flv_header());

        let mut flv_pipe = FlvStreamPipeline::new();

        // ② 尝试注入缓存中的 Sequence Header 与完整 GOP 关键帧序列
        if let Some(cache) = stream_hub.get_keyframe_cache(&str_key_clone).await {
            let init_tags = flv_pipe.inject_cache(&cache, fallback_codec);
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
                            flv_pipe.handle_lagged();
                            continue;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            };

            // 若为主码流，同步推入内存 Ring Buffer 供告警瞬时靶向精准抽帧
            if is_main_stream {
                pipeline_mgr.push_main_packet(&cam_id_for_stream, pkt.clone()).await;
            }

            for tag in flv_pipe.process_packet(&pkt) {
                yield Ok(tag);
            }
        }
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

/// 处理 WebSocket 实时流拉取 (按 query.format 路由至 WS-FLV 或 WS-WebCodecs)
async fn handle_ws_live(
    state: State<AppState>,
    user: AuthUser,
    path: Path<String>,
    query: Query<LiveQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    if query.format.as_deref() == Some("webcodecs") {
        handle_ws_webcodecs(state, user, path, query, ws).await
    } else {
        handle_ws_flv(state, user, path, query, ws).await
    }
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
    let is_main_stream = stream_key.stream_type == StreamType::Main;

    tracing::info!(
        camera_id = %camera.camera_id,
        stream_key = %str_key,
        "启动 WS-FLV WebSocket 实时流通道"
    );

    ws.on_upgrade(move |socket| {
        serve_ws_flv(socket, state, str_key, is_main_stream, camera, packet_rx)
    })
}

async fn serve_ws_flv(
    mut socket: WebSocket,
    state: AppState,
    stream_key: String,
    is_main_stream: bool,
    camera: db::entity::camera::Model,
    mut packet_rx: tokio::sync::broadcast::Receiver<std::sync::Arc<types::EncodedPacket>>,
) {
    let mut shutdown_rx = state.shutdown_tx.subscribe();
    let stream_hub = state.stream_hub.clone();
    let pipeline_mgr = state.pipeline.clone();

    // 增加预览客户端计数
    pipeline_mgr.increment_preview(&camera.camera_id).await;

    let _guard = PreviewSessionGuard {
        stream_hub: stream_hub.clone(),
        pipeline: pipeline_mgr.clone(),
        stream_key: stream_key.clone(),
        camera_id: camera.camera_id.clone(),
        protocol: "WS-FLV",
    };

    let fallback_codec = if camera.last_codec.to_lowercase() == "h265" {
        CodecType::H265
    } else {
        CodecType::H264
    };

    let mut flv_pipe = FlvStreamPipeline::new();

    // ① 发送 13 字节 FLV Header 与缓存中的 Sequence Header / GOP 序列
    let init_success = async {
        if socket
            .send(Message::Binary(FlvMuxer::flv_header()))
            .await
            .is_err()
        {
            return false;
        }
        if let Some(cache) = stream_hub.get_keyframe_cache(&stream_key).await {
            for tag in flv_pipe.inject_cache(&cache, fallback_codec) {
                if socket.send(Message::Binary(tag)).await.is_err() {
                    return false;
                }
            }
        }
        true
    }
    .await;

    // ② 初始序列发送成功后，实时消费并封装 NALU 为 FLV Video Tag 发送至 WebSocket
    if init_success {
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
                            flv_pipe.handle_lagged();
                            continue;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            };

            if is_main_stream {
                pipeline_mgr
                    .push_main_packet(&camera.camera_id, pkt.clone())
                    .await;
            }

            for tag in flv_pipe.process_packet(&pkt) {
                if socket.send(Message::Binary(tag)).await.is_err() {
                    break;
                }
            }
        }
    }
}

/// 处理 WS-WebCodecs 实时流拉取 (WebSocket 直送 12 字节二进制帧头原始 NALU，供前端 WebCodecs 零拷贝低延时渲染)
async fn handle_ws_webcodecs(
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
    let is_main_stream = stream_key.stream_type == StreamType::Main;

    tracing::info!(
        camera_id = %camera.camera_id,
        stream_key = %str_key,
        "启动 WS-WebCodecs 超低延时二进制推流通道"
    );

    ws.on_upgrade(move |socket| {
        serve_ws_webcodecs(socket, state, str_key, is_main_stream, camera, packet_rx)
    })
}

async fn serve_ws_webcodecs(
    mut socket: WebSocket,
    state: AppState,
    stream_key: String,
    is_main_stream: bool,
    camera: db::entity::camera::Model,
    mut packet_rx: tokio::sync::broadcast::Receiver<std::sync::Arc<types::EncodedPacket>>,
) {
    let mut shutdown_rx = state.shutdown_tx.subscribe();
    let stream_hub = state.stream_hub.clone();
    let pipeline_mgr = state.pipeline.clone();

    // 增加预览客户端计数
    pipeline_mgr.increment_preview(&camera.camera_id).await;

    let _guard = PreviewSessionGuard {
        stream_hub: stream_hub.clone(),
        pipeline: pipeline_mgr.clone(),
        stream_key: stream_key.clone(),
        camera_id: camera.camera_id.clone(),
        protocol: "WS-WebCodecs",
    };

    // ① 初始秒开：若缓存中有首包与当前 GOP，立即按 WebCodecs 二进制帧格式发送关键帧与完整 GOP 序列
    let init_success = async {
        if let Some(cache) = stream_hub.get_keyframe_cache(&stream_key).await {
            for pkt in &cache.gop_packets {
                let bin = media::pack_webcodecs_frame(pkt);
                if socket.send(Message::Binary(bin)).await.is_err() {
                    return false;
                }
            }
        }
        true
    }
    .await;

    if !init_success {
        return;
    }

    let mut awaiting_keyframe_after_lag = false;

    // ② 实时消费并封装 NALU 为 12 字节二进制帧格式推送至 WebSocket
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
                            "WebCodecs 消费端网络积压，主动丢弃后续 P 帧并等待下一个关键帧重新对齐"
                        );
                        awaiting_keyframe_after_lag = true;
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        };

        // 若网络发生积压丢包，主动丢弃非关键帧直到收到完整关键帧
        if awaiting_keyframe_after_lag {
            if pkt.is_keyframe {
                awaiting_keyframe_after_lag = false;
            } else {
                continue;
            }
        }

        if is_main_stream {
            pipeline_mgr
                .push_main_packet(&camera.camera_id, pkt.clone())
                .await;
        }

        let bin = media::pack_webcodecs_frame(&pkt);
        if socket.send(Message::Binary(bin)).await.is_err() {
            break;
        }
    }
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

    #[test]
    fn test_webcodecs_framing_codec_and_flags() {
        let packet = types::EncodedPacket {
            pts_ms: 1000,
            is_keyframe: true,
            codec: CodecType::H265,
            payload: Bytes::from_static(b"\x00\x00\x00\x01\x40\x01"),
        };
        let frame = media::pack_webcodecs_frame(&packet);
        let (header, payload) = media::unpack_webcodecs_frame(&frame).unwrap();
        assert_eq!(header.codec, CodecType::H265);
        assert!(header.is_keyframe);
        assert_eq!(header.pts_ms, 1000);
        assert_eq!(payload, b"\x00\x00\x00\x01\x40\x01");
    }
}
