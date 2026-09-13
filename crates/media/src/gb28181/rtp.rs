//! RTP 协议解析与有界 JitterBuffer 乱序重组模块
//!
//! 遵循 RFC 3550 与 RFC 4571 规范：
//! 1. RFC 3550 RTP 头部解析 (Sequence Number、Timestamp、Marker、SSRC)；
//! 2. 支持 TCP (RFC 4571 2-byte length prefix) 与 UDP 两种传输模型；
//! 3. 有界 JitterBuffer (默认 64 包容量)，基于 16 位序号模运算 (wrapping_sub) 执行快速乱序重排与空洞检测；
//! 4. 超时或溢出时强制输出并标记 Discontinuity，防范内存膨胀。

use bytes::Bytes;
use std::collections::BTreeMap;

use crate::error::MediaError;

/// RTP 数据包结构
#[derive(Debug, Clone)]
pub struct RtpPacket {
    pub payload_type: u8,
    pub marker: bool,
    pub sequence_number: u16,
    pub timestamp: u32,
    pub ssrc: u32,
    pub payload: Bytes,
    pub discontinuity: bool,
}

impl RtpPacket {
    /// 解析标准 12 字节 RTP 数据包
    pub fn parse(raw: &[u8]) -> Result<Self, MediaError> {
        if raw.len() < 12 {
            return Err(MediaError::Protocol(format!(
                "RTP 报文过短 (长度: {} 字节 < 12 字节)",
                raw.len()
            )));
        }

        let b0 = raw[0];
        let version = (b0 >> 6) & 0x03;
        if version != 2 {
            return Err(MediaError::Protocol(format!(
                "不支持的 RTP 版本: {version} (仅支持 Version 2)"
            )));
        }

        let padding = (b0 & 0x20) != 0;
        let extension = (b0 & 0x10) != 0;
        let csrc_count = (b0 & 0x0F) as usize;

        let b1 = raw[1];
        let marker = (b1 & 0x80) != 0;
        let payload_type = b1 & 0x7F;

        let sequence_number = u16::from_be_bytes([raw[2], raw[3]]);
        let timestamp = u32::from_be_bytes([raw[4], raw[5], raw[6], raw[7]]);
        let ssrc = u32::from_be_bytes([raw[8], raw[9], raw[10], raw[11]]);

        let mut header_len = 12 + csrc_count * 4;
        if raw.len() < header_len {
            return Err(MediaError::Protocol(
                "RTP CSRC 长度超出报文边界".to_string(),
            ));
        }

        // 解析扩展头 (Extension)
        if extension {
            if raw.len() < header_len + 4 {
                return Err(MediaError::Protocol("RTP 扩展头不完整".to_string()));
            }
            let ext_len = u16::from_be_bytes([raw[header_len + 2], raw[header_len + 3]]) as usize;
            header_len += 4 + ext_len * 4;
            if raw.len() < header_len {
                return Err(MediaError::Protocol("RTP 扩展载荷超出报文边界".to_string()));
            }
        }

        // 处理尾部 Padding
        let mut end_pos = raw.len();
        if padding {
            if end_pos <= header_len {
                return Err(MediaError::Protocol(
                    "RTP Padding 标志存在但无有效载荷".to_string(),
                ));
            }
            let padding_len = raw[end_pos - 1] as usize;
            if padding_len > end_pos - header_len {
                return Err(MediaError::Protocol("RTP Padding 长度非法".to_string()));
            }
            end_pos -= padding_len;
        }

        let payload = Bytes::copy_from_slice(&raw[header_len..end_pos]);

        Ok(Self {
            payload_type,
            marker,
            sequence_number,
            timestamp,
            ssrc,
            payload,
            discontinuity: false,
        })
    }
}

/// 有界 JitterBuffer，负责 UDP 模式下的乱序重排与空洞平滑
#[derive(Debug)]
pub struct JitterBuffer {
    capacity: usize,
    buffer: BTreeMap<u16, RtpPacket>,
    expected_seq: Option<u16>,
}

impl Default for JitterBuffer {
    fn default() -> Self {
        Self::new(64)
    }
}

impl JitterBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(8),
            buffer: BTreeMap::new(),
            expected_seq: None,
        }
    }

    /// 压入一个 RTP 包，返回已按序就绪的包序列
    pub fn push(&mut self, packet: RtpPacket) -> Vec<RtpPacket> {
        let seq = packet.sequence_number;

        // 首包接入时初始化期望序列号
        let expected = match self.expected_seq {
            Some(exp) => exp,
            None => {
                self.expected_seq = Some(seq.wrapping_add(1));
                return vec![packet];
            }
        };

        // 检查该序号与期望序号的关系
        let diff = (seq.wrapping_sub(expected)) as i16;

        if diff < 0 {
            // 过期迟到包，直接丢弃
            return Vec::new();
        }

        self.buffer.insert(seq, packet);

        let mut ready = Vec::new();

        // 顺序取出连续可交付的包
        let mut curr_expected = expected;
        while let Some(pkt) = self.buffer.remove(&curr_expected) {
            curr_expected = curr_expected.wrapping_add(1);
            ready.push(pkt);
        }
        self.expected_seq = Some(curr_expected);

        // 如果缓冲区溢出（说明前方存在长期空洞），强制弹出最老的包并向前跳过空洞
        if self.buffer.len() > self.capacity {
            if let Some(&oldest_seq) = self
                .buffer
                .keys()
                .min_by_key(|&&k| k.wrapping_sub(curr_expected) as i16)
            {
                let mut next_expected = oldest_seq;
                let mut first_popped = true;
                while let Some(mut pkt) = self.buffer.remove(&next_expected) {
                    if first_popped {
                        pkt.discontinuity = true;
                        first_popped = false;
                    }
                    next_expected = next_expected.wrapping_add(1);
                    ready.push(pkt);
                }
                self.expected_seq = Some(next_expected);
            }
        }

        ready
    }

    /// 重置 JitterBuffer
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.expected_seq = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rtp_packet_parse_basic() {
        let mut raw = vec![
            0x80, // V=2, P=0, X=0, CC=0
            0x60, // M=0, PT=96
            0x00, 0x0A, // Sequence = 10
            0x00, 0x01, 0x5F, 0x90, // Timestamp = 90000
            0x12, 0x34, 0x56, 0x78, // SSRC = 0x12345678
        ];
        let payload_data = b"MPEG-PS-CHUNK";
        raw.extend_from_slice(payload_data);

        let pkt = RtpPacket::parse(&raw).expect("parse rtp");
        assert_eq!(pkt.payload_type, 96);
        assert!(!pkt.marker);
        assert_eq!(pkt.sequence_number, 10);
        assert_eq!(pkt.timestamp, 90000);
        assert_eq!(pkt.ssrc, 0x12345678);
        assert_eq!(&pkt.payload[..], payload_data);
    }

    #[test]
    fn test_jitter_buffer_reorder() {
        let mut jb = JitterBuffer::new(16);

        let make_pkt = |seq: u16| RtpPacket {
            payload_type: 96,
            marker: false,
            sequence_number: seq,
            timestamp: 1000,
            ssrc: 1,
            payload: Bytes::from(format!("seq-{seq}")),
            discontinuity: false,
        };

        // 压入 seq 10 (首包)
        let out = jb.push(make_pkt(10));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].sequence_number, 10);

        // 乱序：先到 seq 12，再到 seq 11
        let out12 = jb.push(make_pkt(12));
        assert_eq!(out12.len(), 0); // 缺 11，暂不输出

        let out11 = jb.push(make_pkt(11));
        assert_eq!(out11.len(), 2); // 11 和 12 一并按序输出
        assert_eq!(out11[0].sequence_number, 11);
        assert_eq!(out11[1].sequence_number, 12);
    }

    #[test]
    fn test_jitter_buffer_wraparound_and_discontinuity() {
        let mut jb = JitterBuffer::new(8);

        let make_pkt = |seq: u16| RtpPacket {
            payload_type: 96,
            marker: false,
            sequence_number: seq,
            timestamp: 1000,
            ssrc: 1,
            payload: Bytes::from(format!("seq-{seq}")),
            discontinuity: false,
        };

        // 序号在 65534 附近
        jb.push(make_pkt(65534));

        // 乱序收到 0 和 1 (已回环)，但缺失 65535
        jb.push(make_pkt(0));
        jb.push(make_pkt(1));

        // 填补 65535，连续输出
        let out = jb.push(make_pkt(65535));
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].sequence_number, 65535);
        assert_eq!(out[1].sequence_number, 0);
        assert_eq!(out[2].sequence_number, 1);
        assert!(!out[0].discontinuity);

        // 此时 expected_seq 为 2
        // 发送高于 2 的不连续包 (从 10 开始)，当缓冲区超过 capacity (8) 时触发强制丢弃空洞跳帧
        let mut forced = Vec::new();
        for s in 10..=20 {
            let res = jb.push(make_pkt(s));
            if !res.is_empty() {
                forced = res;
                break;
            }
        }
        assert!(!forced.is_empty());
        assert!(forced[0].discontinuity);
    }
}
