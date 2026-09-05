//! WebCodecs 二进制流帧封装协议 (Ultra-Low-Latency Binary Framing)
//!
//! 专为浏览器 WebCodecs (VideoDecoder) 与 WebSocket 设计的高效零拷贝二进制帧头：
//!
//! +-----------------------------------------------------------------------+
//! | Version (1B) | Codec (1B) | FrameType (1B) | Flags (1B) | PTS_MS (8B) |
//! +-----------------------------------------------------------------------+
//! |                                Payload (N bytes Annex B NALU)        |
//! +-----------------------------------------------------------------------+
//!
//! - [0..1]   Magic / Protocol Version (0x01)
//! - [1..2]   Codec (0x01 = H264, 0x02 = H265)
//! - [2..3]   Frame Type (0x01 = Keyframe / IDR, 0x00 = Delta / P / B)
//! - [3..4]   Flags (0x00 保留)
//! - [4..12]  PTS_MS (8 字节大端整数，毫秒时间戳)
//! - [12..]   Payload (Annex B NALU 原始数据，包含 00 00 00 01 起始码)

use bytes::{BufMut, Bytes, BytesMut};
use types::{CodecType, EncodedPacket};

/// WebCodecs 二进制帧头长度 (12 字节)
pub const WEBCODECS_FRAME_HEADER_LEN: usize = 12;

/// WebCodecs 传输协议版本号
pub const WEBCODECS_PROTOCOL_VERSION: u8 = 0x01;

/// 解析出的 WebCodecs 二进制帧头元数据
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebCodecsFrameHeader {
    pub version: u8,
    pub codec: CodecType,
    pub is_keyframe: bool,
    pub flags: u8,
    pub pts_ms: i64,
}

/// 将内部 EncodedPacket 序列化为 12 字节二进制帧头的 WebCodecs 传输包
pub fn pack_webcodecs_frame(packet: &EncodedPacket) -> Bytes {
    let mut buf = BytesMut::with_capacity(WEBCODECS_FRAME_HEADER_LEN + packet.payload.len());
    buf.put_u8(WEBCODECS_PROTOCOL_VERSION);
    buf.put_u8(match packet.codec {
        CodecType::H264 => 0x01,
        CodecType::H265 => 0x02,
    });
    buf.put_u8(if packet.is_keyframe { 0x01 } else { 0x00 });
    buf.put_u8(0x00);
    buf.put_i64(packet.pts_ms);
    buf.put_slice(&packet.payload);
    buf.freeze()
}

/// 解析 WebCodecs 二进制包，返回帧头信息与原始 Annex B 载荷
pub fn unpack_webcodecs_frame(data: &[u8]) -> Option<(WebCodecsFrameHeader, &[u8])> {
    if data.len() < WEBCODECS_FRAME_HEADER_LEN {
        return None;
    }
    let version = data[0];
    if version != WEBCODECS_PROTOCOL_VERSION {
        return None;
    }
    let codec = match data[1] {
        0x01 => CodecType::H264,
        0x02 => CodecType::H265,
        _ => return None,
    };
    let is_keyframe = data[2] == 0x01;
    let flags = data[3];
    let pts_bytes = &data[4..12];
    let pts_ms = i64::from_be_bytes(pts_bytes.try_into().ok()?);
    let payload = &data[WEBCODECS_FRAME_HEADER_LEN..];

    Some((
        WebCodecsFrameHeader {
            version,
            codec,
            is_keyframe,
            flags,
            pts_ms,
        },
        payload,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_webcodecs_pack_and_unpack_roundtrip_h264() {
        let payload =
            Bytes::from_static(b"\x00\x00\x00\x01\x67\x42\x00\x1f\x00\x00\x00\x01\x65\x88");
        let packet = EncodedPacket {
            pts_ms: 1741100050123,
            is_keyframe: true,
            codec: CodecType::H264,
            payload: payload.clone(),
        };

        let packed = pack_webcodecs_frame(&packet);
        assert_eq!(packed.len(), WEBCODECS_FRAME_HEADER_LEN + payload.len());

        let (header, unpacked_payload) =
            unpack_webcodecs_frame(&packed).expect("unpack should succeed");
        assert_eq!(header.version, WEBCODECS_PROTOCOL_VERSION);
        assert_eq!(header.codec, CodecType::H264);
        assert!(header.is_keyframe);
        assert_eq!(header.flags, 0x00);
        assert_eq!(header.pts_ms, 1741100050123);
        assert_eq!(unpacked_payload, payload.as_ref());
    }

    #[test]
    fn test_webcodecs_pack_and_unpack_roundtrip_h265() {
        let payload = Bytes::from_static(b"\x00\x00\x00\x01\x40\x01\x0c\x00\x00\x00\x01\x26\x01");
        let packet = EncodedPacket {
            pts_ms: 1741100099999,
            is_keyframe: false,
            codec: CodecType::H265,
            payload: payload.clone(),
        };

        let packed = pack_webcodecs_frame(&packet);
        let (header, unpacked_payload) =
            unpack_webcodecs_frame(&packed).expect("unpack should succeed");
        assert_eq!(header.version, WEBCODECS_PROTOCOL_VERSION);
        assert_eq!(header.codec, CodecType::H265);
        assert!(!header.is_keyframe);
        assert_eq!(header.pts_ms, 1741100099999);
        assert_eq!(unpacked_payload, payload.as_ref());
    }

    #[test]
    fn test_webcodecs_unpack_rejects_corrupted_data() {
        // 数据长度不足 12 字节
        assert!(unpack_webcodecs_frame(&[0x01, 0x01, 0x01]).is_none());

        // 错误的版本号
        let mut bad_ver = vec![0x02; 12];
        bad_ver[1] = 0x01;
        assert!(unpack_webcodecs_frame(&bad_ver).is_none());

        // 错误的编码格式
        let mut bad_codec = vec![0x01; 12];
        bad_codec[1] = 0xFF;
        assert!(unpack_webcodecs_frame(&bad_codec).is_none());
    }
}
