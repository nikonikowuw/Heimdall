use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use bytes::{Bytes, BytesMut};
use media::{FlvMuxer, FlvStreamPipeline, MediaError, StreamItem, StreamSubscription};
use serde::Deserialize;
use types::{CodecType, StreamKey, StreamType, TransportPolicy};

use crate::middleware::auth::AuthUser;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct LiveQuery {
    pub stream: Option<String>, // "main" | "sub"
    pub token: Option<String>,
    pub format: Option<String>, // "flv" | "webcodecs"
    /// 是否包含音频数据；默认 false，仅 Hero 主预览窗口按需开启
    pub audio: Option<bool>,
    /// 是否包含视频数据；默认 true。H.265 WebCodecs 视频可配合 audio-only FLV 使用
    pub video: Option<bool>,
}

/// 客户端预览会话 RAII 守护者，确保在任何断开、异常中止或被 Drop 场景下安全注销订阅并扣减按需预览计数
struct PreviewSessionGuard {
    pipeline: std::sync::Arc<pipeline::PipelineManager>,
    stream_key: String,
    camera_id: String,
    protocol: &'static str,
}

impl Drop for PreviewSessionGuard {
    fn drop(&mut self) {
        let pipe = self.pipeline.clone();
        let key = self.stream_key.clone();
        let cid = self.camera_id.clone();
        let proto = self.protocol;
        tokio::spawn(async move {
            pipe.decrement_preview(&cid).await;
            tracing::info!(camera_id = %cid, stream_key = %key, protocol = proto, "实时流客户端已断开，释放订阅与预览引用");
        });
    }
}

fn append_flv_tag(buffer: &mut BytesMut, tag: Bytes, max_bytes: usize) -> Vec<Bytes> {
    if tag.is_empty() {
        return Vec::new();
    }

    let mut chunks = Vec::with_capacity(2);
    if tag.len() > max_bytes {
        if !buffer.is_empty() {
            chunks.push(buffer.split().freeze());
        }
        chunks.push(tag);
        return chunks;
    }

    if !buffer.is_empty() && buffer.len() + tag.len() > max_bytes {
        chunks.push(buffer.split().freeze());
    }
    buffer.extend_from_slice(&tag);
    if buffer.len() == max_bytes {
        chunks.push(buffer.split().freeze());
    }
    chunks
}

fn flush_flv_tags(
    buffer: &mut BytesMut,
    tags: impl IntoIterator<Item = Bytes>,
    max_bytes: usize,
) -> Vec<Bytes> {
    let mut chunks = Vec::new();
    for tag in tags {
        chunks.extend(append_flv_tag(buffer, tag, max_bytes));
    }
    if !buffer.is_empty() {
        chunks.push(buffer.split().freeze());
    }
    chunks
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
    kind: media::ConsumerKind,
) -> Result<(db::entity::camera::Model, StreamKey, StreamSubscription), StatusCode> {
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
        .subscribe_kind(&str_key, &target_url, TransportPolicy::Auto, kind)
        .await
    {
        Ok(subscription) => subscription,
        Err(error) => match error {
            MediaError::TooManyConsumers { .. } => return Err(StatusCode::TOO_MANY_REQUESTS),
            _ => return Err(StatusCode::INTERNAL_SERVER_ERROR),
        },
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
    let include_audio = query.audio.unwrap_or(false);
    let include_video = query.video.unwrap_or(true);
    if !include_audio && !include_video {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let (camera, stream_key, subscription) = match resolve_camera_and_subscribe(
        &state,
        &camera_id,
        query.stream.as_deref(),
        media::ConsumerKind::HttpFlv,
    )
    .await
    {
        Ok(res) => res,
        Err(status) => return status.into_response(),
    };

    let stream_hub = state.stream_hub.clone();
    let pipeline_mgr = state.pipeline.clone();
    let str_key = stream_key.as_str_key();
    let str_key_clone = str_key.clone();
    let mut shutdown_rx = state.shutdown_tx.subscribe();
    let cam_id_for_stream = camera.camera_id.clone();
    let http_merge_flush_ms = state.stream_hub.preview_config().http_merge_flush_ms.max(1);
    let http_merge_max_bytes = state
        .stream_hub
        .preview_config()
        .http_merge_max_bytes
        .max(1);

    // 活跃预览计数增加 (按需激活相关资源)
    pipeline_mgr.increment_preview(&camera.camera_id).await;

    tracing::info!(
        camera_id = %camera.camera_id,
        stream_key = %str_key,
        include_audio,
        include_video,
        "启动 HTTP-FLV 实时流传输 (带 Sequence Header 优先与时间戳单调滤波)"
    );

    let fallback_codec = if camera.last_codec.to_lowercase() == "h265" {
        CodecType::H265
    } else {
        CodecType::H264
    };

    let stream = async_stream::stream! {
        let subscription = subscription;
        let _guard = PreviewSessionGuard {
            pipeline: pipeline_mgr.clone(),
            stream_key: str_key_clone.clone(),
            camera_id: cam_id_for_stream.clone(),
            protocol: "HTTP-FLV",
        };

        // ① 发送 13 字节 FLV Header
        let flv_header = FlvMuxer::flv_header_tracks(include_audio, include_video);
        tracing::debug!(
            stream_key = %str_key_clone,
            include_audio,
            include_video,
            header_len = flv_header.len(),
            type_flags = flv_header.get(4).copied(),
            "HTTP-FLV 已生成 Header"
        );
        yield Ok::<Bytes, std::convert::Infallible>(flv_header);

        let mut flv_pipe = FlvStreamPipeline::new(include_audio);
        let mut flv_merge = BytesMut::with_capacity(http_merge_max_bytes);
        let mut flv_merge_interval = tokio::time::interval(std::time::Duration::from_millis(
            http_merge_flush_ms,
        ));
        flv_merge_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut flv_chunks_merged: u64 = 0;
        let mut flv_flush_count: u64 = 0;
        let mut max_buffer_depth: usize = 0;
        let mut video_packets_seen: u64 = 0;
        let mut audio_packets_seen: u64 = 0;
        let mut audio_packets_waiting_for_video: u64 = 0;
        let mut video_tags_produced: u64 = 0;
        let mut audio_tags_produced: u64 = 0;
        let mut replay_count: u64 = 0;
        let mut source_reset_count: u64 = 0;

        // ② 尝试注入缓存中的 Sequence Header 与完整 GOP 关键帧序列
        if include_video {
            if let Some(cache) = stream_hub.get_keyframe_cache(&str_key_clone).await {
                tracing::debug!(
                    stream_key = %str_key_clone,
                    cache_epoch = cache.epoch,
                    cache_codec = ?cache.codec,
                    gop_packets = cache.gop_packets.len(),
                    sps_bytes = cache.sps.as_ref().map(Bytes::len),
                    pps_bytes = cache.pps.as_ref().map(Bytes::len),
                    vps_bytes = cache.vps.as_ref().map(Bytes::len),
                    "HTTP-FLV 注入缓存 GOP"
                );
                let init_tags = flv_pipe.inject_cache(&cache, fallback_codec);
                tracing::debug!(
                    stream_key = %str_key_clone,
                    init_tags = init_tags.len(),
                    has_sequence_header = flv_pipe.sent_sequence_header,
                    has_first_keyframe = flv_pipe.has_first_keyframe,
                    "HTTP-FLV 缓存 GOP 封装完成"
                );
                for chunk in flush_flv_tags(&mut flv_merge, init_tags, http_merge_max_bytes) {
                    flv_chunks_merged += 1;
                    flv_flush_count += 1;
                    yield Ok(chunk);
                }
                max_buffer_depth = max_buffer_depth.max(flv_merge.len());
            } else {
                tracing::debug!(
                    stream_key = %str_key_clone,
                    "HTTP-FLV 没有可注入的缓存 GOP，等待实时关键帧"
                );
            }
        }

        // ③ 实时消费并封装 NALU 为 FLV Video Tag
        loop {
            let pkt = tokio::select! {
                _ = shutdown_rx.recv() => {
                    tracing::info!(stream_key = %str_key_clone, "HTTP-FLV 收到服务停机信号，主动终止流传输");
                    if !flv_merge.is_empty() {
                        flv_flush_count += 1;
                        yield Ok(flv_merge.split().freeze());
                    }
                    break;
                }
                _ = flv_merge_interval.tick(), if !flv_merge.is_empty() => {
                    flv_flush_count += 1;
                    yield Ok(flv_merge.split().freeze());
                    continue;
                }
                item = subscription.recv() => {
                    match item {
                        Some(StreamItem::Packet(packet)) => packet,
                        Some(StreamItem::Replay(snapshot)) => {
                            replay_count += 1;
                            tracing::debug!(
                                stream_key = %str_key_clone,
                                replay_count,
                                replay_epoch = snapshot.epoch,
                                replay_codec = ?snapshot.codec,
                                replay_packets = snapshot.packets.len(),
                                "HTTP-FLV 收到完整 GOP Replay"
                            );
                            flv_pipe.reset_after_discontinuity();
                            if include_video {
                                let replay_cache = snapshot.to_keyframe_cache();
                                let replay_tags = flv_pipe.inject_cache(&replay_cache, snapshot.codec);
                                for chunk in flush_flv_tags(&mut flv_merge, replay_tags, http_merge_max_bytes) {
                                    flv_chunks_merged += 1;
                                    flv_flush_count += 1;
                                    yield Ok(chunk);
                                }
                                max_buffer_depth = max_buffer_depth.max(flv_merge.len());
                            }
                            continue;
                        }
                        Some(StreamItem::SourceReset { epoch }) => {
                            source_reset_count += 1;
                            tracing::info!(
                                stream_key = %str_key_clone,
                                epoch,
                                source_reset_count,
                                "HTTP-FLV 收到源流重建事件，重置封装状态"
                            );
                            if !flv_merge.is_empty() {
                                flv_flush_count += 1;
                                yield Ok(flv_merge.split().freeze());
                            }
                            flv_pipe.reset_after_discontinuity();
                            continue;
                        }
                        None => {
                            if !flv_merge.is_empty() {
                                flv_flush_count += 1;
                                yield Ok(flv_merge.split().freeze());
                            }
                            break;
                        }
                    }
                }
            };

            // 视频帧：标准 FLV Video Tag 封装
            if include_video && pkt.stream_tag == types::StreamTag::Video {
                video_packets_seen += 1;
                if video_packets_seen == 1 || video_packets_seen.is_multiple_of(100) {
                    tracing::debug!(
                        stream_key = %str_key_clone,
                        video_packets_seen,
                        codec = ?pkt.codec,
                        is_keyframe = pkt.is_keyframe,
                        pts_ms = pkt.pts_ms,
                        payload_bytes = pkt.payload.len(),
                        "HTTP-FLV 收到视频包"
                    );
                }
                let tags = flv_pipe.process_packet(&pkt);
                video_tags_produced += tags.len() as u64;
                for tag in tags {
                    for chunk in append_flv_tag(&mut flv_merge, tag, http_merge_max_bytes) {
                        flv_chunks_merged += 1;
                        yield Ok(chunk);
                    }
                }
            }

            // 音频帧：FLV Audio Tag 封装。audio-only 通道不需要等待视频关键帧。
            if include_audio && pkt.stream_tag == types::StreamTag::Audio {
                audio_packets_seen += 1;
                if audio_packets_seen == 1 || audio_packets_seen.is_multiple_of(100) {
                    tracing::debug!(
                        stream_key = %str_key_clone,
                        audio_packets_seen,
                        codec = ?pkt.codec,
                        pts_ms = pkt.pts_ms,
                        payload_bytes = pkt.payload.len(),
                        has_first_keyframe = flv_pipe.has_first_keyframe,
                        "HTTP-FLV 收到音频包"
                    );
                }
                if !flv_pipe.has_first_keyframe && include_video {
                    audio_packets_waiting_for_video += 1;
                } else {
                    let tags = flv_pipe.process_audio_packet(&pkt);
                    audio_tags_produced += tags.len() as u64;
                    for tag in tags {
                        for chunk in append_flv_tag(&mut flv_merge, tag, http_merge_max_bytes) {
                            flv_chunks_merged += 1;
                            yield Ok(chunk);
                        }
                    }
                }
            }
            max_buffer_depth = max_buffer_depth.max(flv_merge.len());
        }

        tracing::debug!(
            stream_key = %str_key_clone,
            flv_chunks_merged,
            flv_flush_count,
            max_buffer_depth,
            video_packets_seen,
            audio_packets_seen,
            audio_packets_waiting_for_video,
            video_tags_produced,
            audio_tags_produced,
            replay_count,
            source_reset_count,
            "HTTP-FLV 会话合并写指标统计"
        );
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
    let include_audio = query.audio.unwrap_or(false);
    let include_video = query.video.unwrap_or(true);
    if !include_audio && !include_video {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let (camera, stream_key, subscription) = match resolve_camera_and_subscribe(
        &state,
        &camera_id,
        query.stream.as_deref(),
        media::ConsumerKind::HttpFlv,
    )
    .await
    {
        Ok(res) => res,
        Err(status) => return status.into_response(),
    };

    let str_key = stream_key.as_str_key();

    tracing::info!(
        camera_id = %camera.camera_id,
        stream_key = %str_key,
        include_audio,
        include_video,
        "启动 WS-FLV WebSocket 实时流通道"
    );

    let session = WsFlvSession {
        state,
        stream_key: str_key,
        camera,
        subscription,
        include_audio,
        include_video,
    };

    ws.on_upgrade(move |socket| serve_ws_flv(socket, session))
}

struct WsFlvSession {
    state: AppState,
    stream_key: String,
    camera: db::entity::camera::Model,
    subscription: StreamSubscription,
    include_audio: bool,
    include_video: bool,
}

async fn serve_ws_flv(mut socket: WebSocket, session: WsFlvSession) {
    let WsFlvSession {
        state,
        stream_key,
        camera,
        subscription,
        include_audio,
        include_video,
    } = session;

    let mut shutdown_rx = state.shutdown_tx.subscribe();
    let stream_hub = state.stream_hub.clone();
    let pipeline_mgr = state.pipeline.clone();

    // 增加预览客户端计数
    pipeline_mgr.increment_preview(&camera.camera_id).await;

    let _guard = PreviewSessionGuard {
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

    let mut flv_pipe = FlvStreamPipeline::new(include_audio);

    // ① 发送 13 字节 FLV Header 与缓存中的 Sequence Header / GOP 序列
    let flv_header = FlvMuxer::flv_header_tracks(include_audio, include_video);
    let init_success = async {
        if socket.send(Message::Binary(flv_header)).await.is_err() {
            return false;
        }
        if include_video {
            if let Some(cache) = stream_hub.get_keyframe_cache(&stream_key).await {
                for tag in flv_pipe.inject_cache(&cache, fallback_codec) {
                    if socket.send(Message::Binary(tag)).await.is_err() {
                        return false;
                    }
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
                item = subscription.recv() => {
                    match item {
                        Some(StreamItem::Packet(packet)) => packet,
                        Some(StreamItem::Replay(snapshot)) => {
                            flv_pipe.reset_after_discontinuity();
                            if include_video {
                                let replay_cache = snapshot.to_keyframe_cache();
                                for tag in flv_pipe.inject_cache(&replay_cache, fallback_codec) {
                                    if socket.send(Message::Binary(tag)).await.is_err() {
                                        return;
                                    }
                                }
                            }
                            continue;
                        }
                        Some(StreamItem::SourceReset { epoch }) => {
                            tracing::info!(stream_key = %stream_key, epoch, "WS-FLV 收到源流重建事件，重置封装状态");
                            flv_pipe.reset_after_discontinuity();
                            continue;
                        }
                        None => break,
                    }
                }
            };

            // 视频帧：仅 include_video 时封装 Video Tag
            if include_video && pkt.stream_tag == types::StreamTag::Video {
                for tag in flv_pipe.process_packet(&pkt) {
                    if socket.send(Message::Binary(tag)).await.is_err() {
                        return;
                    }
                }
            }

            // 音频帧：FLV Audio Tag 封装。audio-only 通道不需要等待视频关键帧。
            if include_audio && pkt.stream_tag == types::StreamTag::Audio {
                if !flv_pipe.has_first_keyframe && include_video {
                    continue;
                }
                for tag in flv_pipe.process_audio_packet(&pkt) {
                    if socket.send(Message::Binary(tag)).await.is_err() {
                        return;
                    }
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
    let (camera, stream_key, subscription) = match resolve_camera_and_subscribe(
        &state,
        &camera_id,
        query.stream.as_deref(),
        media::ConsumerKind::WebCodecs,
    )
    .await
    {
        Ok(res) => res,
        Err(status) => return status.into_response(),
    };

    let str_key = stream_key.as_str_key();
    let is_main_stream = stream_key.stream_type == StreamType::Main;

    tracing::info!(
        camera_id = %camera.camera_id,
        stream_key = %str_key,
        codec = %camera.last_codec,
        "启动 WS-WebCodecs 超低延时二进制推流通道"
    );

    ws.on_upgrade(move |socket| {
        serve_ws_webcodecs(socket, state, str_key, is_main_stream, camera, subscription)
    })
}

async fn serve_ws_webcodecs(
    mut socket: WebSocket,
    state: AppState,
    stream_key: String,
    _is_main_stream: bool,
    camera: db::entity::camera::Model,
    subscription: StreamSubscription,
) {
    let mut shutdown_rx = state.shutdown_tx.subscribe();
    let stream_hub = state.stream_hub.clone();
    let pipeline_mgr = state.pipeline.clone();

    // 增加预览客户端计数
    pipeline_mgr.increment_preview(&camera.camera_id).await;

    let _guard = PreviewSessionGuard {
        pipeline: pipeline_mgr.clone(),
        stream_key: stream_key.clone(),
        camera_id: camera.camera_id.clone(),
        protocol: "WS-WebCodecs",
    };

    let mut video_packets_seen: u64 = 0;
    let mut webcodecs_frames_sent: u64 = 0;
    let mut replay_count: u64 = 0;
    let mut source_reset_count: u64 = 0;

    // ① 初始秒开：若缓存中有首包与当前 GOP，立即按 WebCodecs 二进制帧格式发送关键帧与完整 GOP 序列
    let init_success = async {
        if let Some(cache) = stream_hub.get_keyframe_cache(&stream_key).await {
            tracing::debug!(
                stream_key = %stream_key,
                cache_epoch = cache.epoch,
                cache_codec = ?cache.codec,
                gop_packets = cache.gop_packets.len(),
                "WebCodecs 注入缓存 GOP"
            );
            for pkt in &cache.gop_packets {
                let bin = media::pack_webcodecs_frame(pkt);
                if socket.send(Message::Binary(bin)).await.is_err() {
                    return false;
                }
                webcodecs_frames_sent += 1;
            }
        } else {
            tracing::debug!(
                stream_key = %stream_key,
                "WebCodecs 没有可注入的缓存 GOP，等待实时关键帧"
            );
        }
        true
    }
    .await;

    if !init_success {
        return;
    }

    let mut pending_discontinuity = false;

    // ② 实时消费并封装 NALU 为 12 字节二进制帧格式推送至 WebSocket
    loop {
        let item = tokio::select! {
            _ = shutdown_rx.recv() => {
                let _ = socket.send(Message::Close(None)).await;
                break;
            }
            item = subscription.recv() => item,
        };

        let Some(item) = item else {
            break;
        };

        match item {
            StreamItem::Replay(snapshot) => {
                replay_count += 1;
                tracing::debug!(
                    stream_key = %stream_key,
                    replay_count,
                    replay_epoch = snapshot.epoch,
                    replay_codec = ?snapshot.codec,
                    replay_packets = snapshot.packets.len(),
                    "WebCodecs 收到完整 GOP Replay"
                );
                pending_discontinuity = false;
                for (index, replay_packet) in snapshot.packets.iter().enumerate() {
                    if replay_packet.stream_tag == types::StreamTag::Audio
                        || !replay_packet.codec.is_video()
                    {
                        continue;
                    }
                    let flags = if index == 0 {
                        media::WEBCODECS_FLAG_DISCONTINUITY
                    } else {
                        0
                    };
                    let bin = media::pack_webcodecs_frame_with_flags(replay_packet, flags);
                    if !bin.is_empty() {
                        webcodecs_frames_sent += 1;
                        if webcodecs_frames_sent == 1 || webcodecs_frames_sent.is_multiple_of(100) {
                            tracing::debug!(
                                stream_key = %stream_key,
                                webcodecs_frames_sent,
                                codec = ?replay_packet.codec,
                                is_keyframe = replay_packet.is_keyframe,
                                pts_ms = replay_packet.pts_ms,
                                payload_bytes = replay_packet.payload.len(),
                                flags,
                                "WebCodecs 发送 Replay 视频帧"
                            );
                        }
                        if socket.send(Message::Binary(bin)).await.is_err() {
                            return;
                        }
                    }
                }
                continue;
            }
            StreamItem::SourceReset { epoch } => {
                source_reset_count += 1;
                tracing::info!(
                    stream_key = %stream_key,
                    epoch,
                    source_reset_count,
                    "WebCodecs 收到源流重建事件，等待新关键帧"
                );
                pending_discontinuity = true;
                continue;
            }
            StreamItem::Packet(pkt) => {
                // WebCodecs 仅传输视频通道数据，忽略音频。
                if pkt.stream_tag == types::StreamTag::Audio || !pkt.codec.is_video() {
                    continue;
                }
                video_packets_seen += 1;
                if video_packets_seen == 1 || video_packets_seen.is_multiple_of(100) {
                    tracing::debug!(
                        stream_key = %stream_key,
                        video_packets_seen,
                        codec = ?pkt.codec,
                        is_keyframe = pkt.is_keyframe,
                        pts_ms = pkt.pts_ms,
                        payload_bytes = pkt.payload.len(),
                        pending_discontinuity,
                        "WebCodecs 收到视频包"
                    );
                }
                if pending_discontinuity && !pkt.is_keyframe {
                    continue;
                }
                let flags = if pending_discontinuity {
                    pending_discontinuity = false;
                    media::WEBCODECS_FLAG_DISCONTINUITY
                } else {
                    0
                };

                let bin = media::pack_webcodecs_frame_with_flags(&pkt, flags);
                if !bin.is_empty() {
                    webcodecs_frames_sent += 1;
                    if webcodecs_frames_sent == 1 || webcodecs_frames_sent.is_multiple_of(100) {
                        tracing::debug!(
                            stream_key = %stream_key,
                            webcodecs_frames_sent,
                            codec = ?pkt.codec,
                            is_keyframe = pkt.is_keyframe,
                            pts_ms = pkt.pts_ms,
                            payload_bytes = pkt.payload.len(),
                            flags,
                            "WebCodecs 发送实时视频帧"
                        );
                    }
                    if socket.send(Message::Binary(bin)).await.is_err() {
                        break;
                    }
                }
            }
        }
    }

    tracing::debug!(
        stream_key = %stream_key,
        video_packets_seen,
        webcodecs_frames_sent,
        replay_count,
        source_reset_count,
        "WebCodecs 会话发送统计"
    );
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_live_flv_rejects_empty_tracks() {
        let db = db::init_test_db().await.unwrap();
        let pipeline = std::sync::Arc::new(pipeline::PipelineManager::new());
        let state = AppState::new(db, pipeline);

        let user = AuthUser {
            username: "admin".into(),
        };

        let response = handle_http_flv(
            State(state),
            user,
            Path("any-cam".into()),
            Query(LiveQuery {
                stream: None,
                token: None,
                format: None,
                audio: Some(false),
                video: Some(false),
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_live_flv_nonexistent_camera() {
        let db = db::init_test_db().await.unwrap();
        let pipeline = std::sync::Arc::new(pipeline::PipelineManager::new());
        let state = AppState::new(db, pipeline);

        let res = resolve_camera_and_subscribe(
            &state,
            "non-existent-uuid",
            None,
            media::ConsumerKind::HttpFlv,
        )
        .await;
        assert!(res.is_err());
        assert_eq!(res.unwrap_err(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_broadcast_lagged_resilience_logic() {
        let mut pipeline = FlvStreamPipeline::new(false);
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
            ..Default::default()
        };
        let frame = media::pack_webcodecs_frame(&packet);
        let (header, payload) = media::unpack_webcodecs_frame(&frame).unwrap();
        assert_eq!(header.codec, CodecType::H265);
        assert!(header.is_keyframe);
        assert_eq!(header.pts_ms, 1000);
        assert_eq!(payload, b"\x00\x00\x00\x01\x40\x01");
    }
}
