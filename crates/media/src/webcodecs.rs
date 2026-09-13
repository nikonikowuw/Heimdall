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
//! - [3..4]   Flags (bit 0 = discontinuity, consumer resets decoder before frame)
//! - [4..12]  PTS_MS (8 字节大端整数，毫秒时间戳)
//! - [12..]   Payload (Annex B NALU 原始数据，包含 00 00 00 01 起始码)

use bytes::{BufMut, Bytes, BytesMut};
use types::{CodecType, EncodedPacket};

/// WebCodecs 二进制帧头长度 (12 字节)
pub const WEBCODECS_FRAME_HEADER_LEN: usize = 12;

/// WebCodecs 传输协议版本号
pub const WEBCODECS_PROTOCOL_VERSION: u8 = 0x01;

/// Flags bit 0 marks that the consumer must reset its decoder before this frame.
pub const WEBCODECS_FLAG_DISCONTINUITY: u8 = 0x01;

/// 解析出的 WebCodecs 二进制帧头元数据
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebCodecsFrameHeader {
    pub version: u8,
    pub codec: CodecType,
    pub is_keyframe: bool,
    pub flags: u8,
    pub pts_ms: i64,
}

/// 构造 12 字节的固定栈上 WebCodecs 帧头元数据
///
/// 消除中间堆分配，可供向量化 I/O 或切片组装直接复用
pub fn build_webcodecs_header(
    packet: &EncodedPacket,
    flags: u8,
) -> Option<[u8; WEBCODECS_FRAME_HEADER_LEN]> {
    if !packet.codec.is_video() || packet.stream_tag == types::StreamTag::Audio {
        return None;
    }
    let codec_byte = match packet.codec {
        CodecType::H264 => 0x01,
        CodecType::H265 => 0x02,
        CodecType::Aac => return None,
    };
    let mut header = [0u8; WEBCODECS_FRAME_HEADER_LEN];
    header[0] = WEBCODECS_PROTOCOL_VERSION;
    header[1] = codec_byte;
    header[2] = if packet.is_keyframe { 0x01 } else { 0x00 };
    header[3] = flags;
    header[4..12].copy_from_slice(&packet.pts_ms.to_be_bytes());
    Some(header)
}

/// 将内部 EncodedPacket 序列化为 12 字节二进制帧头的 WebCodecs 传输包
///
/// 若传入音频包或非视频包，安全返回空 `Bytes`。
pub fn pack_webcodecs_frame(packet: &EncodedPacket) -> Bytes {
    pack_webcodecs_frame_with_flags(packet, 0)
}

/// 将视频帧序列化并携带显式控制 flags（例如 Replay/SourceReset 后的不连续点）。
pub fn pack_webcodecs_frame_with_flags(packet: &EncodedPacket, flags: u8) -> Bytes {
    let header = match build_webcodecs_header(packet, flags) {
        Some(h) => h,
        None => return Bytes::new(),
    };
    let mut buf = BytesMut::with_capacity(WEBCODECS_FRAME_HEADER_LEN + packet.payload.len());
    buf.put_slice(&header);
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
            ..Default::default()
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
            ..Default::default()
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

    #[test]
    fn test_build_webcodecs_header_layout_and_flags() {
        let packet_h264 = EncodedPacket {
            pts_ms: 0x0102_0304_0506_0708,
            is_keyframe: true,
            codec: CodecType::H264,
            payload: Bytes::from_static(&[0x65]),
            ..Default::default()
        };
        let header = build_webcodecs_header(&packet_h264, WEBCODECS_FLAG_DISCONTINUITY)
            .expect("should build h264 header");
        assert_eq!(header[0], WEBCODECS_PROTOCOL_VERSION);
        assert_eq!(header[1], 0x01); // H264
        assert_eq!(header[2], 0x01); // Keyframe
        assert_eq!(header[3], WEBCODECS_FLAG_DISCONTINUITY);
        assert_eq!(&header[4..12], &0x0102_0304_0506_0708i64.to_be_bytes());

        let packet_h265 = EncodedPacket {
            pts_ms: 12345,
            is_keyframe: false,
            codec: CodecType::H265,
            payload: Bytes::from_static(&[0x01]),
            ..Default::default()
        };
        let header265 = build_webcodecs_header(&packet_h265, 0).expect("should build h265 header");
        assert_eq!(header265[0], WEBCODECS_PROTOCOL_VERSION);
        assert_eq!(header265[1], 0x02); // H265
        assert_eq!(header265[2], 0x00); // Non-keyframe
        assert_eq!(header265[3], 0x00);
        assert_eq!(&header265[4..12], &12345i64.to_be_bytes());
    }

    #[test]
    fn test_build_webcodecs_header_rejects_audio() {
        let audio_pkt = EncodedPacket {
            pts_ms: 100,
            is_keyframe: true,
            codec: CodecType::Aac,
            payload: Bytes::from_static(&[0xFF, 0xF1]),
            stream_tag: types::StreamTag::Audio,
        };
        assert!(build_webcodecs_header(&audio_pkt, 0).is_none());

        let video_with_audio_tag = EncodedPacket {
            pts_ms: 100,
            is_keyframe: true,
            codec: CodecType::H264,
            payload: Bytes::from_static(&[0x65]),
            stream_tag: types::StreamTag::Audio,
        };
        assert!(build_webcodecs_header(&video_with_audio_tag, 0).is_none());
    }
}
