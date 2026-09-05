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

/// 默认数据流静默无报文看门狗超时（安防摄像机通常为 15~30fps，连续 6 秒无任何数据即可判定流假死或 TCP 半开）
pub const DEFAULT_STREAM_INACTIVITY_TIMEOUT: Duration = Duration::from_secs(6);

/// 默认 RTSP 单步握手超时时间（DESCRIBE / SETUP / PLAY 阶段防网络黑洞阻塞）
pub const DEFAULT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

/// 基于 Retina 引擎的生产级 RTSP 接入器
#[derive(Debug)]
pub struct RetinaIngestor {
    pub camera_id: String,
    pub rtsp_url: String,
    pub transport_policy: TransportPolicy,
    pub tx: broadcast::Sender<Arc<EncodedPacket>>,
    pub is_running: Arc<AtomicBool>,
    pub inactivity_timeout: Duration,
    pub handshake_timeout: Duration,
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
            inactivity_timeout: DEFAULT_STREAM_INACTIVITY_TIMEOUT,
            handshake_timeout: DEFAULT_HANDSHAKE_TIMEOUT,
        }
    }

    /// 设置流静默看门狗超时时间（连续未收到任何音视频数据包的判定阈值）
    pub fn with_inactivity_timeout(mut self, timeout: Duration) -> Self {
        self.inactivity_timeout = timeout;
        self
    }

    /// 设置单步握手超时时间（DESCRIBE / SETUP / PLAY 阶段超时）
    pub fn with_handshake_timeout(mut self, timeout: Duration) -> Self {
        self.handshake_timeout = timeout;
        self
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

        // 2. 发起 DESCRIBE 握手并解析 SDP（包装握手超时与取消信号监听）
        let mut session = tokio::select! {
            biased;
            change_res = cancel_rx.changed() => {
                if change_res.is_err() || *cancel_rx.borrow() {
                    return Ok(());
                }
                return Err((
                    MediaError::RtspConnect {
                        url: masked_url.clone(),
                        reason: "Retina DESCRIBE 阶段被取消中断".into(),
                    },
                    0,
                ));
            }
            res = tokio::time::timeout(
                self.handshake_timeout,
                Session::describe(clean_url, session_options),
            ) => {
                res.map_err(|_| {
                    (
                        MediaError::RtspConnect {
                            url: masked_url.clone(),
                            reason: format!(
                                "Retina DESCRIBE 握手超时 (超过 {}s 未收到响应)",
                                self.handshake_timeout.as_secs()
                            ),
                        },
                        0,
                    )
                })?
                .map_err(|e| {
                    (
                        MediaError::RtspConnect {
                            url: masked_url.clone(),
                            reason: format!("Retina DESCRIBE 握手失败: {e}"),
                        },
                        0,
                    )
                })?
            }
        };

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

        tokio::select! {
            biased;
            change_res = cancel_rx.changed() => {
                if change_res.is_err() || *cancel_rx.borrow() {
                    return Ok(());
                }
                return Err((
                    MediaError::Protocol("Retina SETUP 阶段被取消中断".into()),
                    0,
                ));
            }
            res = tokio::time::timeout(
                self.handshake_timeout,
                session.setup(video_idx, setup_options),
            ) => {
                res.map_err(|_| {
                    (
                        MediaError::Protocol(format!(
                            "Retina SETUP 阶段超时 (超过 {}s 未收到响应)",
                            self.handshake_timeout.as_secs()
                        )),
                        0,
                    )
                })?
                .map_err(|e| {
                    (
                        MediaError::Protocol(format!("Retina SETUP 阶段失败: {e}")),
                        0,
                    )
                })?;
            }
        }

        // 5. 执行 PLAY 握手并获取解复用流
        let playing_session = tokio::select! {
            biased;
            change_res = cancel_rx.changed() => {
                if change_res.is_err() || *cancel_rx.borrow() {
                    return Ok(());
                }
                return Err((
                    MediaError::Protocol("Retina PLAY 阶段被取消中断".into()),
                    0,
                ));
            }
            res = tokio::time::timeout(
                self.handshake_timeout,
                session.play(PlayOptions::default()),
            ) => {
                res.map_err(|_| {
                    (
                        MediaError::Protocol(format!(
                            "Retina PLAY 阶段超时 (超过 {}s 未收到响应)",
                            self.handshake_timeout.as_secs()
                        )),
                        0,
                    )
                })?
                .map_err(|e| {
                    (
                        MediaError::Protocol(format!("Retina PLAY 阶段失败: {e}")),
                        0,
                    )
                })?
            }
        };

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

        // 6. 持续消费 VideoFrame（看门狗守护，防止半开连接与静默丢包死锁）
        while !cancel_signal.load(Ordering::Relaxed) && !*cancel_rx.borrow() {
            let next_item = tokio::select! {
                biased;
                change_res = cancel_rx.changed() => {
                    if change_res.is_err() || *cancel_rx.borrow() {
                        break;
                    }
                    continue;
                }
                timed_item = tokio::time::timeout(self.inactivity_timeout, demuxed.next()) => {
                    match timed_item {
                        Ok(item) => item,
                        Err(_) => {
                            tracing::warn!(
                                camera_id = %self.camera_id,
                                timeout_secs = self.inactivity_timeout.as_secs(),
                                frames_received,
                                "Retina RTSP 数据流静默超时（未收到数据包，判定为 TCP 半开或网络假死），主动触发清理与重连"
                            );
                            return Err((
                                MediaError::InactivityTimeout(self.inactivity_timeout),
                                frames_received,
                            ));
                        }
                    }
                }
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

    #[test]
    fn test_retina_ingestor_builder_and_defaults() {
        let (tx, _) = broadcast::channel(16);
        let ingestor = RetinaIngestor::new(
            "cam-1".into(),
            "rtsp://127.0.0.1:554/live".into(),
            TransportPolicy::Auto,
            tx.clone(),
        );

        assert_eq!(
            ingestor.inactivity_timeout,
            DEFAULT_STREAM_INACTIVITY_TIMEOUT
        );
        assert_eq!(ingestor.handshake_timeout, DEFAULT_HANDSHAKE_TIMEOUT);

        let customized = ingestor
            .with_inactivity_timeout(Duration::from_secs(8))
            .with_handshake_timeout(Duration::from_millis(500));

        assert_eq!(customized.inactivity_timeout, Duration::from_secs(8));
        assert_eq!(customized.handshake_timeout, Duration::from_millis(500));
    }

    #[test]
    fn test_media_error_inactivity_timeout() {
        let err = MediaError::InactivityTimeout(Duration::from_secs(6));
        assert_eq!(err.error_code(), 20011);
        assert!(err.to_string().contains("媒体流静默超时"));
    }

    #[tokio::test]
    async fn test_handshake_timeout_on_silent_socket() {
        // 模拟工控安防现场：网络可通但 IPC 服务死锁无响应（Silent Socket）
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock listener");
        let port = listener.local_addr().expect("local addr").port();

        // 仅 accept 连接，不写入任何 RTSP 响应
        tokio::spawn(async move {
            if let Ok((_sock, _)) = listener.accept().await {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });

        let (tx, _) = broadcast::channel(16);
        let ingestor = RetinaIngestor::new(
            "cam-hang".into(),
            format!("rtsp://127.0.0.1:{port}/live"),
            TransportPolicy::Tcp,
            tx,
        )
        .with_handshake_timeout(Duration::from_millis(100));

        let cancel_signal = Arc::new(AtomicBool::new(false));
        let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);

        let start = std::time::Instant::now();
        let res = ingestor
            .stream_session(TransportMode::Tcp, cancel_signal, cancel_rx)
            .await;

        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_millis(500),
            "握手必须在超时配置 (100ms) 内快速返回，实际耗时: {:?}",
            elapsed
        );

        match res {
            Err((MediaError::RtspConnect { reason, .. }, frames)) => {
                assert_eq!(frames, 0);
                assert!(
                    reason.contains("超时"),
                    "错误原因应明确标注握手超时: {reason}"
                );
            }
            other => panic!("期望超时错误，实际收到: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_inactivity_watchdog_triggers_on_silent_stream() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock listener");
        let port = listener.local_addr().expect("local addr").port();

        // 模拟标准 RTSP 握手成功后进入流传输阶段，但随后静默无数据（模拟 TCP 半开 / 摄像头死锁）
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 2048];

                // 1. 处理 DESCRIBE
                let n = sock.read(&mut buf).await.unwrap_or(0);
                if n > 0 {
                    let sdp = "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=Test\r\nt=0 0\r\nm=video 0 RTP/AVP 96\r\na=rtpmap:96 H264/90000\r\na=control:trackID=0\r\n";
                    let resp = format!(
                        "RTSP/1.0 200 OK\r\nCSeq: 1\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\n\r\n{}",
                        sdp.len(),
                        sdp
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                }

                // 2. 处理 SETUP
                let n = sock.read(&mut buf).await.unwrap_or(0);
                if n > 0 {
                    let resp = "RTSP/1.0 200 OK\r\nCSeq: 2\r\nSession: TESTSESSION123\r\nTransport: RTP/AVP/TCP;unicast;interleaved=0-1\r\n\r\n";
                    let _ = sock.write_all(resp.as_bytes()).await;
                }

                // 3. 处理 PLAY
                let n = sock.read(&mut buf).await.unwrap_or(0);
                if n > 0 {
                    let resp = "RTSP/1.0 200 OK\r\nCSeq: 3\r\nSession: TESTSESSION123\r\n\r\n";
                    let _ = sock.write_all(resp.as_bytes()).await;
                }

                // 4. 握手闭环完成后，模拟工控机网络半开 / IPC 静默挂死：保持 Socket 打开但绝不发送任何 RTP 数据包
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });

        let (tx, _) = broadcast::channel(16);
        let ingestor = RetinaIngestor::new(
            "cam-silent-drop".into(),
            format!("rtsp://127.0.0.1:{port}/live"),
            TransportPolicy::Tcp,
            tx,
        )
        .with_handshake_timeout(Duration::from_secs(2))
        .with_inactivity_timeout(Duration::from_millis(150)); // 设置为 150ms 方便单测断言

        let cancel_signal = Arc::new(AtomicBool::new(false));
        let (_cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);

        let start = std::time::Instant::now();
        let res = ingestor
            .stream_session(TransportMode::Tcp, cancel_signal, cancel_rx)
            .await;
        let elapsed = start.elapsed();

        // 验证耗时：握手完毕后在 150ms 左右由看门狗触发超时返回，绝不无限挂起
        assert!(
            elapsed < Duration::from_millis(800),
            "流静默看门狗应在 150ms 阈值附近触发，实际耗时: {:?}",
            elapsed
        );

        match res {
            Err((MediaError::InactivityTimeout(dur), frames)) => {
                assert_eq!(dur, Duration::from_millis(150));
                assert_eq!(frames, 0);
            }
            other => panic!(
                "期望返回 MediaError::InactivityTimeout，实际收到: {:?}",
                other
            ),
        }
    }

    #[tokio::test]
    async fn test_cancellation_during_handshake() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock listener");
        let port = listener.local_addr().expect("local addr").port();

        tokio::spawn(async move {
            if let Ok((_sock, _)) = listener.accept().await {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });

        let (tx, _) = broadcast::channel(16);
        let ingestor = RetinaIngestor::new(
            "cam-cancel".into(),
            format!("rtsp://127.0.0.1:{port}/live"),
            TransportPolicy::Tcp,
            tx,
        )
        .with_handshake_timeout(Duration::from_secs(5));

        let cancel_signal = Arc::new(AtomicBool::new(false));
        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);

        // 延迟 50ms 发送取消信号
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let _ = cancel_tx.send(true);
        });

        let start = std::time::Instant::now();
        let res = ingestor
            .stream_session(TransportMode::Tcp, cancel_signal, cancel_rx)
            .await;

        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_millis(500),
            "收到取消信号后应立即返回，耗时: {:?}",
            elapsed
        );
        assert!(res.is_ok(), "取消时应正常平稳退出返回 Ok: {:?}", res);
    }
}
