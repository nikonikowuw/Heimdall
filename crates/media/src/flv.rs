use bytes::{BufMut, Bytes, BytesMut};
use types::{CodecType, EncodedPacket};

use crate::stream_hub::KeyframeCache;

/// 剥离 NALU 可能携带的 Annex B 起始码 (0x00 00 00 01 或 0x00 00 01)，确保纯净 NALU 载荷
pub fn strip_nalu_start_code(data: &[u8]) -> &[u8] {
    if data.starts_with(&[0, 0, 0, 1]) {
        &data[4..]
    } else if data.starts_with(&[0, 0, 1]) {
        &data[3..]
    } else {
        data
    }
}

/// FLV 容器封装器（支持标准 H.264 与 Enhanced FLV H.265 / HEVC）
#[derive(Debug)]
pub struct FlvMuxer;

impl FlvMuxer {
    /// 生成标准 9 字节 FLV 文件头 + 4 字节 PreviousTagSize0
    pub fn flv_header() -> Bytes {
        let mut b = BytesMut::with_capacity(13);
        // "FLV" + version 1 + Flags (0x01 = Video only) + DataOffset (9) + PreviousTagSize0 (0)
        b.extend_from_slice(&[
            b'F', b'L', b'V', 0x01, // Signature & Version
            0x01, // TypeFlags (Video Only)
            0x00, 0x00, 0x00, 0x09, // DataOffset
            0x00, 0x00, 0x00, 0x00, // PreviousTagSize0
        ]);
        b.freeze()
    }

    /// 构建单个 FLV Tag（11 字节 Tag Header + 数据体 + 4 字节 PreviousTagSize）
    fn wrap_tag(tag_type: u8, timestamp_ms: u32, payload: &[u8]) -> Bytes {
        let data_size = payload.len() as u32;
        let mut tag = BytesMut::with_capacity(11 + payload.len() + 4);

        // Tag Type (1B)
        tag.put_u8(tag_type);
        // Data Size (3B Big-Endian)
        tag.put_u8(((data_size >> 16) & 0xFF) as u8);
        tag.put_u8(((data_size >> 8) & 0xFF) as u8);
        tag.put_u8((data_size & 0xFF) as u8);
        // Timestamp (3B Big-Endian) + TimestampExtended (1B)
        tag.put_u8(((timestamp_ms >> 16) & 0xFF) as u8);
        tag.put_u8(((timestamp_ms >> 8) & 0xFF) as u8);
        tag.put_u8((timestamp_ms & 0xFF) as u8);
        tag.put_u8(((timestamp_ms >> 24) & 0xFF) as u8);
        // StreamID (3B = 0)
        tag.put_slice(&[0x00, 0x00, 0x00]);

        // Tag Payload
        tag.put_slice(payload);

        // PreviousTagSize (4B = 11 + DataSize)
        tag.put_u32(11 + data_size);

        tag.freeze()
    }

    /// 生成 H.264 AVC Sequence Header (AVCDecoderConfigurationRecord / SPS + PPS)
    pub fn build_h264_sequence_header(sps: &[u8], pps: &[u8]) -> Option<Bytes> {
        let sps = strip_nalu_start_code(sps);
        let pps = strip_nalu_start_code(pps);

        if sps.len() < 4 || pps.is_empty() {
            return None;
        }

        let mut body = BytesMut::with_capacity(64 + sps.len() + pps.len());
        // FLV Video Header: Keyframe (0x10) | AVC (0x07) = 0x17
        body.put_u8(0x17);
        // AVCPacketType = 0 (Sequence Header)
        body.put_u8(0x00);
        // CompositionTime = 0
        body.put_slice(&[0x00, 0x00, 0x00]);

        // AVCDecoderConfigurationRecord
        body.put_u8(0x01); // configurationVersion = 1
        body.put_u8(sps[1]); // AVCProfileIndication
        body.put_u8(sps[2]); // profile_compatibility
        body.put_u8(sps[3]); // AVCLevelIndication
        body.put_u8(0xFF); // lengthSizeMinusOne = 3 (4 bytes length)
        body.put_u8(0xE1); // numOfSequenceParameterSets = 1
        body.put_u16(sps.len() as u16);
        body.put_slice(sps);
        body.put_u8(0x01); // numOfPictureParameterSets = 1
        body.put_u16(pps.len() as u16);
        body.put_slice(pps);

        Some(Self::wrap_tag(0x09, 0, &body))
    }

    /// 生成 Enhanced FLV H.265 Sequence Header (HEVCDecoderConfigurationRecord / VPS + SPS + PPS)
    pub fn build_h265_sequence_header(vps: Option<&[u8]>, sps: &[u8], pps: &[u8]) -> Option<Bytes> {
        let sps = strip_nalu_start_code(sps);
        let pps = strip_nalu_start_code(pps);
        let vps = vps.map(strip_nalu_start_code);

        if sps.len() < 4 || pps.is_empty() {
            return None;
        }

        let mut body = BytesMut::with_capacity(128 + sps.len() + pps.len());
        // Enhanced FLV Header: IsExHeader (0x80) | FrameType Keyframe (0x10) | PacketType SequenceStart (0x00) = 0x90
        body.put_u8(0x90);
        // FourCC: "hvc1"
        body.put_slice(b"hvc1");

        // HEVCDecoderConfigurationRecord
        body.put_u8(0x01); // configurationVersion = 1

        // 从 SPS 提取真实 profile_tier_level (12 字节)，若不足则使用兼容 Main Profile 垫片
        if sps.len() >= 15 {
            body.put_slice(&sps[3..15]);
        } else {
            body.put_u8(0x01); // general_profile_space(0) | tier_flag(0) | profile_idc(1)
            body.put_u32(0x60000000); // general_profile_compatibility_flags
            body.put_slice(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00]); // constraint_indicator_flags (6B)
            body.put_u8(0x78); // general_level_idc (Level 4.0 = 120 = 0x78)
        }

        body.put_u16(0xF000); // min_spatial_segmentation_idc
        body.put_u8(0xFC); // parallelismType
        body.put_u8(0xFD); // chroma_format_idc (1 = 4:2:0)
        body.put_u8(0xF8); // bit_depth_luma_minus8 (0 = 8bit)
        body.put_u8(0xF8); // bit_depth_chroma_minus8 (0 = 8bit)
        body.put_u16(0x0000); // avgFrameRate
                              // constantFrameRate(00b) | numTemporalLayers(001b = 1) | temporalIdNested(0b) | lengthSizeMinusOne(11b = 3) = 0x1B
                              // 遵循 ISO/IEC 14496-15，numTemporalLayers 至少为 1，杜绝非标准 0 值引发部分浏览器 MSE 初始化失败
        body.put_u8(0x1B);

        let num_arrays = if vps.is_some() { 3u8 } else { 2u8 };
        body.put_u8(num_arrays);

        // Array 1: VPS (NAL type 32, array_completeness = 1 => 0x80 | 32 = 0xA0)
        if let Some(v) = vps {
            body.put_u8(0xA0);
            body.put_u16(1); // numNalus
            body.put_u16(v.len() as u16);
            body.put_slice(v);
        }

        // Array 2: SPS (NAL type 33, array_completeness = 1 => 0x80 | 33 = 0xA1)
        body.put_u8(0xA1);
        body.put_u16(1); // numNalus
        body.put_u16(sps.len() as u16);
        body.put_slice(sps);

        // Array 3: PPS (NAL type 34, array_completeness = 1 => 0x80 | 34 = 0xA2)
        body.put_u8(0xA2);
        body.put_u16(1); // numNalus
        body.put_u16(pps.len() as u16);
        body.put_slice(pps);

        Some(Self::wrap_tag(0x09, 0, &body))
    }

    /// 将视频关键帧参数集缓存转换为首包 Sequence Header
    pub fn build_sequence_header_from_cache(
        codec: CodecType,
        cache: &KeyframeCache,
    ) -> Option<Bytes> {
        let sps = cache.sps.as_deref()?;
        let pps = cache.pps.as_deref()?;

        match codec {
            CodecType::H264 => Self::build_h264_sequence_header(sps, pps),
            CodecType::H265 => Self::build_h265_sequence_header(cache.vps.as_deref(), sps, pps),
        }
    }

    /// 将单个 EncodedPacket 封装为 FLV Video Tag（内置起始码自动剥离与时间戳单调递增看门狗）
    pub fn packet_to_flv_tag_with_filter(
        pkt: &EncodedPacket,
        base_pts_ms: i64,
        last_timestamp_ms: &mut u32,
    ) -> Bytes {
        let raw_rel_ts = (pkt.pts_ms.saturating_sub(base_pts_ms)).max(0) as u32;
        // 时间戳单调递增看门狗：防止网络抖动或摄像头时间戳回跳导致 MSE SourceBuffer 卡死
        let rel_ts = raw_rel_ts.max(*last_timestamp_ms);
        *last_timestamp_ms = rel_ts;

        let clean_payload = strip_nalu_start_code(&pkt.payload);

        match pkt.codec {
            CodecType::H264 => {
                let mut body = BytesMut::with_capacity(5 + 4 + clean_payload.len());
                let frame_type = if pkt.is_keyframe { 0x17 } else { 0x27 };
                body.put_u8(frame_type);
                body.put_u8(0x01); // AVCPacketType = 1 (NALU)
                body.put_slice(&[0x00, 0x00, 0x00]); // CompositionTime = 0
                body.put_u32(clean_payload.len() as u32);
                body.put_slice(clean_payload);
                Self::wrap_tag(0x09, rel_ts, &body)
            }
            CodecType::H265 => {
                let mut body = BytesMut::with_capacity(5 + 3 + 4 + clean_payload.len());
                // Enhanced FLV: IsExHeader(0x80) | FrameType(0x10 / 0x20) | PacketType(0x01 = CodedFrames)
                let header_byte = 0x80 | (if pkt.is_keyframe { 0x10 } else { 0x20 }) | 0x01;
                body.put_u8(header_byte);
                body.put_slice(b"hvc1"); // FourCC
                body.put_slice(&[0x00, 0x00, 0x00]); // CompositionTime = 0
                body.put_u32(clean_payload.len() as u32);
                body.put_slice(clean_payload);
                Self::wrap_tag(0x09, rel_ts, &body)
            }
        }
    }

    /// 将单个 EncodedPacket 封装为 FLV Video Tag
    pub fn packet_to_flv_tag(pkt: &EncodedPacket, base_pts_ms: i64) -> Bytes {
        let mut dummy_ts = 0;
        Self::packet_to_flv_tag_with_filter(pkt, base_pts_ms, &mut dummy_ts)
    }
}

/// FLV 实时流生成管道状态机（统一封装 Sequence Header、GOP 注入、参数集解析与时间戳单调滤波）
#[derive(Debug, Default)]
pub struct FlvStreamPipeline {
    pub sent_sequence_header: bool,
    pub has_first_keyframe: bool,
    pub base_pts_ms: Option<i64>,
    pub last_flv_ts: u32,
    pub last_gop_pts: i64,
    pub sps_buf: Option<Bytes>,
    pub pps_buf: Option<Bytes>,
    pub vps_buf: Option<Bytes>,
    pub frame_count: u64,
}

impl FlvStreamPipeline {
    pub fn new() -> Self {
        Self::default()
    }

    /// 从 KeyframeCache 注入首屏 Sequence Header 与完整 GOP 关键帧包
    pub fn inject_cache(&mut self, cache: &KeyframeCache, fallback_codec: CodecType) -> Vec<Bytes> {
        let mut tags = Vec::new();
        let codec = cache.codec.unwrap_or(fallback_codec);

        if let Some(seq_tag) = FlvMuxer::build_sequence_header_from_cache(codec, cache) {
            tags.push(seq_tag);
            self.sent_sequence_header = true;
        }

        if !cache.gop_packets.is_empty() {
            let first_pts = cache.gop_packets[0].pts_ms;
            self.base_pts_ms = Some(first_pts);
            for gop_pkt in &cache.gop_packets {
                let tag = FlvMuxer::packet_to_flv_tag_with_filter(
                    gop_pkt,
                    first_pts,
                    &mut self.last_flv_ts,
                );
                tags.push(tag);
                self.frame_count += 1;
            }
            self.last_gop_pts = cache.gop_packets.last().map(|p| p.pts_ms).unwrap_or(0);
            self.has_first_keyframe = true;
        }

        tags
    }

    /// 处理消费端 Lagged 事件（重置关键帧对齐标记）
    pub fn handle_lagged(&mut self) {
        self.has_first_keyframe = false;
    }

    /// 处理实时收到的单个 EncodedPacket，返回待下发的 FLV Tags（可能包含 Sequence Header + 视频帧 Tag）
    pub fn process_packet(&mut self, pkt: &EncodedPacket) -> Vec<Bytes> {
        // 过滤 GOP 缓存中已发送的历史帧
        if pkt.pts_ms <= self.last_gop_pts {
            return Vec::new();
        }

        let mut out_tags = Vec::new();
        let clean = strip_nalu_start_code(&pkt.payload);

        match pkt.codec {
            CodecType::H264 => {
                if clean.is_empty() {
                    return Vec::new();
                }
                let nal_type = clean[0] & 0x1F;
                if nal_type == 7 {
                    self.sps_buf = Some(Bytes::copy_from_slice(clean));
                } else if nal_type == 8 {
                    self.pps_buf = Some(Bytes::copy_from_slice(clean));
                }

                if !self.sent_sequence_header {
                    if let (Some(sps), Some(pps)) = (&self.sps_buf, &self.pps_buf) {
                        if let Some(seq_tag) = FlvMuxer::build_h264_sequence_header(sps, pps) {
                            out_tags.push(seq_tag);
                            self.sent_sequence_header = true;
                        }
                    }
                }

                // SPS(7), PPS(8), AUD(9), SEI(6) 不作为单独视频 Tag 下发
                if nal_type == 6 || nal_type == 7 || nal_type == 8 || nal_type == 9 {
                    return out_tags;
                }
            }
            CodecType::H265 => {
                if clean.len() < 2 {
                    return Vec::new();
                }
                let nal_type = (clean[0] >> 1) & 0x3F;
                if nal_type == 32 {
                    self.vps_buf = Some(Bytes::copy_from_slice(clean));
                } else if nal_type == 33 {
                    self.sps_buf = Some(Bytes::copy_from_slice(clean));
                } else if nal_type == 34 {
                    self.pps_buf = Some(Bytes::copy_from_slice(clean));
                }

                if !self.sent_sequence_header {
                    if let (Some(sps), Some(pps)) = (&self.sps_buf, &self.pps_buf) {
                        if let Some(seq_tag) =
                            FlvMuxer::build_h265_sequence_header(self.vps_buf.as_deref(), sps, pps)
                        {
                            out_tags.push(seq_tag);
                            self.sent_sequence_header = true;
                        }
                    }
                }

                // VPS(32), SPS(33), PPS(34), AUD(35), SEI(39, 40) 不作为单独视频 Tag 下发
                if (32..=35).contains(&nal_type) || nal_type == 39 || nal_type == 40 {
                    return out_tags;
                }
            }
        }

        // 严格对齐首个关键帧：未发送 Sequence Header 或尚未收到第一个关键帧前，丢弃非关键帧
        if !self.sent_sequence_header {
            return out_tags;
        }

        if !self.has_first_keyframe {
            if pkt.is_keyframe {
                self.has_first_keyframe = true;
            } else {
                return out_tags;
            }
        }

        let base_pts = *self.base_pts_ms.get_or_insert(pkt.pts_ms);
        self.frame_count += 1;

        let tag = FlvMuxer::packet_to_flv_tag_with_filter(pkt, base_pts, &mut self.last_flv_ts);
        out_tags.push(tag);
        out_tags
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_flv_header_structure() {
        let header = FlvMuxer::flv_header();
        assert_eq!(header.len(), 13);
        assert_eq!(&header[..3], b"FLV");
        assert_eq!(header[4], 0x01); // Video only
    }

    #[test]
    fn test_h264_flv_sequence_header() {
        let sps = vec![0x67, 0x42, 0x00, 0x1E];
        let pps = vec![0x68, 0xCE];
        let tag = FlvMuxer::build_h264_sequence_header(&sps, &pps).expect("build h264 seq header");
        assert!(tag.len() > 15);
        assert_eq!(tag[0], 0x09); // Video tag
        assert_eq!(tag[11], 0x17); // Keyframe + AVC
        assert_eq!(tag[12], 0x00); // Sequence Header
    }

    #[test]
    fn test_strip_nalu_start_code() {
        let with_4b = [0x00, 0x00, 0x00, 0x01, 0x67, 0x42];
        assert_eq!(strip_nalu_start_code(&with_4b), &[0x67, 0x42]);

        let with_3b = [0x00, 0x00, 0x01, 0x65, 0x88];
        assert_eq!(strip_nalu_start_code(&with_3b), &[0x65, 0x88]);

        let raw = [0x65, 0x88];
        assert_eq!(strip_nalu_start_code(&raw), &[0x65, 0x88]);
    }

    #[test]
    fn test_packet_to_flv_tag_timestamp_monotonic() {
        let pkt1 = EncodedPacket {
            pts_ms: 1000,
            is_keyframe: true,
            codec: CodecType::H264,
            payload: Bytes::from_static(&[0x65, 0x88]),
        };
        let pkt2 = EncodedPacket {
            pts_ms: 900, // 异常回退时间戳
            is_keyframe: false,
            codec: CodecType::H264,
            payload: Bytes::from_static(&[0x41, 0x00]),
        };

        let mut last_ts = 0u32;
        let _tag1 = FlvMuxer::packet_to_flv_tag_with_filter(&pkt1, 1000, &mut last_ts);
        assert_eq!(last_ts, 0);

        let _tag2 = FlvMuxer::packet_to_flv_tag_with_filter(&pkt2, 1000, &mut last_ts);
        // 时间戳被看门狗拉平至不小于上一次
        assert_eq!(last_ts, 0);
    }

    #[test]
    fn test_flv_stream_pipeline_lifecycle() {
        let mut pipeline = FlvStreamPipeline::new();
        let cache = KeyframeCache {
            codec: Some(CodecType::H264),
            sps: Some(Bytes::from_static(&[0x67, 0x42, 0x00, 0x1E])),
            pps: Some(Bytes::from_static(&[0x68, 0xCE])),
            gop_packets: vec![Arc::new(EncodedPacket {
                pts_ms: 500,
                is_keyframe: true,
                codec: CodecType::H264,
                payload: Bytes::from_static(&[0x65, 0x88]),
            })],
            ..Default::default()
        };

        let init_tags = pipeline.inject_cache(&cache, CodecType::H264);
        assert_eq!(init_tags.len(), 2); // Seq header + 1 GOP packet
        assert!(pipeline.sent_sequence_header);
        assert!(pipeline.has_first_keyframe);

        let new_pkt = EncodedPacket {
            pts_ms: 540,
            is_keyframe: false,
            codec: CodecType::H264,
            payload: Bytes::from_static(&[0x41, 0x01]),
        };
        let live_tags = pipeline.process_packet(&new_pkt);
        assert_eq!(live_tags.len(), 1);
    }
}
