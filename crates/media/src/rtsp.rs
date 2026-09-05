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
    pub qop: Option<String>,
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
                    let mut qop = None;

                    for part in params.split(',') {
                        if let Some((k, v)) = part.split_once('=') {
                            let key = k.trim().to_lowercase();
                            let val = v.trim().trim_matches('"').to_string();
                            match key.as_str() {
                                "realm" => realm = val,
                                "nonce" => nonce = val,
                                "opaque" => opaque = Some(val),
                                "qop" => qop = Some(val),
                                _ => {}
                            }
                        }
                    }

                    if !realm.is_empty() && !nonce.is_empty() {
                        return Some(Self {
                            realm,
                            nonce,
                            opaque,
                            qop,
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

        // 若相机支持 RFC 2617 qop="auth"（主流海康、大华等现代 IPC 默认开启）
        if let Some(ref qop_val) = self.qop {
            if qop_val.split(',').any(|s| s.trim() == "auth") {
                let nc = "00000001";
                let cnonce = format!(
                    "{:08x}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_nanos() as u64)
                        .unwrap_or(0x1a2b3c4d)
                );

                // response = MD5(HA1:nonce:nc:cnonce:qop:HA2)
                let mut hasher3 = Md5::new();
                hasher3.update(format!("{ha1}:{}:{nc}:{cnonce}:auth:{ha2}", self.nonce).as_bytes());
                let response = format!("{:x}", hasher3.finalize());

                let mut header = format!(
                    "Authorization: Digest username=\"{user}\", realm=\"{}\", nonce=\"{}\", uri=\"{uri}\", response=\"{response}\", qop=auth, nc={nc}, cnonce=\"{cnonce}\"",
                    self.realm, self.nonce
                );
                if let Some(ref op) = self.opaque {
                    header.push_str(&format!(", opaque=\"{op}\""));
                }
                header.push_str("\r\n");
                return header;
            }
        }

        // RFC 2068 无 qop 旧版兼容回退: response = MD5(HA1:nonce:HA2)
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

/// 分解后的 RTSP URL 组件（支持强密码保留字符如 @, :, #, ? 等）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedRtspUrl {
    pub scheme: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub host: String,
    pub port: Option<u16>,
    pub path_and_query: String,
}

impl ParsedRtspUrl {
    fn format_userinfo(&self, mask_password: bool) -> String {
        match (&self.username, &self.password) {
            (Some(u), Some(p)) => {
                let pass = if mask_password { "***" } else { p.as_str() };
                format!("{u}:{pass}@")
            }
            (Some(u), None) => format!("{u}@"),
            (None, Some(p)) => {
                let pass = if mask_password { "***" } else { p.as_str() };
                format!(":{pass}@")
            }
            (None, None) => String::new(),
        }
    }

    /// 构造脱去账密后的纯净 url::Url，供 Retina 或底层标准客户端建立连接与握手
    pub fn to_clean_url(&self) -> Result<url::Url, MediaError> {
        let port_part = self.port.map(|p| format!(":{p}")).unwrap_or_default();
        let path = if self.path_and_query.is_empty() {
            "/"
        } else {
            &self.path_and_query
        };
        let url_str = format!("{}://{}{}{}", self.scheme, self.host, port_part, path);
        url::Url::parse(&url_str).map_err(|e| MediaError::RtspConnect {
            url: self.to_masked_string(),
            reason: format!("纯净 URL 构造失败 ({url_str}): {e}"),
        })
    }

    /// 构造脱敏后的字符串表示（密码隐藏为 ***）
    pub fn to_masked_string(&self) -> String {
        let port_part = self.port.map(|p| format!(":{p}")).unwrap_or_default();
        let userinfo_part = self.format_userinfo(true);
        format!(
            "{}://{}{}{}{}",
            self.scheme, userinfo_part, self.host, port_part, self.path_and_query
        )
    }

    /// 构造规范化复用键（用于 StreamHub 连接复用与去重）
    pub fn to_canonical_key(&self) -> String {
        let port = self.port.unwrap_or(554);
        let userinfo_part = self.format_userinfo(false);
        let path = self.path_and_query.trim_end_matches('/');
        let path = if path.is_empty() { "/" } else { path };
        format!("rtsp://{}{}:{}{}", userinfo_part, self.host, port, path)
    }
}

/// 工业级 RTSP 脏 URL 解析清洗器
///
/// 彻底攻克安防监控现场密码包含保留字符（如 `@`, `:`, `#`, `?`, `!` 等）导致常规 Url::parse 崩溃或截断的顽疾。
/// 基于“主机名/IP 绝对不含 `@`”的不变性数学约束，利用逆向锚点定位切分 userinfo 与 host。
pub fn parse_and_clean_rtsp_url(raw_url: &str) -> Result<ParsedRtspUrl, MediaError> {
    let trimmed = raw_url.trim();
    if trimmed.is_empty() {
        return Err(MediaError::RtspConnect {
            url: String::new(),
            reason: "URL 不能为空".into(),
        });
    }

    // 1. 提取 scheme (支持 rtsp:// 或 rtsps://，大小写不敏感)
    let scheme_end = trimmed.find("://").ok_or_else(|| MediaError::RtspConnect {
        url: trimmed.to_string(),
        reason: "缺少协议头 (rtsp:// 或 rtsps://)".into(),
    })?;

    let scheme = trimmed[..scheme_end].to_ascii_lowercase();
    if scheme != "rtsp" && scheme != "rtsps" {
        return Err(MediaError::RtspConnect {
            url: trimmed.to_string(),
            reason: format!("不支持的协议类型: {scheme}"),
        });
    }

    let rest = &trimmed[scheme_end + 3..];

    // 2. 切分 authority (userinfo + host:port) 与 path_and_query
    // authority 必定在首个 '/' 之前结束；若无 '/'，则在首个 '?' 或 '#' 之前结束
    let (authority_part, path_and_query) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => match rest.find(['?', '#']) {
            Some(idx) => (&rest[..idx], &rest[idx..]),
            None => (rest, ""),
        },
    };

    if authority_part.is_empty() {
        return Err(MediaError::RtspConnect {
            url: trimmed.to_string(),
            reason: "URL 缺少主机地址".into(),
        });
    }

    // 3. 寻找 authority 中最后一个 '@'
    // 不变性约束：Host (IPv4、IPv6 或域名) 绝不会包含 '@'，因此 authority 中最后一个 '@' 必定是 userinfo 与 host 的分界点
    let (userinfo_opt, host_port_part) = match authority_part.rfind('@') {
        Some(at_idx) => {
            let userinfo = &authority_part[..at_idx];
            let host_port = &authority_part[at_idx + 1..];
            (Some(userinfo), host_port)
        }
        None => (None, authority_part),
    };

    if host_port_part.is_empty() {
        return Err(MediaError::RtspConnect {
            url: trimmed.to_string(),
            reason: "URL 缺少主机地址 (未在 '@' 后找到有效主机)".into(),
        });
    }

    // 4. 解析 host 与 port (兼容 IPv6 [fe80::1]:554)
    let (host, port) = if host_port_part.starts_with('[') {
        if let Some(close_bracket) = host_port_part.find(']') {
            let host = host_port_part[..=close_bracket].to_string();
            let after_bracket = &host_port_part[close_bracket + 1..];
            let port = if let Some(stripped) = after_bracket.strip_prefix(':') {
                Some(
                    stripped
                        .parse::<u16>()
                        .map_err(|e| MediaError::RtspConnect {
                            url: trimmed.to_string(),
                            reason: format!("IPv6 端口解析失败 ({stripped}): {e}"),
                        })?,
                )
            } else {
                None
            };
            (host, port)
        } else {
            return Err(MediaError::RtspConnect {
                url: trimmed.to_string(),
                reason: "IPv6 地址格式缺失闭合括号 ']'".into(),
            });
        }
    } else if let Some(colon_idx) = host_port_part.rfind(':') {
        let host = &host_port_part[..colon_idx];
        let port_str = &host_port_part[colon_idx + 1..];
        let port = port_str
            .parse::<u16>()
            .map_err(|e| MediaError::RtspConnect {
                url: trimmed.to_string(),
                reason: format!("端口解析失败 ({port_str}): {e}"),
            })?;
        (host.to_string(), Some(port))
    } else {
        (host_port_part.to_string(), None)
    };

    // 5. 解析 userinfo: 第一个 ':' 划分 username 和 password
    let (username, password) = match userinfo_opt {
        Some(userinfo) => match userinfo.find(':') {
            Some(colon_idx) => {
                let user = &userinfo[..colon_idx];
                let pass = &userinfo[colon_idx + 1..];
                (
                    (!user.is_empty()).then(|| user.to_string()),
                    Some(pass.to_string()),
                )
            }
            None => ((!userinfo.is_empty()).then(|| userinfo.to_string()), None),
        },
        None => (None, None),
    };

    Ok(ParsedRtspUrl {
        scheme,
        username,
        password,
        host,
        port,
        path_and_query: path_and_query.to_string(),
    })
}

/// 对 RTSP URL 中的敏感信息（用户名和密码）进行脱敏隐藏
/// 例如：rtsp://admin:123456@192.168.1.100:554/live -> rtsp://admin:***@192.168.1.100:554/live
pub fn mask_rtsp_url(raw_url: &str) -> String {
    match parse_and_clean_rtsp_url(raw_url) {
        Ok(parsed) => parsed.to_masked_string(),
        Err(_) => {
            // 容错兜底脱敏
            if let Some(at_idx) = raw_url.rfind('@') {
                let search_start = raw_url.find("://").map(|p| p + 3).unwrap_or(0);
                if let Some(colon_offset) = raw_url[search_start..at_idx].find(':') {
                    let colon_idx = search_start + colon_offset;
                    return format!("{}***{}", &raw_url[..colon_idx + 1], &raw_url[at_idx..]);
                }
            }
            raw_url.to_string()
        }
    }
}

/// 校验 RTSP 响应状态行是否为 2xx 成功状态
pub fn check_rtsp_status(resp: &str, method: &str) -> Result<(), MediaError> {
    let first_line = resp.lines().next().unwrap_or("").trim();
    if !first_line.starts_with("RTSP/1.0 2") && !resp.contains("200 OK") {
        return Err(MediaError::Protocol(format!(
            "{method} 握手失败: {}",
            if first_line.is_empty() {
                "空响应"
            } else {
                first_line
            }
        )));
    }
    Ok(())
}

pub const MAX_RTSP_HEADER_SIZE: usize = 64 * 1024; // 64 KB 响应头上限
pub const MAX_RTSP_BODY_SIZE: usize = 1024 * 1024; // 1 MB SDP / Body 上限
pub const RTSP_STEP_TIMEOUT: Duration = Duration::from_secs(5); // RTSP 单步握手 5 秒超时保护

/// 带超时保护的 RTSP 响应读取
pub async fn read_rtsp_response_timeout<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut R,
    read_buf: &mut BytesMut,
    timeout: Duration,
    masked_url: &str,
) -> Result<String, MediaError> {
    tokio::time::timeout(timeout, read_rtsp_response(reader, read_buf))
        .await
        .map_err(|_| MediaError::RtspConnect {
            url: masked_url.to_string(),
            reason: format!("读取 RTSP 响应超时 (超过 {timeout:?})"),
        })?
}

/// 流式完整读取 RTSP 响应（包含多包分片重组与 Content-Length Body 完整读取）
pub async fn read_rtsp_response<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut R,
    read_buf: &mut BytesMut,
) -> Result<String, MediaError> {
    let mut scratch = [0u8; 4096];
    let header_end_pos;

    // 1. 循环读取直至命中 \r\n\r\n 头部边界（带 64KB 防 OOM 长度保护）
    loop {
        if let Some(pos) = read_buf.windows(4).position(|w| w == b"\r\n\r\n") {
            header_end_pos = pos + 4;
            break;
        }
        if read_buf.len() > MAX_RTSP_HEADER_SIZE {
            return Err(MediaError::Protocol(format!(
                "RTSP 响应头部尺寸超出上限保护 ({} 字节)",
                MAX_RTSP_HEADER_SIZE
            )));
        }
        let n = reader.read(&mut scratch).await?;
        if n == 0 {
            return Err(MediaError::RtspConnect {
                url: String::new(),
                reason: "连接在读取 RTSP 头部时对端已关闭 (EOF)".into(),
            });
        }
        read_buf.extend_from_slice(&scratch[..n]);
    }

    let header_bytes = &read_buf[..header_end_pos];
    let header_str = String::from_utf8_lossy(header_bytes).to_string();

    // 2. 检查是否有 Content-Length
    let mut content_length: usize = 0;
    for line in header_str.lines() {
        let trimmed = line.trim();
        if trimmed.to_lowercase().starts_with("content-length:") {
            if let Ok(len) = trimmed["content-length:".len()..].trim().parse::<usize>() {
                if len > MAX_RTSP_BODY_SIZE {
                    return Err(MediaError::Protocol(format!(
                        "RTSP Content-Length ({len} 字节) 超出最大上限保护 ({} 字节)",
                        MAX_RTSP_BODY_SIZE
                    )));
                }
                content_length = len;
            }
            break;
        }
    }

    // 3. 如果有 Content-Length，必须继续读取直至 body 完全收到
    let total_required = header_end_pos + content_length;
    while read_buf.len() < total_required {
        let n = reader.read(&mut scratch).await?;
        if n == 0 {
            return Err(MediaError::RtspConnect {
                url: String::new(),
                reason: "连接在读取 RTSP Body 时对端已关闭 (EOF)".into(),
            });
        }
        read_buf.extend_from_slice(&scratch[..n]);
    }

    let response_bytes = read_buf.split_to(total_required);
    Ok(String::from_utf8_lossy(&response_bytes).to_string())
}

/// RTP 32 位时间戳单调展开器（处理 13.25 小时 32 位回绕，防止时间戳跳水与 MSE 卡死）
#[derive(Debug, Default)]
pub struct TimestampUnwrapper {
    last_unwrapped: Option<i64>,
    last_raw: u32,
}

impl TimestampUnwrapper {
    pub fn new() -> Self {
        Self::default()
    }

    /// 将 32 位可能回绕的 RTP 时间戳展开为 64 位连续单调增长的时钟
    pub fn unwrap(&mut self, raw: u32) -> i64 {
        match self.last_unwrapped {
            None => {
                self.last_raw = raw;
                self.last_unwrapped = Some(raw as i64);
                raw as i64
            }
            Some(last) => {
                // 计算差值（以有符号 32 位处理，差值在 ±2^31 范围时自动处理正向溢出与少许乱序回退）
                let diff = raw.wrapping_sub(self.last_raw) as i32;
                let new_unwrapped = last + (diff as i64);
                self.last_raw = raw;
                self.last_unwrapped = Some(new_unwrapped);
                new_unwrapped
            }
        }
    }
}

/// H.264 RTP 解包重组状态 (RFC 6184)
#[derive(Debug, Default)]
struct H264Depacketizer {
    fu_buffer: BytesMut,
    fu_start_pts: i64,
    fu_is_keyframe: bool,
    last_seq: Option<u16>,
}

impl H264Depacketizer {
    const MAX_NALU_SIZE: usize = 4 * 1024 * 1024; // 4MB 硬上限保护，杜绝畸形包 OOM

    fn process_rtp(
        &mut self,
        payload: &[u8],
        pts_ms: i64,
        seq_num: u16,
    ) -> Option<Vec<EncodedPacket>> {
        // 检查 RTP 序列号连续性（检测网络丢包与乱序）
        if let Some(prev_seq) = self.last_seq {
            let expected_seq = prev_seq.wrapping_add(1);
            if seq_num != expected_seq && !self.fu_buffer.is_empty() {
                tracing::warn!(
                    prev_seq,
                    seq_num,
                    "H.264 RTP 序列号不连续，丢弃未完成分片包"
                );
                self.fu_buffer.clear();
            }
        }
        self.last_seq = Some(seq_num);

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
                let is_keyframe = nal_type == 5;
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
                        let is_keyframe = sub_type == 5;
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
                    // 若收到新起始包且前包未结束，丢弃上一残片
                    self.fu_buffer.clear();
                    self.fu_buffer.reserve(payload.len() + 4096);
                    self.fu_buffer
                        .extend_from_slice(&[reconstructed_nal_header]);
                    self.fu_buffer.extend_from_slice(&payload[2..]);
                    self.fu_start_pts = pts_ms;
                    self.fu_is_keyframe = original_nal_type == 5;
                } else if !self.fu_buffer.is_empty() {
                    if self.fu_buffer.len() + payload.len() > Self::MAX_NALU_SIZE {
                        tracing::warn!("H.264 NALU 尺寸超限 (4MB)，强制清空");
                        self.fu_buffer.clear();
                        return None;
                    }
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

/// H.265 (HEVC / RFC 7798) RTP 解包重组状态
#[derive(Debug, Default)]
struct H265Depacketizer {
    fu_buffer: BytesMut,
    fu_start_pts: i64,
    fu_is_keyframe: bool,
    last_seq: Option<u16>,
}

impl H265Depacketizer {
    const MAX_NALU_SIZE: usize = 4 * 1024 * 1024;

    fn process_rtp(
        &mut self,
        payload: &[u8],
        pts_ms: i64,
        seq_num: u16,
    ) -> Option<Vec<EncodedPacket>> {
        if let Some(prev_seq) = self.last_seq {
            let expected_seq = prev_seq.wrapping_add(1);
            if seq_num != expected_seq && !self.fu_buffer.is_empty() {
                tracing::warn!(
                    prev_seq,
                    seq_num,
                    "H.265 RTP 序列号不连续，丢弃未完成分片包"
                );
                self.fu_buffer.clear();
            }
        }
        self.last_seq = Some(seq_num);

        if payload.len() < 2 {
            return None;
        }

        // H.265 Payload Header (2 字节):
        // F (1 bit), Type (6 bits), LayerId (6 bits), TID (3 bits)
        let nal_type = (payload[0] >> 1) & 0x3F;

        let mut packets = Vec::new();

        match nal_type {
            // 单一 NALU (Single NAL Unit Packet, Type 0..=47)
            0..=47 => {
                // 16..=21: IRAP 帧 (IDR_W_RADL, IDR_N_LP, CRA, BLA 等关键帧)
                let is_keyframe = (16..=21).contains(&nal_type);
                packets.push(EncodedPacket {
                    pts_ms,
                    is_keyframe,
                    codec: CodecType::H265,
                    payload: Bytes::copy_from_slice(payload),
                });
            }
            // AP 组合包 (Aggregation Packet, Type 48)
            48 => {
                let mut data = &payload[2..]; // 跳过 2 字节 AP 头部
                while data.len() >= 2 {
                    let nalu_len = ((data[0] as usize) << 8) | (data[1] as usize);
                    data = &data[2..];
                    if data.len() < nalu_len {
                        break;
                    }
                    let nalu = &data[..nalu_len];
                    data = &data[nalu_len..];

                    if nalu.len() >= 2 {
                        let sub_type = (nalu[0] >> 1) & 0x3F;
                        let is_keyframe = (16..=21).contains(&sub_type);
                        packets.push(EncodedPacket {
                            pts_ms,
                            is_keyframe,
                            codec: CodecType::H265,
                            payload: Bytes::copy_from_slice(nalu),
                        });
                    }
                }
            }
            // FU 分片包 (Fragmentation Unit, Type 49)
            49 => {
                if payload.len() < 3 {
                    return None;
                }
                let fu_header = payload[2];
                let start_bit = (fu_header & 0x80) != 0;
                let end_bit = (fu_header & 0x40) != 0;
                let original_nal_type = fu_header & 0x3F;

                if start_bit {
                    // 重组原始 2 字节 NAL Header:
                    // payload[0] 的 F(bit 7) 与 LayerId 高位(bit 0), 嵌入 original_nal_type
                    let reconstructed_byte0 =
                        (payload[0] & 0x81) | ((original_nal_type & 0x3F) << 1);
                    let reconstructed_byte1 = payload[1];

                    self.fu_buffer.clear();
                    self.fu_buffer.reserve(payload.len() + 8192);
                    self.fu_buffer
                        .extend_from_slice(&[reconstructed_byte0, reconstructed_byte1]);
                    self.fu_buffer.extend_from_slice(&payload[3..]);
                    self.fu_start_pts = pts_ms;
                    self.fu_is_keyframe = (16..=21).contains(&original_nal_type);
                } else if !self.fu_buffer.is_empty() {
                    if self.fu_buffer.len() + payload.len() > Self::MAX_NALU_SIZE {
                        tracing::warn!("H.265 NALU 尺寸超限 (4MB)，强制清空");
                        self.fu_buffer.clear();
                        return None;
                    }
                    self.fu_buffer.extend_from_slice(&payload[3..]);
                    if end_bit {
                        let full_nalu = self.fu_buffer.split().freeze();
                        packets.push(EncodedPacket {
                            pts_ms: self.fu_start_pts,
                            is_keyframe: self.fu_is_keyframe,
                            codec: CodecType::H265,
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

/// 统一 H.264 / H.265 解包调度器
#[derive(Debug)]
enum StreamDepacketizer {
    H264(H264Depacketizer),
    H265(H265Depacketizer),
}

impl StreamDepacketizer {
    fn process_rtp(
        &mut self,
        payload: &[u8],
        pts_ms: i64,
        seq_num: u16,
    ) -> Option<Vec<EncodedPacket>> {
        match self {
            Self::H264(d) => d.process_rtp(payload, pts_ms, seq_num),
            Self::H265(d) => d.process_rtp(payload, pts_ms, seq_num),
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
    pub async fn run_loop(
        self: Arc<Self>,
        cancel_signal: Arc<AtomicBool>,
        mut cancel_rx: tokio::sync::watch::Receiver<bool>,
    ) {
        self.is_running.store(true, Ordering::SeqCst);
        let mut backoff = Duration::from_secs(1);
        let max_backoff = Duration::from_secs(30);

        tracing::info!(camera_id = %self.camera_id, url = %mask_rtsp_url(&self.rtsp_url), "RTSP Ingestor 启动");

        while !cancel_signal.load(Ordering::Relaxed) && !*cancel_rx.borrow() {
            match self
                .stream_session(cancel_signal.clone(), cancel_rx.clone())
                .await
            {
                Ok(()) => {
                    tracing::info!(camera_id = %self.camera_id, "RTSP 拉流正常关闭或挂起");
                    break;
                }
                Err(err) => {
                    if cancel_signal.load(Ordering::Relaxed) || *cancel_rx.borrow() {
                        break;
                    }
                    tracing::warn!(
                        camera_id = %self.camera_id,
                        error = ?err,
                        retry_after_secs = backoff.as_secs(),
                        "RTSP 拉流异常中断，准备指数退避重连"
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
        tracing::info!(camera_id = %self.camera_id, "RTSP Ingestor 已停止");
    }

    async fn stream_session(
        &self,
        cancel_signal: Arc<AtomicBool>,
        mut cancel_rx: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), MediaError> {
        let masked_url = mask_rtsp_url(&self.rtsp_url);
        let parsed_url = url::Url::parse(&self.rtsp_url).map_err(|e| MediaError::RtspConnect {
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

        let mut stream = tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(&addr))
            .await
            .map_err(|_| MediaError::RtspConnect {
                url: masked_url.clone(),
                reason: format!("TCP 连接超时 ({addr}, 5秒)"),
            })?
            .map_err(|e| MediaError::RtspConnect {
                url: masked_url.clone(),
                reason: format!("TCP 连接失败 ({addr}): {e}"),
            })?;
        let _ = stream.set_nodelay(true);

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

        // 1. OPTIONS 握手 (带超时保护与完整流式响应解析)
        let mut cseq = 1;
        let mut auth_hdr = get_auth_header(&digest_auth, "OPTIONS", &self.rtsp_url);
        let req = format!(
            "OPTIONS {} RTSP/1.0\r\nCSeq: {}\r\n{}User-Agent: Heimdall/1.0\r\n\r\n",
            self.rtsp_url, cseq, auth_hdr
        );
        stream.write_all(req.as_bytes()).await?;
        let resp =
            read_rtsp_response_timeout(&mut stream, &mut read_buf, RTSP_STEP_TIMEOUT, &masked_url)
                .await?;
        tracing::debug!(camera_id = %self.camera_id, "OPTIONS 响应成功");

        let mut supports_get_parameter = resp
            .lines()
            .find(|line| line.to_lowercase().starts_with("public:"))
            .map(|line| line.to_uppercase().contains("GET_PARAMETER"))
            .unwrap_or(true);

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
                let resp = read_rtsp_response_timeout(
                    &mut stream,
                    &mut read_buf,
                    RTSP_STEP_TIMEOUT,
                    &masked_url,
                )
                .await?;
                check_rtsp_status(&resp, "OPTIONS (Digest 鉴权后)")?;
                supports_get_parameter = resp
                    .lines()
                    .find(|line| line.to_lowercase().starts_with("public:"))
                    .map(|line| line.to_uppercase().contains("GET_PARAMETER"))
                    .unwrap_or(supports_get_parameter);
            } else {
                return Err(MediaError::RtspConnect {
                    url: masked_url.clone(),
                    reason: "RTSP 鉴权失败 (401 Unauthorized)".into(),
                });
            }
        } else {
            check_rtsp_status(&resp, "OPTIONS")?;
        }

        // 2. DESCRIBE 握手 (带超时保护)
        cseq += 1;
        auth_hdr = get_auth_header(&digest_auth, "DESCRIBE", &self.rtsp_url);
        let req = format!(
            "DESCRIBE {} RTSP/1.0\r\nCSeq: {}\r\n{}Accept: application/sdp\r\nUser-Agent: Heimdall/1.0\r\n\r\n",
            self.rtsp_url, cseq, auth_hdr
        );
        stream.write_all(req.as_bytes()).await?;
        let mut sdp_resp =
            read_rtsp_response_timeout(&mut stream, &mut read_buf, RTSP_STEP_TIMEOUT, &masked_url)
                .await?;

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
                sdp_resp = read_rtsp_response_timeout(
                    &mut stream,
                    &mut read_buf,
                    RTSP_STEP_TIMEOUT,
                    &masked_url,
                )
                .await?;
            }
        }

        check_rtsp_status(&sdp_resp, "DESCRIBE")?;

        // 提取控制 URL (trackID / streamid)
        let control_url = Self::extract_control_url(&sdp_resp, &self.rtsp_url);

        // 3. SETUP 握手 (TCP Interleaved 传输，带超时保护)
        cseq += 1;
        auth_hdr = get_auth_header(&digest_auth, "SETUP", &control_url);
        let req = format!(
            "SETUP {} RTSP/1.0\r\nCSeq: {}\r\n{}Transport: RTP/AVP/TCP;unicast;interleaved=0-1\r\nUser-Agent: Heimdall/1.0\r\n\r\n",
            control_url, cseq, auth_hdr
        );
        stream.write_all(req.as_bytes()).await?;
        let setup_resp =
            read_rtsp_response_timeout(&mut stream, &mut read_buf, RTSP_STEP_TIMEOUT, &masked_url)
                .await?;
        check_rtsp_status(&setup_resp, "SETUP")?;

        let (session_id, timeout_secs) = Self::extract_session_id_and_timeout(&setup_resp);
        let session_id =
            session_id.ok_or_else(|| MediaError::Protocol("SETUP 响应中缺少 Session ID".into()))?;
        let video_channel = Self::extract_interleaved_channel(&setup_resp);

        tracing::info!(
            camera_id = %self.camera_id,
            session_id = %session_id,
            timeout_secs = timeout_secs,
            video_channel = video_channel,
            "SETUP 握手成功，已建立会话并提取参数"
        );

        // 4. PLAY 握手 (带超时保护)
        cseq += 1;
        auth_hdr = get_auth_header(&digest_auth, "PLAY", &self.rtsp_url);
        let req = format!(
            "PLAY {} RTSP/1.0\r\nCSeq: {}\r\nSession: {}\r\n{}Range: npt=0.000-\r\nUser-Agent: Heimdall/1.0\r\n\r\n",
            self.rtsp_url, cseq, session_id, auth_hdr
        );
        stream.write_all(req.as_bytes()).await?;
        let play_resp =
            read_rtsp_response_timeout(&mut stream, &mut read_buf, RTSP_STEP_TIMEOUT, &masked_url)
                .await?;
        check_rtsp_status(&play_resp, "PLAY")?;

        tracing::info!(
            camera_id = %self.camera_id,
            "RTSP 握手完成，进入 TCP Interleaved 码流接收循环并启动心跳保活"
        );

        // 拆分读写端以并发驱动保活心跳
        let (mut read_half, write_half) = stream.into_split();
        let writer_arc = Arc::new(tokio::sync::Mutex::new(write_half));

        // 启动后台 Keep-Alive 任务（默认周期 25 秒，杜绝海康/大华等设备 60 秒超时断开）
        // 使用 RAII Guard 严格管理 Keep-Alive 生命周期，退出 stream_session 即刻 abort，杜绝孤儿任务残留
        let keepalive_interval = Duration::from_secs((timeout_secs / 2).clamp(10, 30));
        let writer_clone = writer_arc.clone();
        let keepalive_url = self.rtsp_url.clone();
        let keepalive_session = session_id.clone();
        let camera_id_log = self.camera_id.clone();
        let digest_auth_clone = digest_auth.clone();
        let username_clone = username.clone();
        let password_clone = password.clone();
        let has_auth_clone = has_auth;

        let keepalive_handle = tokio::spawn(async move {
            let mut ping_cseq = 300;
            loop {
                // 必须严格先睡眠保活周期（例如 25 秒），严禁刚完成 PLAY 握手即刻发送无意义心跳打断推流
                tokio::time::sleep(keepalive_interval).await;

                ping_cseq += 1;
                let method = if supports_get_parameter {
                    "GET_PARAMETER"
                } else {
                    "OPTIONS"
                };
                let auth_hdr = if has_auth_clone {
                    if let Some(ref d) = digest_auth_clone {
                        d.build_auth_header(
                            &username_clone,
                            &password_clone,
                            method,
                            &keepalive_url,
                        )
                    } else {
                        let user_pass = format!("{username_clone}:{password_clone}");
                        let encoded = base64::engine::general_purpose::STANDARD.encode(user_pass);
                        format!("Authorization: Basic {encoded}\r\n")
                    }
                } else {
                    String::new()
                };

                let ping_req = format!(
                    "{method} {keepalive_url} RTSP/1.0\r\nCSeq: {ping_cseq}\r\nSession: {keepalive_session}\r\n{auth_hdr}User-Agent: Heimdall/1.0\r\n\r\n"
                );
                let mut guard = writer_clone.lock().await;
                tracing::debug!(camera_id = %camera_id_log, method, "RTSP 发送保活心跳");
                if let Err(e) = guard.write_all(ping_req.as_bytes()).await {
                    tracing::debug!(camera_id = %camera_id_log, error = ?e, "RTSP 保活心跳写入失败或连接已断");
                    break;
                }
            }
        });

        struct AbortOnDrop(tokio::task::JoinHandle<()>);
        impl Drop for AbortOnDrop {
            fn drop(&mut self) {
                self.0.abort();
            }
        }
        let _keepalive_guard = AbortOnDrop(keepalive_handle);

        // 5. TCP Interleaved 循环读取: $ (1B) + Channel (1B) + Length (2B) + RTP
        let is_h265 =
            sdp_resp.to_uppercase().contains("H265") || sdp_resp.to_uppercase().contains("HEVC");
        let mut depacketizer = if is_h265 {
            tracing::info!(
                camera_id = %self.camera_id,
                "RTSP 握手完成，检测到 H.265 (HEVC) 编码，启动 RFC 7798 解包循环"
            );
            StreamDepacketizer::H265(H265Depacketizer::default())
        } else {
            tracing::info!(
                camera_id = %self.camera_id,
                "RTSP 握手完成，检测到 H.264 编码，启动 RFC 6184 解包循环"
            );
            StreamDepacketizer::H264(H264Depacketizer::default())
        };

        let mut stream_buf = read_buf; // 复用前期残余缓冲
        let mut read_scratch = [0u8; 8192];
        let base_timestamp_ms = chrono::Utc::now().timestamp_millis();
        let mut timestamp_unwrapper = TimestampUnwrapper::new();
        let mut first_unwrapped_ts: Option<i64> = None;
        let mut last_emitted_pts: i64 = 0;

        // 清空前置伪唤醒标记，确保循环内的 select 只响应新的取消变更
        let _ = cancel_rx.borrow_and_update();

        while !cancel_signal.load(Ordering::Relaxed) && !*cancel_rx.borrow() {
            let bytes_read = tokio::select! {
                biased;
                change_res = cancel_rx.changed() => {
                    if change_res.is_err() || *cancel_rx.borrow() {
                        break;
                    }
                    continue;
                }
                res = read_half.read(&mut read_scratch) => res?,
            };
            if bytes_read == 0 {
                return Err(MediaError::RtspConnect {
                    url: masked_url.clone(),
                    reason: "RTSP TCP 连接对端已关闭 (EOF)".into(),
                });
            }
            stream_buf.extend_from_slice(&read_scratch[..bytes_read]);
            tracing::trace!(
                bytes_read,
                total_buf = stream_buf.len(),
                "收到 TCP 原始字节"
            );

            // 解析可能交错的一个或多个 Interleaved 数据帧
            while stream_buf.len() >= 4 {
                if stream_buf[0] != 0x24 {
                    // 寻找下一个 '$' 起始字节（同时跳过心跳回复等非数据行）
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

                // 根据协商提取的通道动态过滤视频流
                if channel == video_channel && packet_bytes.len() >= 12 {
                    let rtp_data = &packet_bytes[..];
                    let seq_num = ((rtp_data[2] as u16) << 8) | (rtp_data[3] as u16);
                    let rtp_ts = ((rtp_data[4] as u32) << 24)
                        | ((rtp_data[5] as u32) << 16)
                        | ((rtp_data[6] as u32) << 8)
                        | (rtp_data[7] as u32);

                    // 使用 64 位单调展开器，彻底消除 13.25 小时 32 位回绕与时钟跳跃崩溃
                    let unwrapped_ts = timestamp_unwrapper.unwrap(rtp_ts);
                    let base_unwrapped = *first_unwrapped_ts.get_or_insert(unwrapped_ts);
                    let delta_ticks = unwrapped_ts.saturating_sub(base_unwrapped);
                    let calculated_pts = base_timestamp_ms + (delta_ticks * 1000 / 90000);
                    let pts_ms = calculated_pts.max(last_emitted_pts);
                    last_emitted_pts = pts_ms;

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
                        if let Some(encoded_packets) =
                            depacketizer.process_rtp(rtp_payload, pts_ms, seq_num)
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

    fn extract_session_id_and_timeout(setup_resp: &str) -> (Option<String>, u64) {
        for line in setup_resp.lines() {
            let trimmed = line.trim();
            if trimmed.to_lowercase().starts_with("session:") {
                let val = trimmed["session:".len()..].trim();
                let mut parts = val.split(';');
                let session_id = parts.next().unwrap_or("").trim().to_string();
                let mut timeout = 60u64;
                for part in parts {
                    if let Some((k, v)) = part.split_once('=') {
                        if k.trim().to_lowercase() == "timeout" {
                            if let Ok(secs) = v.trim().parse::<u64>() {
                                timeout = secs;
                            }
                        }
                    }
                }
                if !session_id.is_empty() {
                    return (Some(session_id), timeout);
                }
            }
        }
        (None, 60)
    }

    fn extract_interleaved_channel(setup_resp: &str) -> u8 {
        for line in setup_resp.lines() {
            let trimmed = line.trim();
            if trimmed.to_lowercase().starts_with("transport:") {
                let val = trimmed["transport:".len()..].trim();
                for part in val.split(';') {
                    if let Some((k, v)) = part.split_once('=') {
                        if k.trim().to_lowercase() == "interleaved" {
                            if let Some(ch_str) = v.split('-').next() {
                                if let Ok(ch) = ch_str.trim().parse::<u8>() {
                                    return ch;
                                }
                            }
                        }
                    }
                }
            }
        }
        0
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
    fn test_digest_auth_with_qop_auth() {
        let resp = "RTSP/1.0 401 Unauthorized\r\nCSeq: 1\r\nWWW-Authenticate: Digest realm=\"Hikvision\", nonce=\"d6f78e\", qop=\"auth\"\r\n\r\n";
        let challenge =
            DigestAuthChallenge::from_response(resp).expect("parse digest challenge with qop");
        assert_eq!(challenge.realm, "Hikvision");
        assert_eq!(challenge.nonce, "d6f78e");
        assert_eq!(challenge.qop.as_deref(), Some("auth"));

        let auth_hdr = challenge.build_auth_header(
            "admin",
            "pass123",
            "DESCRIBE",
            "rtsp://192.168.1.64:554/Streaming/Channels/101",
        );
        assert!(auth_hdr.contains("qop=auth"));
        assert!(auth_hdr.contains("nc=00000001"));
        assert!(auth_hdr.contains("cnonce="));

        // 验证基于 Digest 构建的保活请求格式正确性
        let keepalive_auth = challenge.build_auth_header(
            "admin",
            "pass123",
            "GET_PARAMETER",
            "rtsp://192.168.1.64:554/Streaming/Channels/101",
        );
        let ping_req = format!(
            "GET_PARAMETER rtsp://192.168.1.64:554/Streaming/Channels/101 RTSP/1.0\r\nCSeq: 301\r\nSession: 123456\r\n{keepalive_auth}User-Agent: Heimdall/1.0\r\n\r\n"
        );
        assert!(ping_req.starts_with("GET_PARAMETER"));
        assert!(ping_req.contains("Session: 123456"));
        assert!(ping_req.contains("Authorization: Digest"));
    }

    #[test]
    fn test_h264_single_nalu_depacketize() {
        let mut depack = H264Depacketizer::default();
        // IDR NALU: 0x65 (type 5)
        let payload = vec![0x65, 0x88, 0x84, 0x00, 0x33];
        let pkts = depack
            .process_rtp(&payload, 1000, 1)
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
            .process_rtp(&payload, 2000, 2)
            .expect("depacketize stap-a");
        assert_eq!(pkts.len(), 2);
        assert!(!pkts[0].is_keyframe); // SPS (Parameter set)
        assert_eq!(pkts[0].payload.as_ref(), &[0x67, 0x42, 0x00]);
        assert_eq!(pkts[1].payload.as_ref(), &[0x68, 0xCE]);
    }

    #[test]
    fn test_h265_single_nalu_depacketize() {
        let mut depack = H265Depacketizer::default();
        // H.265 IDR NALU: NAL type 19 (IDR_W_RADL) -> (19 << 1) = 38 (0x26)
        let payload = vec![0x26, 0x01, 0x88, 0x84, 0x00, 0x33];
        let pkts = depack
            .process_rtp(&payload, 1000, 10)
            .expect("depacketize single h265");
        assert_eq!(pkts.len(), 1);
        assert!(pkts[0].is_keyframe);
        assert_eq!(pkts[0].codec, CodecType::H265);
        assert_eq!(pkts[0].payload.as_ref(), &payload);
    }

    #[test]
    fn test_h265_fu_fragmentation_reassembly() {
        let mut depack = H265Depacketizer::default();

        // H.265 FU (type 49 = 0x62 in byte 0: (49 << 1) = 98 = 0x62)
        // FU Header: Start bit (0x80) | IDR type 19 (0x13) = 0x93
        let part1 = vec![0x62, 0x01, 0x93, 0x11, 0x22];
        let res1 = depack.process_rtp(&part1, 3000, 101);
        assert!(res1.is_none());

        // Middle FU
        let part2 = vec![0x62, 0x01, 0x13, 0x33, 0x44];
        let res2 = depack.process_rtp(&part2, 3000, 102);
        assert!(res2.is_none());

        // End FU: End bit (0x40) | 19 = 0x53
        let part3 = vec![0x62, 0x01, 0x53, 0x55, 0x66];
        let res3 = depack.process_rtp(&part3, 3000, 103).expect("fu complete");
        assert_eq!(res3.len(), 1);
        assert!(res3[0].is_keyframe);
        assert_eq!(res3[0].codec, CodecType::H265);
        // 重建头部: byte0 = (0x62 & 0x81) | (19 << 1) = 0x00 | 0x26 = 0x26, byte1 = 0x01
        assert_eq!(
            res3[0].payload.as_ref(),
            &[0x26, 0x01, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66]
        );
    }

    #[test]
    fn test_mask_rtsp_url() {
        assert_eq!(
            mask_rtsp_url("rtsp://admin:123456@192.168.1.100:554/live/ch0"),
            "rtsp://admin:***@192.168.1.100:554/live/ch0"
        );
        assert_eq!(
            mask_rtsp_url("rtsp://192.168.1.100:554/live/ch0"),
            "rtsp://192.168.1.100:554/live/ch0"
        );
        assert_eq!(
            mask_rtsp_url("rtsp://user@192.168.1.100:554/live"),
            "rtsp://user@192.168.1.100:554/live"
        );
        // 复杂保留字符（密码中含 @, :, #, ? 等）
        assert_eq!(
            mask_rtsp_url("rtsp://admin:p@ss:word#123@192.168.1.100:554/live"),
            "rtsp://admin:***@192.168.1.100:554/live"
        );
    }

    #[test]
    fn test_parse_and_clean_rtsp_url_special_chars() {
        // 1. 密码含 @ 与 :
        let parsed = parse_and_clean_rtsp_url("rtsp://admin:p@ss:word@192.168.1.10:554/live/ch0")
            .expect("should parse");
        assert_eq!(parsed.scheme, "rtsp");
        assert_eq!(parsed.username.as_deref(), Some("admin"));
        assert_eq!(parsed.password.as_deref(), Some("p@ss:word"));
        assert_eq!(parsed.host, "192.168.1.10");
        assert_eq!(parsed.port, Some(554));
        assert_eq!(parsed.path_and_query, "/live/ch0");
        assert_eq!(
            parsed.to_clean_url().expect("valid clean url").as_str(),
            "rtsp://192.168.1.10:554/live/ch0"
        );

        // 2. 密码含 # 与 ?
        let parsed =
            parse_and_clean_rtsp_url("rtsp://root:pass#123?456@camera.local:554/stream?channel=1")
                .expect("should parse");
        assert_eq!(parsed.username.as_deref(), Some("root"));
        assert_eq!(parsed.password.as_deref(), Some("pass#123?456"));
        assert_eq!(parsed.host, "camera.local");
        assert_eq!(parsed.path_and_query, "/stream?channel=1");

        // 3. IPv6 格式
        let parsed = parse_and_clean_rtsp_url("rtsp://admin:p@ss@[fe80::1]:554/live")
            .expect("should parse ipv6");
        assert_eq!(parsed.username.as_deref(), Some("admin"));
        assert_eq!(parsed.password.as_deref(), Some("p@ss"));
        assert_eq!(parsed.host, "[fe80::1]");
        assert_eq!(parsed.port, Some(554));

        // 4. Path 含 @ 与 :
        let parsed = parse_and_clean_rtsp_url("rtsp://admin:123@192.168.1.10:554/live@ch1:sub")
            .expect("should parse path with special chars");
        assert_eq!(parsed.username.as_deref(), Some("admin"));
        assert_eq!(parsed.password.as_deref(), Some("123"));
        assert_eq!(parsed.path_and_query, "/live@ch1:sub");
    }

    #[test]
    fn test_check_rtsp_status() {
        assert!(check_rtsp_status("RTSP/1.0 200 OK\r\nCSeq: 1\r\n\r\n", "OPTIONS").is_ok());
        assert!(check_rtsp_status("RTSP/1.0 200 OK", "SETUP").is_ok());
        assert!(check_rtsp_status("RTSP/1.0 404 Stream Not Found\r\n", "DESCRIBE").is_err());
        assert!(check_rtsp_status("RTSP/1.0 500 Internal Server Error\r\n", "PLAY").is_err());
    }

    #[tokio::test]
    async fn test_read_rtsp_response_header_limit() {
        // 构造超长头部 (> 64KB)
        let large_junk = vec![b'a'; 70 * 1024];
        let mut cursor = std::io::Cursor::new(large_junk);
        let mut buf = BytesMut::new();
        let res = read_rtsp_response(&mut cursor, &mut buf).await;
        assert!(res.is_err());
        let err_msg = res.expect_err("should reject oversized header").to_string();
        assert!(err_msg.contains("超出上限保护"));
    }

    #[tokio::test]
    async fn test_read_rtsp_response_body_limit() {
        // 构造超大 Content-Length (> 1MB)
        let resp = "RTSP/1.0 200 OK\r\nCSeq: 1\r\nContent-Length: 2000000\r\n\r\n";
        let mut cursor = std::io::Cursor::new(resp.as_bytes());
        let mut buf = BytesMut::new();
        let res = read_rtsp_response(&mut cursor, &mut buf).await;
        assert!(res.is_err());
        let err_msg = res
            .expect_err("should reject oversized content length")
            .to_string();
        assert!(err_msg.contains("超出最大上限保护"));
    }

    #[test]
    fn test_timestamp_unwrapper_monotonic_and_wraparound() {
        let mut unwrapper = TimestampUnwrapper::new();

        // 初始基准
        let t1 = 0xFFFF_FFE0u32; // 接近 2^32
        let u1 = unwrapper.unwrap(t1);
        assert_eq!(u1, t1 as i64);

        // 正常小步前进
        let t2 = 0xFFFF_FFF0u32;
        let u2 = unwrapper.unwrap(t2);
        assert_eq!(u2, u1 + 16);

        // 触发 32 位溢出回绕到 0x0000_0010
        // 实际跨越了 0，增量为 (0xFFFF_FFFF - 0xFFFF_FFF0) + 1 + 0x10 = 15 + 1 + 16 = 32
        let t3 = 0x0000_0010u32;
        let u3 = unwrapper.unwrap(t3);
        assert_eq!(u3, u2 + 32);
        assert!(u3 > u2); // 绝对单调递增，没有跳水

        // 再次前进
        let t4 = 0x0000_0030u32;
        let u4 = unwrapper.unwrap(t4);
        assert_eq!(u4, u3 + 32);
    }
}
