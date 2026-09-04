use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use bytes::{Buf, Bytes, BytesMut};
use md5::{Digest, Md5};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::broadcast;
use types::{CodecType, EncodedPacket, TransportPolicy};

use crate::error::MediaError;

/// RTSP Digest 鉴权挑战结构体 (RFC 2617 / RFC 2068)
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DigestAuthChallenge {
    pub realm: String,
    pub nonce: String,
    pub opaque: Option<String>,
}

impl DigestAuthChallenge {
    pub fn from_response(resp: &str) -> Option<Self> {
        for line in resp.lines() {
            let trimmed = line.trim();
            if trimmed.to_lowercase().starts_with("www-authenticate:") {
                let auth_val = trimmed["www-authenticate:".len()..].trim();
                if auth_val.to_lowercase().starts_with("digest") {
                    let params = &auth_val[6..].trim();
                    let mut realm = String::new();
                    let mut nonce = String::new();
                    let mut opaque = None;

                    for part in params.split(',') {
                        if let Some((k, v)) = part.split_once('=') {
                            let key = k.trim().to_lowercase();
                            let val = v.trim().trim_matches('"').to_string();
                            match key.as_str() {
                                "realm" => realm = val,
                                "nonce" => nonce = val,
                                "opaque" => opaque = Some(val),
                                _ => {}
                            }
                        }
                    }

                    if !realm.is_empty() && !nonce.is_empty() {
                        return Some(Self {
                            realm,
                            nonce,
                            opaque,
                        });
                    }
                }
            }
        }
        None
    }

    pub fn build_auth_header(&self, user: &str, pass: &str, method: &str, uri: &str) -> String {
        // HA1 = MD5(username:realm:password)
        let mut hasher1 = Md5::new();
        hasher1.update(format!("{user}:{}:{pass}", self.realm).as_bytes());
        let ha1 = format!("{:x}", hasher1.finalize());

        // HA2 = MD5(method:uri)
        let mut hasher2 = Md5::new();
        hasher2.update(format!("{method}:{uri}").as_bytes());
        let ha2 = format!("{:x}", hasher2.finalize());

        // response = MD5(HA1:nonce:HA2)
        let mut hasher3 = Md5::new();
        hasher3.update(format!("{ha1}:{}:{ha2}", self.nonce).as_bytes());
        let response = format!("{:x}", hasher3.finalize());

        let mut header = format!(
            "Authorization: Digest username=\"{user}\", realm=\"{}\", nonce=\"{}\", uri=\"{uri}\", response=\"{response}\"",
            self.realm, self.nonce
        );
        if let Some(ref op) = self.opaque {
            header.push_str(&format!(", opaque=\"{op}\""));
        }
        header.push_str("\r\n");
        header
    }
}

/// H.264 RTP 解包重组状态
#[derive(Debug, Default)]
struct H264Depacketizer {
    fu_buffer: BytesMut,
    fu_start_pts: i64,
    fu_is_keyframe: bool,
}

impl H264Depacketizer {
    fn process_rtp(&mut self, payload: &[u8], pts_ms: i64) -> Option<Vec<EncodedPacket>> {
        if payload.is_empty() {
            return None;
        }

        let nal_header = payload[0];
        let nal_type = nal_header & 0x1F;
        let nri = (nal_header >> 5) & 0x03;

        let mut packets = Vec::new();

        match nal_type {
            // 单一 NALU (Single NAL Unit Packet)
            1..=23 => {
                let is_keyframe = nal_type == 5 || nal_type == 7 || nal_type == 8;
                packets.push(EncodedPacket {
                    pts_ms,
                    is_keyframe,
                    codec: CodecType::H264,
                    payload: Bytes::copy_from_slice(payload),
                });
            }
            // STAP-A 组合包 (Aggregation Packet)
            24 => {
                let mut data = &payload[1..];
                while data.len() >= 2 {
                    let nalu_len = ((data[0] as usize) << 8) | (data[1] as usize);
                    data = &data[2..];
                    if data.len() < nalu_len {
                        break;
                    }
                    let nalu = &data[..nalu_len];
                    data = &data[nalu_len..];

                    if !nalu.is_empty() {
                        let sub_type = nalu[0] & 0x1F;
                        let is_keyframe = sub_type == 5 || sub_type == 7 || sub_type == 8;
                        packets.push(EncodedPacket {
                            pts_ms,
                            is_keyframe,
                            codec: CodecType::H264,
                            payload: Bytes::copy_from_slice(nalu),
                        });
                    }
                }
            }
            // FU-A 分片包 (Fragmentation Unit)
            28 => {
                if payload.len() < 2 {
                    return None;
                }
                let fu_header = payload[1];
                let start_bit = (fu_header & 0x80) != 0;
                let end_bit = (fu_header & 0x40) != 0;
                let original_nal_type = fu_header & 0x1F;
                let reconstructed_nal_header = (nri << 5) | original_nal_type;

                if start_bit {
                    self.fu_buffer.clear();
                    self.fu_buffer.reserve(payload.len() + 4096);
                    self.fu_buffer
                        .extend_from_slice(&[reconstructed_nal_header]);
                    self.fu_buffer.extend_from_slice(&payload[2..]);
                    self.fu_start_pts = pts_ms;
                    self.fu_is_keyframe = original_nal_type == 5;
                } else if !self.fu_buffer.is_empty() {
                    self.fu_buffer.extend_from_slice(&payload[2..]);
                    if end_bit {
                        let full_nalu = self.fu_buffer.split().freeze();
                        packets.push(EncodedPacket {
                            pts_ms: self.fu_start_pts,
                            is_keyframe: self.fu_is_keyframe,
                            codec: CodecType::H264,
                            payload: full_nalu,
                        });
                    }
                }
            }
            _ => {}
        }

        if packets.is_empty() {
            None
        } else {
            Some(packets)
        }
    }
}

/// 纯 Rust 异步 RTSP 拉流与解包器
#[derive(Debug)]
pub struct RtspIngestor {
    pub camera_id: String,
    pub rtsp_url: String,
    pub transport_policy: TransportPolicy,
    pub tx: broadcast::Sender<Arc<EncodedPacket>>,
    pub is_running: Arc<AtomicBool>,
}

impl RtspIngestor {
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

    /// 启动拉流任务循环（具备指数退避自愈与优雅退出）
    pub async fn run_loop(self: Arc<Self>, cancel_signal: Arc<AtomicBool>) {
        self.is_running.store(true, Ordering::SeqCst);
        let mut backoff = Duration::from_secs(1);
        let max_backoff = Duration::from_secs(30);

        tracing::info!(camera_id = %self.camera_id, url = %self.rtsp_url, "RTSP Ingestor 启动");

        while !cancel_signal.load(Ordering::Relaxed) {
            match self.stream_session(&cancel_signal).await {
                Ok(()) => {
                    tracing::info!(camera_id = %self.camera_id, "RTSP 拉流正常关闭或挂起");
                    break;
                }
                Err(err) => {
                    if cancel_signal.load(Ordering::Relaxed) {
                        break;
                    }
                    tracing::warn!(
                        camera_id = %self.camera_id,
                        error = ?err,
                        retry_after_secs = backoff.as_secs(),
                        "RTSP 拉流异常中断，准备指数退避重连"
                    );
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(max_backoff);
                }
            }
        }

        self.is_running.store(false, Ordering::SeqCst);
        tracing::info!(camera_id = %self.camera_id, "RTSP Ingestor 已停止");
    }

    async fn stream_session(&self, cancel_signal: &AtomicBool) -> Result<(), MediaError> {
        let parsed_url = url::Url::parse(&self.rtsp_url).map_err(|e| MediaError::RtspConnect {
            url: self.rtsp_url.clone(),
            reason: format!("URL 格式不合法: {e}"),
        })?;

        let host = parsed_url
            .host_str()
            .ok_or_else(|| MediaError::RtspConnect {
                url: self.rtsp_url.clone(),
                reason: "缺少主机地址".into(),
            })?;
        let port = parsed_url.port().unwrap_or(554);
        let addr = format!("{host}:{port}");

        let mut stream = TcpStream::connect(&addr)
            .await
            .map_err(|e| MediaError::RtspConnect {
                url: self.rtsp_url.clone(),
                reason: format!("TCP 连接失败 ({addr}): {e}"),
            })?;

        let username = parsed_url.username().to_string();
        let password = parsed_url.password().unwrap_or("").to_string();
        let has_auth = !username.is_empty();

        let mut digest_auth: Option<DigestAuthChallenge> = None;

        let get_auth_header =
            |digest: &Option<DigestAuthChallenge>, method: &str, uri: &str| -> String {
                if !has_auth {
                    return String::new();
                }
                if let Some(ref d) = digest {
                    d.build_auth_header(&username, &password, method, uri)
                } else {
                    let user_pass = format!("{username}:{password}");
                    let encoded = base64::engine::general_purpose::STANDARD.encode(user_pass);
                    format!("Authorization: Basic {encoded}\r\n")
                }
            };

        // 1. OPTIONS 握手
        let mut cseq = 1;
        let mut auth_hdr = get_auth_header(&digest_auth, "OPTIONS", &self.rtsp_url);
        let req = format!(
            "OPTIONS {} RTSP/1.0\r\nCSeq: {}\r\n{}User-Agent: Heimdall/1.0\r\n\r\n",
            self.rtsp_url, cseq, auth_hdr
        );
        stream.write_all(req.as_bytes()).await?;
        let mut buf = vec![0u8; 4096];
        let n = stream.read(&mut buf).await?;
        let resp = String::from_utf8_lossy(&buf[..n]);

        if resp.contains("401 Unauthorized") {
            if let Some(challenge) = DigestAuthChallenge::from_response(&resp) {
                digest_auth = Some(challenge);
                cseq += 1;
                auth_hdr = get_auth_header(&digest_auth, "OPTIONS", &self.rtsp_url);
                let req = format!(
                    "OPTIONS {} RTSP/1.0\r\nCSeq: {}\r\n{}User-Agent: Heimdall/1.0\r\n\r\n",
                    self.rtsp_url, cseq, auth_hdr
                );
                stream.write_all(req.as_bytes()).await?;
                let n = stream.read(&mut buf).await?;
                let resp = String::from_utf8_lossy(&buf[..n]);
                if !resp.contains("200 OK") && !resp.starts_with("RTSP/1.0 200") {
                    return Err(MediaError::Protocol(format!(
                        "OPTIONS 握手失败 (Digest 鉴权后): {resp}"
                    )));
                }
            } else {
                return Err(MediaError::RtspConnect {
                    url: self.rtsp_url.clone(),
                    reason: "RTSP 鉴权失败 (401 Unauthorized)".into(),
                });
            }
        } else if !resp.contains("200 OK") && !resp.starts_with("RTSP/1.0 200") {
            return Err(MediaError::Protocol(format!("OPTIONS 握手失败: {resp}")));
        }

        // 2. DESCRIBE 握手
        cseq += 1;
        auth_hdr = get_auth_header(&digest_auth, "DESCRIBE", &self.rtsp_url);
        let req = format!(
            "DESCRIBE {} RTSP/1.0\r\nCSeq: {}\r\n{}Accept: application/sdp\r\nUser-Agent: Heimdall/1.0\r\n\r\n",
            self.rtsp_url, cseq, auth_hdr
        );
        stream.write_all(req.as_bytes()).await?;
        let mut sdp_buf = vec![0u8; 8192];
        let n = stream.read(&mut sdp_buf).await?;
        let mut sdp_resp = String::from_utf8_lossy(&sdp_buf[..n]).to_string();

        if sdp_resp.contains("401 Unauthorized") {
            if let Some(challenge) = DigestAuthChallenge::from_response(&sdp_resp) {
                digest_auth = Some(challenge);
                cseq += 1;
                auth_hdr = get_auth_header(&digest_auth, "DESCRIBE", &self.rtsp_url);
                let req = format!(
                    "DESCRIBE {} RTSP/1.0\r\nCSeq: {}\r\n{}Accept: application/sdp\r\nUser-Agent: Heimdall/1.0\r\n\r\n",
                    self.rtsp_url, cseq, auth_hdr
                );
                stream.write_all(req.as_bytes()).await?;
                let n = stream.read(&mut sdp_buf).await?;
                sdp_resp = String::from_utf8_lossy(&sdp_buf[..n]).to_string();
            }
        }

        if !sdp_resp.contains("200 OK") && !sdp_resp.starts_with("RTSP/1.0 200") {
            return Err(MediaError::Protocol(format!(
                "DESCRIBE 握手失败: {sdp_resp}"
            )));
        }

        // 提取控制 URL (trackID / streamid)
        let control_url = Self::extract_control_url(&sdp_resp, &self.rtsp_url);

        // 3. SETUP 握手 (TCP Interleaved 传输)
        cseq += 1;
        auth_hdr = get_auth_header(&digest_auth, "SETUP", &control_url);
        let req = format!(
            "SETUP {} RTSP/1.0\r\nCSeq: {}\r\n{}Transport: RTP/AVP/TCP;unicast;interleaved=0-1\r\nUser-Agent: Heimdall/1.0\r\n\r\n",
            control_url, cseq, auth_hdr
        );
        stream.write_all(req.as_bytes()).await?;
        let mut setup_buf = vec![0u8; 4096];
        let n = stream.read(&mut setup_buf).await?;
        let setup_resp = String::from_utf8_lossy(&setup_buf[..n]);

        let session_id = Self::extract_session_id(&setup_resp)
            .ok_or_else(|| MediaError::Protocol("SETUP 响应中缺少 Session ID".into()))?;

        // 4. PLAY 握手
        cseq += 1;
        auth_hdr = get_auth_header(&digest_auth, "PLAY", &self.rtsp_url);
        let req = format!(
            "PLAY {} RTSP/1.0\r\nCSeq: {}\r\nSession: {}\r\n{}Range: npt=0.000-\r\nUser-Agent: Heimdall/1.0\r\n\r\n",
            self.rtsp_url, cseq, session_id, auth_hdr
        );
        stream.write_all(req.as_bytes()).await?;
        let mut play_buf = vec![0u8; 4096];
        let _ = stream.read(&mut play_buf).await?;

        tracing::info!(camera_id = %self.camera_id, "RTSP 握手完成，进入 TCP Interleaved 码流接收循环");

        // 5. TCP Interleaved 循环读取: $ (1B) + Channel (1B) + Length (2B) + RTP
        let mut depacketizer = H264Depacketizer::default();
        let mut stream_buf = BytesMut::with_capacity(65536);

        let mut read_scratch = [0u8; 8192];
        let base_timestamp_ms = chrono::Utc::now().timestamp_millis();
        let mut first_rtp_ts: Option<u32> = None;

        while !cancel_signal.load(Ordering::Relaxed) {
            let bytes_read = stream.read(&mut read_scratch).await?;
            if bytes_read == 0 {
                return Err(MediaError::RtspConnect {
                    url: self.rtsp_url.clone(),
                    reason: "RTSP TCP 连接对端已关闭 (EOF)".into(),
                });
            }
            stream_buf.extend_from_slice(&read_scratch[..bytes_read]);

            // 解析可能交错的一个或多个 Interleaved 数据帧
            while stream_buf.len() >= 4 {
                if stream_buf[0] != 0x24 {
                    // 寻找下一个 '$' 起始字节
                    if let Some(pos) = stream_buf.iter().position(|&b| b == 0x24) {
                        stream_buf.advance(pos);
                    } else {
                        stream_buf.clear();
                        break;
                    }
                }

                if stream_buf.len() < 4 {
                    break;
                }

                let channel = stream_buf[1];
                let length = ((stream_buf[2] as usize) << 8) | (stream_buf[3] as usize);

                if stream_buf.len() < 4 + length {
                    // 缓冲未收满一个完整 RTP 包，等待下一次 read
                    break;
                }

                // 消耗 4 字节头部
                stream_buf.advance(4);
                let packet_bytes = stream_buf.split_to(length);

                // 只处理 Video 通道 (Channel 0)
                if channel == 0 && packet_bytes.len() >= 12 {
                    let rtp_data = &packet_bytes[..];
                    let rtp_ts = ((rtp_data[4] as u32) << 24)
                        | ((rtp_data[5] as u32) << 16)
                        | ((rtp_data[6] as u32) << 8)
                        | (rtp_data[7] as u32);

                    let first_ts = *first_rtp_ts.get_or_insert(rtp_ts);
                    let delta_ts = rtp_ts.wrapping_sub(first_ts);
                    let pts_ms = base_timestamp_ms + ((delta_ts as i64) * 1000 / 90000);

                    // 跳过 12 字节固定 RTP 头部与可能的 CSRC/Extension
                    let cc = (rtp_data[0] & 0x0F) as usize;
                    let mut payload_offset = 12 + cc * 4;
                    if (rtp_data[0] & 0x10) != 0 && rtp_data.len() >= payload_offset + 4 {
                        // 有扩展头
                        let ext_len = (((rtp_data[payload_offset + 2] as usize) << 8)
                            | (rtp_data[payload_offset + 3] as usize))
                            * 4;
                        payload_offset += 4 + ext_len;
                    }

                    if payload_offset < rtp_data.len() {
                        let rtp_payload = &rtp_data[payload_offset..];
                        if let Some(encoded_packets) = depacketizer.process_rtp(rtp_payload, pts_ms)
                        {
                            for pkt in encoded_packets {
                                // 广播分发，遇慢消费者自动丢弃旧包
                                let _ = self.tx.send(Arc::new(pkt));
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn extract_control_url(sdp: &str, rtsp_url: &str) -> String {
        let mut in_video_media = false;
        for line in sdp.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("m=video") {
                in_video_media = true;
                continue;
            }
            if in_video_media && trimmed.starts_with("m=") {
                in_video_media = false;
            }
            if in_video_media && trimmed.starts_with("a=control:") {
                let control_val = &trimmed["a=control:".len()..];
                if control_val.starts_with("rtsp://") {
                    return control_val.to_string();
                } else if control_val == "*" {
                    return rtsp_url.to_string();
                } else if rtsp_url.ends_with('/') {
                    return format!("{rtsp_url}{control_val}");
                } else {
                    return format!("{rtsp_url}/{control_val}");
                }
            }
        }
        rtsp_url.to_string()
    }

    fn extract_session_id(setup_resp: &str) -> Option<String> {
        for line in setup_resp.lines() {
            let trimmed = line.trim();
            if trimmed.to_lowercase().starts_with("session:") {
                let val = &trimmed["session:".len()..].trim();
                let session_id = val.split(';').next().unwrap_or("").trim();
                if !session_id.is_empty() {
                    return Some(session_id.to_string());
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_digest_auth_challenge_parse_and_build() {
        let resp = "RTSP/1.0 401 Unauthorized\r\nCSeq: 1\r\nWWW-Authenticate: Digest realm=\"IP Camera(C6523)\", nonce=\"4b340000000000000000\"\r\n\r\n";
        let challenge = DigestAuthChallenge::from_response(resp).expect("parse digest challenge");
        assert_eq!(challenge.realm, "IP Camera(C6523)");
        assert_eq!(challenge.nonce, "4b340000000000000000");

        let auth_hdr = challenge.build_auth_header(
            "admin",
            "12345",
            "OPTIONS",
            "rtsp://192.168.1.100:554/live/ch0",
        );
        assert!(auth_hdr.starts_with("Authorization: Digest"));
        assert!(auth_hdr.contains("username=\"admin\""));
        assert!(auth_hdr.contains("realm=\"IP Camera(C6523)\""));
        assert!(auth_hdr.contains("response="));
    }

    #[test]
    fn test_h264_single_nalu_depacketize() {
        let mut depack = H264Depacketizer::default();
        // IDR NALU: 0x65 (type 5)
        let payload = vec![0x65, 0x88, 0x84, 0x00, 0x33];
        let pkts = depack
            .process_rtp(&payload, 1000)
            .expect("depacketize single");
        assert_eq!(pkts.len(), 1);
        assert!(pkts[0].is_keyframe);
        assert_eq!(pkts[0].payload.as_ref(), &[0x65, 0x88, 0x84, 0x00, 0x33]);
    }

    #[test]
    fn test_h264_stap_a_depacketize() {
        let mut depack = H264Depacketizer::default();
        // STAP-A (0x78) 包含 SPS (0x67, 3 bytes) 和 PPS (0x68, 2 bytes)
        let payload = vec![
            0x78, // STAP-A Header
            0x00, 0x03, 0x67, 0x42, 0x00, // NALU 1 (SPS)
            0x00, 0x02, 0x68, 0xCE, // NALU 2 (PPS)
        ];
        let pkts = depack
            .process_rtp(&payload, 2000)
            .expect("depacketize stap-a");
        assert_eq!(pkts.len(), 2);
        assert!(pkts[0].is_keyframe); // SPS
        assert_eq!(pkts[0].payload.as_ref(), &[0x67, 0x42, 0x00]);
        assert_eq!(pkts[1].payload.as_ref(), &[0x68, 0xCE]);
    }

    #[test]
    fn test_h264_fu_a_fragmentation_reassembly() {
        let mut depack = H264Depacketizer::default();

        // 1. FU-A Start packet (NAL type 28 = 0x7C, Start bit + original type 5 IDR = 0x85)
        let part1 = vec![0x7C, 0x85, 0x11, 0x22];
        let res1 = depack.process_rtp(&part1, 3000);
        assert!(res1.is_none());

        // 2. FU-A Middle packet
        let part2 = vec![0x7C, 0x05, 0x33, 0x44];
        let res2 = depack.process_rtp(&part2, 3000);
        assert!(res2.is_none());

        // 3. FU-A End packet (End bit = 0x45)
        let part3 = vec![0x7C, 0x45, 0x55, 0x66];
        let res3 = depack.process_rtp(&part3, 3000).expect("fu-a complete");
        assert_eq!(res3.len(), 1);
        assert!(res3[0].is_keyframe);
        // 重建头部: (3 << 5) | 5 = 0x65
        assert_eq!(
            res3[0].payload.as_ref(),
            &[0x65, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66]
        );
    }
}
