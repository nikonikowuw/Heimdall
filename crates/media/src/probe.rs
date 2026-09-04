use base64::Engine;
use bytes::BytesMut;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

use crate::error::MediaError;
use crate::rtsp::{mask_rtsp_url, read_rtsp_response, DigestAuthChallenge};
use crate::sps::{parse_h264_sps, parse_h265_sps, SpsInfo};

/// 视频流探活信息
#[derive(Debug, Clone, PartialEq)]
pub struct StreamInfo {
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
}

impl StreamInfo {
    /// 检查分辨率与帧率是否已通过 SPS 明确解析
    pub fn is_dimension_known(&self) -> bool {
        self.width > 0 && self.height > 0
    }
}

impl From<SpsInfo> for StreamInfo {
    fn from(sps: SpsInfo) -> Self {
        Self {
            codec: sps.codec,
            width: sps.width,
            height: sps.height,
            fps: sps.fps,
        }
    }
}

/// 摄像头码流测活探针
#[derive(Debug, Default)]
pub struct StreamProber;

impl StreamProber {
    /// 针对 RTSP 地址发起带超时的轻量握手探活并提取宽高等元数据
    pub async fn probe(rtsp_url: &str, timeout: Duration) -> Result<StreamInfo, MediaError> {
        tokio::time::timeout(timeout, Self::probe_internal(rtsp_url))
            .await
            .map_err(|_| MediaError::ProbeTimeout(timeout))?
    }

    async fn probe_internal(rtsp_url: &str) -> Result<StreamInfo, MediaError> {
        let masked_url = mask_rtsp_url(rtsp_url);
        let parsed_url = url::Url::parse(rtsp_url).map_err(|e| MediaError::RtspConnect {
            url: masked_url.clone(),
            reason: format!("URL 格式不合法: {e}"),
        })?;

        let host = parsed_url
            .host_str()
            .ok_or_else(|| MediaError::RtspConnect {
                url: masked_url.clone(),
                reason: "缺少主机地址".into(),
            })?;
        let port = parsed_url.port().unwrap_or(554);
        let addr = format!("{host}:{port}");

        let mut stream = TcpStream::connect(&addr)
            .await
            .map_err(|e| MediaError::RtspConnect {
                url: masked_url.clone(),
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

        let mut read_buf = BytesMut::with_capacity(8192);

        // 1. 发送 OPTIONS 检查可达性
        let mut cseq = 1;
        let mut auth_hdr = get_auth_header(&digest_auth, "OPTIONS", rtsp_url);
        let options_req = format!(
            "OPTIONS {rtsp_url} RTSP/1.0\r\nCSeq: {}\r\n{}User-Agent: Heimdall/1.0\r\n\r\n",
            cseq, auth_hdr
        );
        stream.write_all(options_req.as_bytes()).await?;
        let resp = read_rtsp_response(&mut stream, &mut read_buf).await?;

        if resp.contains("401 Unauthorized") {
            if let Some(challenge) = DigestAuthChallenge::from_response(&resp) {
                digest_auth = Some(challenge);
                cseq += 1;
                auth_hdr = get_auth_header(&digest_auth, "OPTIONS", rtsp_url);
                let options_req = format!(
                    "OPTIONS {rtsp_url} RTSP/1.0\r\nCSeq: {}\r\n{}User-Agent: Heimdall/1.0\r\n\r\n",
                    cseq, auth_hdr
                );
                stream.write_all(options_req.as_bytes()).await?;
                let resp = read_rtsp_response(&mut stream, &mut read_buf).await?;
                if !resp.starts_with("RTSP/1.0 200") && !resp.contains("200 OK") {
                    return Err(MediaError::RtspConnect {
                        url: masked_url.clone(),
                        reason: "RTSP Digest 鉴权失败 (401 Unauthorized)".into(),
                    });
                }
            } else {
                return Err(MediaError::RtspConnect {
                    url: masked_url.clone(),
                    reason: "RTSP 鉴权失败 (401 Unauthorized)".into(),
                });
            }
        } else if !resp.starts_with("RTSP/1.0 200") && !resp.contains("200 OK") {
            return Err(MediaError::RtspConnect {
                url: masked_url.clone(),
                reason: format!("OPTIONS 握手失败: {resp}"),
            });
        }

        // 2. 发送 DESCRIBE 请求拉取 SDP (利用流式读取防止跨包截断)
        cseq += 1;
        auth_hdr = get_auth_header(&digest_auth, "DESCRIBE", rtsp_url);
        let describe_req = format!(
            "DESCRIBE {rtsp_url} RTSP/1.0\r\nCSeq: {}\r\n{}Accept: application/sdp\r\nUser-Agent: Heimdall/1.0\r\n\r\n",
            cseq, auth_hdr
        );
        stream.write_all(describe_req.as_bytes()).await?;
        let mut sdp_resp = read_rtsp_response(&mut stream, &mut read_buf).await?;

        if sdp_resp.contains("401 Unauthorized") {
            if let Some(challenge) = DigestAuthChallenge::from_response(&sdp_resp) {
                digest_auth = Some(challenge);
                cseq += 1;
                auth_hdr = get_auth_header(&digest_auth, "DESCRIBE", rtsp_url);
                let describe_req = format!(
                    "DESCRIBE {rtsp_url} RTSP/1.0\r\nCSeq: {}\r\n{}Accept: application/sdp\r\nUser-Agent: Heimdall/1.0\r\n\r\n",
                    cseq, auth_hdr
                );
                stream.write_all(describe_req.as_bytes()).await?;
                sdp_resp = read_rtsp_response(&mut stream, &mut read_buf).await?;
            }
        }

        // 校验 DESCRIBE 响应状态码是否为 200 OK
        if !sdp_resp.starts_with("RTSP/1.0 200") && !sdp_resp.contains("200 OK") {
            tracing::warn!(
                url = %masked_url,
                resp = %sdp_resp.lines().next().unwrap_or(""),
                "RTSP 探活 DESCRIBE 请求失败 (非 200 OK)"
            );
            return Err(MediaError::RtspConnect {
                url: masked_url.clone(),
                reason: format!(
                    "DESCRIBE 握手失败: {}",
                    sdp_resp.lines().next().unwrap_or("未知响应")
                ),
            });
        }

        // 严格校验 SDP 是否包含有效的视频轨道描述 (m=video)
        if !sdp_resp.contains("m=video") {
            tracing::warn!(url = %masked_url, "RTSP 探活未在响应中发现有效 SDP 视频描述 (缺少 m=video)");
            return Err(MediaError::RtspConnect {
                url: masked_url.clone(),
                reason: "SDP 描述中缺少视频轨 (m=video)".into(),
            });
        }

        if let Some(info) = Self::parse_sdp(&sdp_resp) {
            tracing::info!(
                url = %masked_url,
                codec = %info.codec,
                width = info.width,
                height = info.height,
                fps = info.fps,
                "RTSP 探活成功 (已从 SDP 解析出 SPS 宽高)"
            );
            return Ok(info);
        }

        let is_h265 =
            sdp_resp.to_uppercase().contains("H265") || sdp_resp.to_uppercase().contains("HEVC");
        tracing::info!(
            url = %masked_url,
            is_h265 = is_h265,
            "RTSP 探活成功 (SDP 包含有效视频轨，宽高参数待首帧解码确定)"
        );

        // 若从 SDP 中未能直接解析出 SPS，返回已知编码格式但宽高待定的状态 (0x0 @ 0fps)，杜绝伪造 1080P
        Ok(StreamInfo {
            codec: if is_h265 {
                "h265".to_string()
            } else {
                "h264".to_string()
            },
            width: 0,
            height: 0,
            fps: 0.0,
        })
    }

    /// 从 SDP 描述文本中提取 H.264 / H.265 SPS 并解析
    pub fn parse_sdp(sdp: &str) -> Option<StreamInfo> {
        let mut is_h265 = false;

        for line in sdp.lines() {
            let trimmed = line.trim();
            if trimmed.contains("H265") || trimmed.contains("HEVC") {
                is_h265 = true;
            }

            // 解析 H.264 的 sprop-parameter-sets
            if let Some(idx) = trimmed.find("sprop-parameter-sets=") {
                let params = &trimmed[idx + "sprop-parameter-sets=".len()..];
                let first_param = params
                    .split(',')
                    .next()
                    .unwrap_or("")
                    .split(';')
                    .next()
                    .unwrap_or("");
                if let Ok(sps_bytes) =
                    base64::engine::general_purpose::STANDARD.decode(first_param.trim())
                {
                    if let Ok(sps_info) = parse_h264_sps(&sps_bytes) {
                        return Some(sps_info.into());
                    }
                }
            }

            // 解析 H.265 的 sprop-sps
            if let Some(idx) = trimmed.find("sprop-sps=") {
                let params = &trimmed[idx + "sprop-sps=".len()..];
                let first_param = params
                    .split(';')
                    .next()
                    .unwrap_or("")
                    .split(',')
                    .next()
                    .unwrap_or("");
                if let Ok(sps_bytes) =
                    base64::engine::general_purpose::STANDARD.decode(first_param.trim())
                {
                    if let Ok(sps_info) = parse_h265_sps(&sps_bytes) {
                        return Some(sps_info.into());
                    }
                }
            }
        }

        if is_h265 {
            Some(StreamInfo {
                codec: "h265".to_string(),
                width: 0,
                height: 0,
                fps: 0.0,
            })
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_sdp_with_h264_sps() {
        let sdp = r#"
v=0
o=- 1747584000 1 IN IP4 127.0.0.1
s=RTSP Session
m=video 0 RTP/AVP 96
a=rtpmap:96 H264/90000
a=fmtp:96 packetization-mode=1;sprop-parameter-sets=Z2QAKacyhEB4AiflwEQAAAMABAAAAPDY8MYxWA==,aM48gA==
"#;

        let info = StreamProber::parse_sdp(sdp).expect("parse sdp sps");
        assert_eq!(info.codec, "h264");
        assert_eq!(info.width, 1920);
        assert_eq!(info.height, 1080);
        assert_eq!(info.fps.round() as u32, 30);
        assert!(info.is_dimension_known());
    }

    #[test]
    fn test_parse_sdp_fallback() {
        let sdp = "v=0\r\nm=video 0 RTP/AVP 96\r\na=rtpmap:96 H265/90000\r\n";
        let info = StreamProber::parse_sdp(sdp).expect("parse h265 sdp");
        assert_eq!(info.codec, "h265");
        assert_eq!(info.width, 0);
        assert_eq!(info.height, 0);
        assert_eq!(info.fps, 0.0);
        assert!(!info.is_dimension_known());
    }

    #[test]
    fn test_parse_sdp_with_h265_sprop_sps() {
        let sdp = r#"
v=0
o=- 1747584000 1 IN IP4 127.0.0.1
s=RTSP Session
m=video 0 RTP/AVP 96
a=rtpmap:96 H265/90000
a=fmtp:96 sprop-sps=QgEBAWAAAAMAsAAAAwAAAwB4oAPAgBDllmZpJMreEAAAAEAg;sprop-pps=RAEBAw==
"#;
        let info = StreamProber::parse_sdp(sdp).expect("parse h265 sdp with sprop-sps");
        assert_eq!(info.codec, "h265");
        assert_eq!(info.width, 1920);
        assert_eq!(info.height, 1080);
        assert!(info.is_dimension_known());
    }

    #[test]
    fn test_parse_sdp_rejects_audio_only() {
        let sdp = "v=0\r\nm=audio 0 RTP/AVP 0\r\n";
        assert!(!sdp.contains("m=video"));
    }
}
