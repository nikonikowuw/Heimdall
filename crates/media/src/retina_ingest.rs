//! 基于 Retina 的生产级异步 RTSP 客户端接入内核
//!
//! 采用成熟工业级 `retina` 库 (Moonfire NVR 核心) 驱动：
//! 1. 深度抗非标安防摄像头畸形 SDP 与异常握手报文；
//! 2. 原生支持 TCP (Interleaved) 与 UDP RTP 传输策略；
//! 3. 自动将音视频流解包为标准 Annex B NALU 序列 (`FrameFormat::SIMPLE`)；
//! 4. 将网络数据流转化为系统统一的 `types::EncodedPacket`，无缝送入 StreamHub 广播分发。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use futures::StreamExt;
use retina::client::{
    Credentials, PlayOptions, Session, SessionOptions, SetupOptions, TcpTransportOptions,
    Transport, UdpTransportOptions,
};
use retina::codec::{CodecItem, FrameFormat};
use tokio::sync::broadcast;
use types::{CodecType, EncodedPacket, TransportPolicy};

use crate::error::MediaError;
use crate::rtsp::{mask_rtsp_url, parse_and_clean_rtsp_url};

/// 将原始 RTSP URL 中的凭证（用户名与密码）提取分离，并返回供 Retina 使用的纯净 Url
///
/// Retina 严格禁止 Url 中包含嵌入式凭据，否则会在运行时返回 `URL must not contain credentials`。
/// 本函数依托逆向锚点解析，能完整支持密码中包含 `@`, `:`, `#`, `?`, `!` 等特殊字符的工业级 RTSP 地址。
pub fn sanitize_rtsp_url_and_credentials(
    raw_url: &str,
) -> Result<(url::Url, Option<Credentials>), MediaError> {
    let parsed = parse_and_clean_rtsp_url(raw_url)?;
    let clean_url = parsed.to_clean_url()?;
    let creds = parsed.username.map(|username| Credentials {
        username,
        password: parsed.password.unwrap_or_default(),
    });
    Ok((clean_url, creds))
}

/// 根据系统配置将 TransportPolicy 转换为 Retina 的底层传输模型
pub fn map_transport_policy(policy: TransportPolicy) -> Transport {
    match policy {
        TransportPolicy::Udp => Transport::Udp(UdpTransportOptions::default()),
        TransportPolicy::Tcp | TransportPolicy::Auto => {
            Transport::Tcp(TcpTransportOptions::default())
        }
    }
}

/// 实际底层传输使用的网络协议模式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportMode {
    Tcp,
    Udp,
}

/// 从 SDP 编码名称推导 CodecType（严格白名单校验 H.264 与 H.265，拒绝未知编码）
pub fn parse_video_codec(encoding_name: &str) -> Result<CodecType, MediaError> {
    let lower = encoding_name.to_lowercase();
    if lower.contains("h265") || lower.contains("hevc") {
        Ok(CodecType::H265)
    } else if lower.contains("h264") || lower.contains("avc") {
        Ok(CodecType::H264)
    } else {
        Err(MediaError::UnsupportedCodec(format!(
            "Retina 不支持的视频编码格式: {encoding_name}"
        )))
    }
}

/// 基于 Retina 引擎的生产级 RTSP 接入器
#[derive(Debug)]
pub struct RetinaIngestor {
    pub camera_id: String,
    pub rtsp_url: String,
    pub transport_policy: TransportPolicy,
    pub tx: broadcast::Sender<Arc<EncodedPacket>>,
    pub is_running: Arc<AtomicBool>,
}

impl RetinaIngestor {
    pub fn new(
        camera_id: String,
        rtsp_url: String,
        transport_policy: TransportPolicy,
        tx: broadcast::Sender<Arc<EncodedPacket>>,
    ) -> Self {
        Self {
            camera_id,
            rtsp_url,
            transport_policy,
            tx,
            is_running: Arc::new(AtomicBool::new(false)),
        }
    }

    /// 启动拉流循环（具备指数退避自愈、Auto 模式自动降级与 Tokio 异步取消支持）
    pub async fn run_loop(
        self: Arc<Self>,
        cancel_signal: Arc<AtomicBool>,
        mut cancel_rx: tokio::sync::watch::Receiver<bool>,
    ) {
        self.is_running.store(true, Ordering::SeqCst);
        let mut current_transport = match self.transport_policy {
            TransportPolicy::Udp => TransportMode::Udp,
            TransportPolicy::Tcp | TransportPolicy::Auto => TransportMode::Tcp,
        };
        let mut backoff = Duration::from_secs(1);
        let max_backoff = Duration::from_secs(30);

        tracing::info!(
            camera_id = %self.camera_id,
            url = %mask_rtsp_url(&self.rtsp_url),
            policy = ?self.transport_policy,
            "Retina RTSP Ingestor 核心拉流器启动"
        );

        while !cancel_signal.load(Ordering::Relaxed) && !*cancel_rx.borrow() {
            match self
                .stream_session(current_transport, cancel_signal.clone(), cancel_rx.clone())
                .await
            {
                Ok(()) => {
                    tracing::info!(camera_id = %self.camera_id, "Retina RTSP 拉流会话正常关闭或退出");
                    break;
                }
                Err((err, frames_streamed)) => {
                    if cancel_signal.load(Ordering::Relaxed) || *cancel_rx.borrow() {
                        break;
                    }

                    // 经历过稳定流数据接收后发生的断开，代表长连接中途网络波动，重置退避时间为 1 秒
                    if frames_streamed > 0 {
                        backoff = Duration::from_secs(1);
                    }

                    // Auto 模式下若初始连接/握手阶段失败（未成功产生帧），自适应降级至 UDP 模式尝试拉流
                    if self.transport_policy == TransportPolicy::Auto && frames_streamed == 0 {
                        if current_transport == TransportMode::Tcp {
                            tracing::warn!(
                                camera_id = %self.camera_id,
                                "TransportPolicy::Auto: TCP 传输建立失败，自适应降级切换至 UDP 模式尝试拉流"
                            );
                            current_transport = TransportMode::Udp;
                        } else {
                            current_transport = TransportMode::Tcp;
                        }
                    }

                    tracing::warn!(
                        camera_id = %self.camera_id,
                        error = %err,
                        transport = ?current_transport,
                        retry_after_secs = backoff.as_secs(),
                        "Retina RTSP 连接异常中断，准备指数退避重连"
                    );

                    tokio::select! {
                        biased;
                        change_res = cancel_rx.changed() => {
                            if change_res.is_err() || *cancel_rx.borrow() {
                                break;
                            }
                        }
                        _ = tokio::time::sleep(backoff) => {}
                    }
                    backoff = (backoff * 2).min(max_backoff);
                }
            }
        }

        self.is_running.store(false, Ordering::SeqCst);
        tracing::info!(camera_id = %self.camera_id, "Retina RTSP Ingestor 核心拉流器已停止");
    }

    /// 单个 RTSP 会话的完整生命周期（DESCRIBE -> SETUP -> PLAY -> 持续解包接收）
    async fn stream_session(
        &self,
        transport_mode: TransportMode,
        cancel_signal: Arc<AtomicBool>,
        mut cancel_rx: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), (MediaError, u64)> {
        let masked_url = mask_rtsp_url(&self.rtsp_url);
        let (clean_url, creds) =
            sanitize_rtsp_url_and_credentials(&self.rtsp_url).map_err(|e| (e, 0))?;

        // 1. 配置 Retina Session 参数（注入凭证与 User-Agent）
        let session_options = SessionOptions::default()
            .user_agent("Heimdall/1.0".to_string())
            .creds(creds);

        tracing::debug!(
            camera_id = %self.camera_id,
            url = %masked_url,
            transport = ?transport_mode,
            "Retina 发起 DESCRIBE 请求握手"
        );

        // 2. 发起 DESCRIBE 握手并解析 SDP
        let mut session = Session::describe(clean_url, session_options)
            .await
            .map_err(|e| {
                (
                    MediaError::RtspConnect {
                        url: masked_url.clone(),
                        reason: format!("Retina DESCRIBE 握手失败: {e}"),
                    },
                    0,
                )
            })?;

        // 3. 寻找可用的视频轨道并校验编解码格式
        let (video_idx, codec) = session
            .streams()
            .iter()
            .enumerate()
            .find(|(_, s)| s.media() == "video")
            .map(|(idx, s)| {
                parse_video_codec(s.encoding_name())
                    .map(|c| (idx, c))
                    .map_err(|e| (e, 0))
            })
            .ok_or_else(|| {
                (
                    MediaError::Protocol(
                        "Retina SDP 响应中未找到有效的视频轨道 (video track)".into(),
                    ),
                    0,
                )
            })??;

        tracing::info!(
            camera_id = %self.camera_id,
            video_track = video_idx,
            ?codec,
            transport = ?transport_mode,
            "Retina 成功解析出视频轨，准备执行 SETUP"
        );

        // 4. 执行 SETUP，指定网络传输策略与 FrameFormat::SIMPLE (自动注入 Annex B 与关键帧参数集)
        let transport = match transport_mode {
            TransportMode::Tcp => Transport::Tcp(TcpTransportOptions::default()),
            TransportMode::Udp => Transport::Udp(UdpTransportOptions::default()),
        };
        let setup_options = SetupOptions::default()
            .transport(transport)
            .frame_format(FrameFormat::SIMPLE);

        session.setup(video_idx, setup_options).await.map_err(|e| {
            (
                MediaError::Protocol(format!("Retina SETUP 阶段失败: {e}")),
                0,
            )
        })?;

        // 5. 执行 PLAY 握手并获取解复用流
        let playing_session = session.play(PlayOptions::default()).await.map_err(|e| {
            (
                MediaError::Protocol(format!("Retina PLAY 阶段失败: {e}")),
                0,
            )
        })?;

        let mut demuxed = playing_session.demuxed().map_err(|e| {
            (
                MediaError::Protocol(format!("Retina demuxed 初始化失败: {e}")),
                0,
            )
        })?;

        tracing::info!(
            camera_id = %self.camera_id,
            "Retina RTSP 握手闭环成功，开始消费解复用帧数据"
        );

        let base_timestamp_ms = chrono::Utc::now().timestamp_millis();
        let mut last_emitted_pts = 0i64;
        let mut frames_received: u64 = 0;

        // 清理伪唤醒
        let _ = cancel_rx.borrow_and_update();

        // 6. 持续消费 VideoFrame
        while !cancel_signal.load(Ordering::Relaxed) && !*cancel_rx.borrow() {
            let next_item = tokio::select! {
                biased;
                change_res = cancel_rx.changed() => {
                    if change_res.is_err() || *cancel_rx.borrow() {
                        break;
                    }
                    continue;
                }
                item = demuxed.next() => item,
            };

            let item = match next_item {
                Some(Ok(it)) => it,
                Some(Err(e)) => {
                    tracing::warn!(
                        camera_id = %self.camera_id,
                        error = %e,
                        frames_received,
                        "Retina 解复用读取错误，准备重连"
                    );
                    return Err((
                        MediaError::Protocol(format!("Retina 数据流错误: {e}")),
                        frames_received,
                    ));
                }
                None => {
                    return Err((
                        MediaError::RtspConnect {
                            url: masked_url.clone(),
                            reason: "Retina 数据流已到达 EOF (对端已关闭)".into(),
                        },
                        frames_received,
                    ));
                }
            };

            if let CodecItem::VideoFrame(frame) = item {
                if frame.stream_id() == video_idx {
                    frames_received += 1;
                    let elapsed_ms = (frame.timestamp().elapsed_secs() * 1000.0) as i64;
                    let calculated_pts = base_timestamp_ms + elapsed_ms;
                    let pts_ms = calculated_pts.max(last_emitted_pts);
                    last_emitted_pts = pts_ms;

                    let is_keyframe = frame.is_random_access_point();
                    // 零拷贝借出底层 Vec<u8> 生成 Bytes，已包含 Annex B 0x00000001
                    let payload = Bytes::from(frame.into_data());

                    let packet = Arc::new(EncodedPacket {
                        pts_ms,
                        is_keyframe,
                        codec,
                        payload,
                    });

                    // 广播分发至所有订阅者（StreamHub / RingBuffer / FlvPipeline）
                    let _ = self.tx.send(packet);
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_rtsp_url_with_credentials() {
        let raw = "rtsp://admin:secret123@192.168.1.100:554/live/ch0";
        let (url, creds) = sanitize_rtsp_url_and_credentials(raw).expect("should sanitize");

        assert_eq!(url.username(), "");
        assert!(url.password().is_none());
        assert_eq!(url.as_str(), "rtsp://192.168.1.100:554/live/ch0");

        let creds = creds.expect("credentials should be extracted");
        assert_eq!(creds.username, "admin");
        assert_eq!(creds.password, "secret123");
    }

    #[test]
    fn test_sanitize_rtsp_url_with_reserved_characters() {
        // 密码包含 @, :, #, ? 等保留字符
        let raw = "rtsp://admin:p@ss:word#123?auth=ok@192.168.1.100:554/live/ch0";
        let (url, creds) = sanitize_rtsp_url_and_credentials(raw).expect("should sanitize");

        assert_eq!(url.username(), "");
        assert!(url.password().is_none());
        assert_eq!(url.as_str(), "rtsp://192.168.1.100:554/live/ch0");

        let creds = creds.expect("credentials should be extracted");
        assert_eq!(creds.username, "admin");
        assert_eq!(creds.password, "p@ss:word#123?auth=ok");
    }

    #[test]
    fn test_sanitize_rtsp_url_without_credentials() {
        let raw = "rtsp://192.168.1.200:8554/stream1";
        let (url, creds) = sanitize_rtsp_url_and_credentials(raw).expect("should sanitize");

        assert_eq!(url.username(), "");
        assert!(url.password().is_none());
        assert_eq!(url.as_str(), "rtsp://192.168.1.200:8554/stream1");
        assert!(creds.is_none());
    }

    #[test]
    fn test_map_transport_policy() {
        let udp_transport = map_transport_policy(TransportPolicy::Udp);
        assert!(matches!(udp_transport, Transport::Udp(_)));

        let tcp_transport = map_transport_policy(TransportPolicy::Tcp);
        assert!(matches!(tcp_transport, Transport::Tcp(_)));

        let auto_transport = map_transport_policy(TransportPolicy::Auto);
        assert!(matches!(auto_transport, Transport::Tcp(_)));
    }

    #[test]
    fn test_parse_video_codec() {
        assert_eq!(
            parse_video_codec("H264").expect("parse H264"),
            CodecType::H264
        );
        assert_eq!(
            parse_video_codec("h264").expect("parse h264"),
            CodecType::H264
        );
        assert_eq!(
            parse_video_codec("AVC").expect("parse AVC"),
            CodecType::H264
        );
        assert_eq!(
            parse_video_codec("H265").expect("parse H265"),
            CodecType::H265
        );
        assert_eq!(
            parse_video_codec("hevc").expect("parse hevc"),
            CodecType::H265
        );
        assert_eq!(
            parse_video_codec("HEVC").expect("parse HEVC"),
            CodecType::H265
        );

        assert!(matches!(
            parse_video_codec("JPEG"),
            Err(MediaError::UnsupportedCodec(_))
        ));
        assert!(matches!(
            parse_video_codec("VP9"),
            Err(MediaError::UnsupportedCodec(_))
        ));
    }
}
