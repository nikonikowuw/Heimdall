use base64::Engine;
use std::time::Duration;

use crate::error::MediaError;
use crate::retina_ingest::sanitize_rtsp_url_and_credentials;
use crate::rtsp::mask_rtsp_url;
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
        let (clean_url, creds) = sanitize_rtsp_url_and_credentials(rtsp_url)?;
        let session_options = retina::client::SessionOptions::default()
            .user_agent("Heimdall/1.0".to_string())
            .creds(creds);

        // 统一复用 Retina 进行 DESCRIBE 探活与 SDP 语法分析
        let session = retina::client::Session::describe(clean_url, session_options)
            .await
            .map_err(|e| MediaError::RtspConnect {
                url: masked_url.clone(),
                reason: format!("Retina 探活握手失败: {e}"),
            })?;

        // 严格检索视频轨 (video track)
        for stream in session.streams() {
            if stream.media() == "video" {
                let encoding = stream.encoding_name().to_lowercase();
                let codec = if encoding.contains("h265") || encoding.contains("hevc") {
                    "h265".to_string()
                } else {
                    "h264".to_string()
                };

                let mut fps = stream.framerate().map(|f| f as f64).unwrap_or(0.0);
                let mut width = 0u32;
                let mut height = 0u32;

                // 1. 优先从 Retina 解析出的 VideoParameters 中读取分辨率
                if let Some(retina::codec::ParametersRef::Video(v)) = stream.parameters() {
                    let (w, h) = v.pixel_dimensions();
                    width = w;
                    height = h;
                    if let Some((num, den)) = v.frame_rate() {
                        if den > 0 {
                            fps = num as f64 / den as f64;
                        }
                    }
                }

                // 2. 若 Retina 未能从 SDP 头中直接解析出尺寸，降级调用 parse_sdp 提取 sprop 参数集
                if width == 0 || height == 0 {
                    let sdp_str = String::from_utf8_lossy(session.sdp());
                    if let Some(info) = Self::parse_sdp(&sdp_str) {
                        width = info.width;
                        height = info.height;
                        if fps <= 0.0 {
                            fps = info.fps;
                        }
                    }
                }

                tracing::info!(
                    url = %masked_url,
                    codec = %codec,
                    width,
                    height,
                    fps,
                    "Retina 统一探活完成"
                );

                return Ok(StreamInfo {
                    codec,
                    width,
                    height,
                    fps,
                });
            }
        }

        Err(MediaError::RtspConnect {
            url: masked_url,
            reason: "SDP 描述中缺少视频轨 (m=video)".into(),
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

    /// 从 SDP 描述文本中提取带外参数集 (Extradata)，拼接为标准 Annex-B 格式 (`00 00 00 01 NALU ...`)。
    /// 包含 H.264 的 SPS+PPS 或 H.265 的 VPS+SPS+PPS。
    /// 当摄像头不在码流中重复发送带内参数集时，可用于合成初始关键帧注入硬件解码器以唤醒解码流水线。
    pub fn extract_sdp_extradata(sdp: &str) -> Option<Vec<u8>> {
        let mut extradata = Vec::new();
        let mut is_h265 = false;

        for line in sdp.lines() {
            let trimmed = line.trim();
            if trimmed.contains("H265") || trimmed.contains("HEVC") {
                is_h265 = true;
            }

            // H.264: sprop-parameter-sets=base64(SPS),base64(PPS)
            if let Some(idx) = trimmed.find("sprop-parameter-sets=") {
                let params = &trimmed[idx + "sprop-parameter-sets=".len()..];
                let param_list = params.split(';').next().unwrap_or("");
                for nalu_b64 in param_list.split(',') {
                    let clean_b64 = nalu_b64.trim();
                    if !clean_b64.is_empty() {
                        if let Ok(nalu_bytes) =
                            base64::engine::general_purpose::STANDARD.decode(clean_b64)
                        {
                            extradata.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
                            extradata.extend_from_slice(&nalu_bytes);
                        }
                    }
                }
            }

            // H.265: sprop-vps, sprop-sps, sprop-pps
            if is_h265 {
                for key in &["sprop-vps=", "sprop-sps=", "sprop-pps="] {
                    if let Some(idx) = trimmed.find(key) {
                        let params = &trimmed[idx + key.len()..];
                        let first = params
                            .split(';')
                            .next()
                            .unwrap_or("")
                            .split(',')
                            .next()
                            .unwrap_or("");
                        let clean_b64 = first.trim();
                        if !clean_b64.is_empty() {
                            if let Ok(nalu_bytes) =
                                base64::engine::general_purpose::STANDARD.decode(clean_b64)
                            {
                                extradata.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
                                extradata.extend_from_slice(&nalu_bytes);
                            }
                        }
                    }
                }
            }
        }

        if extradata.is_empty() {
            None
        } else {
            Some(extradata)
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
    fn test_extract_sdp_extradata_h264() {
        let sdp = r#"
v=0
m=video 0 RTP/AVP 96
a=rtpmap:96 H264/90000
a=fmtp:96 packetization-mode=1;sprop-parameter-sets=Z2QAKacyhEB4AiflwEQAAAMABAAAAPDY8MYxWA==,aM48gA==
"#;
        let extradata =
            StreamProber::extract_sdp_extradata(sdp).expect("should extract h264 extradata");
        // 应该包含两个 NALU，均以 00 00 00 01 开头
        assert!(extradata.len() > 8);
        assert_eq!(&extradata[..4], &[0x00, 0x00, 0x00, 0x01]);
        // SPS NAL unit type = 7
        assert_eq!(extradata[4] & 0x1F, 7);
        // 搜索第二个起始码
        let second_start = extradata[4..]
            .windows(4)
            .position(|w| w == [0, 0, 0, 1])
            .map(|p| p + 4);
        assert!(second_start.is_some());
        let pps_idx = second_start.expect("second start code must be present") + 4;
        // PPS NAL unit type = 8
        assert_eq!(extradata[pps_idx] & 0x1F, 8);
    }

    #[test]
    fn test_extract_sdp_extradata_h265() {
        let sdp = r#"
v=0
m=video 0 RTP/AVP 96
a=rtpmap:96 H265/90000
a=fmtp:96 sprop-sps=QgEBAWAAAAMAsAAAAwAAAwB4oAPAgBDllmZpJMreEAAAAEAg;sprop-pps=RAEBAw==
"#;
        let extradata =
            StreamProber::extract_sdp_extradata(sdp).expect("should extract h265 extradata");
        assert!(extradata.len() > 8);
        assert_eq!(&extradata[..4], &[0x00, 0x00, 0x00, 0x01]);
        // H265 SPS NAL unit type = 33
        assert_eq!((extradata[4] >> 1) & 0x3F, 33);
        let second_start = extradata[4..]
            .windows(4)
            .position(|w| w == [0, 0, 0, 1])
            .map(|p| p + 4);
        assert!(second_start.is_some());
        let pps_idx = second_start.expect("second start code must be present") + 4;
        // H265 PPS NAL unit type = 34
        assert_eq!((extradata[pps_idx] >> 1) & 0x3F, 34);
    }

    #[test]
    fn test_parse_sdp_rejects_audio_only() {
        let sdp = "v=0\r\nm=audio 0 RTP/AVP 0\r\n";
        assert!(!sdp.contains("m=video"));
    }
}
