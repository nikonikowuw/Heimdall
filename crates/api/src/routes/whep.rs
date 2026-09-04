use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, post};
use axum::Router;
use serde::Deserialize;
use types::TransportPolicy;
use webrtc::api::media_engine::{MediaEngine, MIME_TYPE_H264};
use webrtc::api::APIBuilder;
use webrtc::media::Sample;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::peer_connection_state::RTCPeerConnectionState;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;
use webrtc::track::track_local::TrackLocal;

use crate::state::{AppState, WhepSessionContext};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WhepQuery {
    pub camera_id: Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", post(handle_whep_offer))
        .route("/{sessionId}", delete(handle_whep_delete))
}

fn whep_plain_error(status: StatusCode, msg: impl std::fmt::Display) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "text/plain")],
        msg.to_string(),
    )
        .into_response()
}

/// 处理前端 WebRTC WHEP (RFC 9385) SDP Offer 协商请求
async fn handle_whep_offer(
    State(state): State<AppState>,
    Query(query): Query<WhepQuery>,
    body: String,
) -> Response {
    let camera_id = match query.camera_id {
        Some(id) if !id.trim().is_empty() => id,
        _ => {
            return whep_plain_error(
                StatusCode::BAD_REQUEST,
                "Missing 'cameraId' in query parameters",
            );
        }
    };

    // 1. 查询目标摄像头是否存在
    let camera = match db::CameraRepo::find_by_camera_id(&state.db, &camera_id).await {
        Ok(Some(cam)) => cam,
        Ok(None) => return whep_plain_error(StatusCode::NOT_FOUND, "Camera not found"),
        Err(e) => {
            return whep_plain_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Database error: {e}"),
            );
        }
    };

    // 2. 初始化 WebRTC API 与 H.264 媒体引擎
    let mut media_engine = MediaEngine::default();
    if let Err(e) = media_engine.register_default_codecs() {
        return whep_plain_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Media engine error: {e}"),
        );
    }

    let api = APIBuilder::new().with_media_engine(media_engine).build();
    let peer_connection = match api.new_peer_connection(RTCConfiguration::default()).await {
        Ok(pc) => Arc::new(pc),
        Err(e) => {
            return whep_plain_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to create PeerConnection: {e}"),
            );
        }
    };

    // 3. 创建 H.264 视频轨道
    let video_track = Arc::new(TrackLocalStaticSample::new(
        RTCRtpCodecCapability {
            mime_type: MIME_TYPE_H264.to_owned(),
            ..Default::default()
        },
        "video".to_owned(),
        format!("heimdall-{}", camera.camera_id),
    ));

    let rtp_sender = match peer_connection
        .add_track(Arc::clone(&video_track) as Arc<dyn TrackLocal + Send + Sync>)
        .await
    {
        Ok(sender) => sender,
        Err(e) => {
            return whep_plain_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to add video track: {e}"),
            );
        }
    };

    // 消耗 rtp_sender 避免警告
    tokio::spawn(async move {
        let mut rtcp_buf = vec![0u8; 1500];
        while let Ok((_, _)) = rtp_sender.read(&mut rtcp_buf).await {}
    });

    // 4. 解析 Remote SDP Offer
    let offer = match RTCSessionDescription::offer(body) {
        Ok(o) => o,
        Err(e) => {
            return whep_plain_error(StatusCode::BAD_REQUEST, format!("Invalid SDP Offer: {e}"));
        }
    };

    if let Err(e) = peer_connection.set_remote_description(offer).await {
        return whep_plain_error(
            StatusCode::BAD_REQUEST,
            format!("Failed to set remote description: {e}"),
        );
    }

    // 5. 生成 Local SDP Answer
    let answer = match peer_connection.create_answer(None).await {
        Ok(ans) => ans,
        Err(e) => {
            return whep_plain_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to create SDP answer: {e}"),
            );
        }
    };

    // 收集 ICE Candidate
    let mut gather_complete = peer_connection.gathering_complete_promise().await;
    if let Err(e) = peer_connection.set_local_description(answer).await {
        return whep_plain_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to set local description: {e}"),
        );
    }
    let _ = gather_complete.recv().await;

    let local_desc = match peer_connection.local_description().await {
        Some(desc) => desc,
        None => {
            return whep_plain_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to get local SDP answer",
            );
        }
    };

    // 6. 订阅码流分发并在后台注入秒开帧
    let stream_hub = state.stream_hub.clone();
    let cam_id_clone = camera.camera_id.clone();
    let rtsp_url = camera.rtsp_url.clone();
    let transport_policy = TransportPolicy::Auto;

    let mut packet_rx = match stream_hub
        .subscribe(&cam_id_clone, &rtsp_url, transport_policy)
        .await
    {
        Ok(rx) => rx,
        Err(e) => {
            return whep_plain_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("StreamHub subscription error: {e}"),
            );
        }
    };

    let session_id = uuid::Uuid::new_v4().to_string();
    let session_closed = Arc::new(AtomicBool::new(false));
    let session_closed_pc = session_closed.clone();
    let hub_cleanup = stream_hub.clone();
    let cam_id_cleanup = cam_id_clone.clone();
    let whep_sessions_cleanup = state.whep_sessions.clone();
    let sid_cleanup = session_id.clone();

    // 注册会话至状态管理器
    {
        let mut sessions = state.whep_sessions.write().await;
        sessions.insert(
            session_id.clone(),
            WhepSessionContext {
                camera_id: cam_id_clone.clone(),
                peer_connection: peer_connection.clone(),
                closed: session_closed.clone(),
            },
        );
    }

    // 监听连接断开状态
    peer_connection.on_peer_connection_state_change(Box::new(move |s: RTCPeerConnectionState| {
        if matches!(
            s,
            RTCPeerConnectionState::Failed
                | RTCPeerConnectionState::Closed
                | RTCPeerConnectionState::Disconnected
        ) && !session_closed_pc.swap(true, Ordering::SeqCst)
        {
            let hub = hub_cleanup.clone();
            let cid = cam_id_cleanup.clone();
            let sessions = whep_sessions_cleanup.clone();
            let sid = sid_cleanup.clone();
            tokio::spawn(async move {
                hub.unsubscribe(&cid).await;
                let mut guard = sessions.write().await;
                guard.remove(&sid);
            });
        }
        Box::pin(async {})
    }));

    // 启动异步推流转发协程
    let track_forwarder = video_track.clone();
    tokio::spawn(async move {
        // ① 秒开加速：检查并注入 SPS/PPS 与最近关键帧
        if let Some(cache) = stream_hub.get_keyframe_cache(&cam_id_clone).await {
            for data in [cache.sps, cache.pps, cache.last_keyframe]
                .into_iter()
                .flatten()
            {
                let _ = track_forwarder
                    .write_sample(&Sample {
                        data,
                        duration: Duration::from_millis(40),
                        ..Default::default()
                    })
                    .await;
            }
        }

        // ② 实时持续分发
        while !session_closed.load(Ordering::Relaxed) {
            match packet_rx.recv().await {
                Ok(pkt) => {
                    let sample = Sample {
                        data: pkt.payload.clone(),
                        duration: Duration::from_millis(40),
                        ..Default::default()
                    };
                    if track_forwarder.write_sample(&sample).await.is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // 慢消费静默丢旧包
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    break;
                }
            }
        }
    });

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/sdp"),
    );
    if let Ok(loc_val) =
        axum::http::HeaderValue::from_str(&format!("/api/v1/webrtc/whep/{session_id}"))
    {
        headers.insert(header::LOCATION, loc_val);
    }

    (StatusCode::CREATED, headers, local_desc.sdp).into_response()
}

/// 处理前端显式断开 WHEP 会话
async fn handle_whep_delete(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Response {
    tracing::info!(session_id = %session_id, "收到前端显式 WHEP 释放请求");
    let session_opt = {
        let mut sessions = state.whep_sessions.write().await;
        sessions.remove(&session_id)
    };

    if let Some(session) = session_opt {
        if !session.closed.swap(true, Ordering::SeqCst) {
            let _ = session.peer_connection.close().await;
            state.stream_hub.unsubscribe(&session.camera_id).await;
        }
    }

    StatusCode::NO_CONTENT.into_response()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_whep_delete_nonexistent_session() {
        let db = db::init_db(":memory:").await.unwrap();
        db::create_tables_if_not_exist(&db).await.unwrap();
        let pipeline = Arc::new(pipeline::PipelineManager::new());
        let state = AppState::new(db, pipeline);

        let app = router().with_state(state);
        let req = Request::builder()
            .uri("/dummy-session-123")
            .method("DELETE")
            .body(Body::empty())
            .unwrap();

        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn test_whep_offer_missing_or_invalid_camera() {
        let db = db::init_db(":memory:").await.unwrap();
        db::create_tables_if_not_exist(&db).await.unwrap();
        let pipeline = Arc::new(pipeline::PipelineManager::new());
        let state = AppState::new(db, pipeline);

        let app = router().with_state(state);

        // Missing cameraId
        let req1 = Request::builder()
            .uri("/")
            .method("POST")
            .header("Content-Type", "application/sdp")
            .body(Body::from("v=0\r\n"))
            .unwrap();
        let res1 = app.clone().oneshot(req1).await.unwrap();
        assert_eq!(res1.status(), StatusCode::BAD_REQUEST);

        // Camera not found
        let req2 = Request::builder()
            .uri("/?cameraId=non-existent-cam")
            .method("POST")
            .header("Content-Type", "application/sdp")
            .body(Body::from("v=0\r\n"))
            .unwrap();
        let res2 = app.oneshot(req2).await.unwrap();
        assert_eq!(res2.status(), StatusCode::NOT_FOUND);
    }
}
