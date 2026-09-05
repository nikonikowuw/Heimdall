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

    /// 将单个 EncodedPacket 封装为 FLV Video Tag（内置 Annex B 拆分、4 字节大端长度前缀与时间戳单调递增看门狗）
    /// 将单个 EncodedPacket 按照指定的 DTS 与 CTS 封装为 FLV Video Tag
    ///
    /// - `dts_ms`: FLV Tag Header 记录的解码时间戳（单调递增）
    /// - `cts_ms`: FLV Video Header 记录的合成时间偏移（CTS = PTS - DTS，必须 >= 0）
    pub fn packet_to_flv_tag_with_dts_cts(pkt: &EncodedPacket, dts_ms: u32, cts_ms: u32) -> Bytes {
        let nalus = crate::sps::split_annex_b_nalus(&pkt.payload);
        if nalus.is_empty() {
            return Bytes::new();
        }

        let is_vcl = |nalu: &[u8]| match pkt.codec {
            CodecType::H264 => !nalu.is_empty() && !matches!(nalu[0] & 0x1F, 7..=9),
            CodecType::H265 => nalu.len() >= 2 && !(32..=35).contains(&((nalu[0] >> 1) & 0x3F)),
        };

        let vcl_nalus: Vec<&[u8]> = nalus.into_iter().filter(|n| is_vcl(n)).collect();
        if vcl_nalus.is_empty() {
            return Bytes::new();
        }

        let total_payload_len: usize = vcl_nalus.iter().map(|n| 4 + n.len()).sum();
        let mut body = match pkt.codec {
            CodecType::H264 => {
                let mut b = BytesMut::with_capacity(5 + total_payload_len);
                let frame_type = if pkt.is_keyframe { 0x17 } else { 0x27 };
                b.put_u8(frame_type);
                b.put_u8(0x01); // AVCPacketType = 1 (NALU)
                                // 3 字节大端序 CompositionTime
                b.put_u8(((cts_ms >> 16) & 0xFF) as u8);
                b.put_u8(((cts_ms >> 8) & 0xFF) as u8);
                b.put_u8((cts_ms & 0xFF) as u8);
                b
            }
            CodecType::H265 => {
                let mut b = BytesMut::with_capacity(8 + total_payload_len);
                // Enhanced FLV: IsExHeader(0x80) | FrameType(0x10 / 0x20) | PacketType(0x01 = CodedFrames)
                let header_byte = 0x80 | (if pkt.is_keyframe { 0x10 } else { 0x20 }) | 0x01;
                b.put_u8(header_byte);
                b.put_slice(b"hvc1"); // FourCC
                                      // 3 字节大端序 CompositionTime
                b.put_u8(((cts_ms >> 16) & 0xFF) as u8);
                b.put_u8(((cts_ms >> 8) & 0xFF) as u8);
                b.put_u8((cts_ms & 0xFF) as u8);
                b
            }
        };

        for nalu in vcl_nalus {
            body.put_u32(nalu.len() as u32);
            body.put_slice(nalu);
        }

        Self::wrap_tag(0x09, dts_ms, &body)
    }

    /// 将单个 EncodedPacket 封装为 FLV Video Tag (单调滤波兜底接口，CTS 默认 0)
    pub fn packet_to_flv_tag_with_filter(
        pkt: &EncodedPacket,
        base_pts_ms: i64,
        last_timestamp_ms: &mut u32,
    ) -> Bytes {
        let raw_rel_ts = (pkt.pts_ms.saturating_sub(base_pts_ms)).max(0) as u32;
        let rel_ts = raw_rel_ts.max(*last_timestamp_ms);
        *last_timestamp_ms = rel_ts;
        Self::packet_to_flv_tag_with_dts_cts(pkt, rel_ts, 0)
    }

    /// 将单个 EncodedPacket 封装为 FLV Video Tag
    pub fn packet_to_flv_tag(pkt: &EncodedPacket, base_pts_ms: i64) -> Bytes {
        let mut dummy_ts = 0;
        Self::packet_to_flv_tag_with_filter(pkt, base_pts_ms, &mut dummy_ts)
    }
}

/// B 帧时序感知与自适应时间戳矫正管理器
///
/// 攻克摄像头开启 B 帧（如 H.264 High Profile / H.265 Main Profile）导致
/// PTS 乱序到达、CompositionTime 缺失引发的浏览器画面倒退与抽搐问题。
/// 具备零延迟双模自愈能力：无 B 帧时 0ms 延迟直通，有 B 帧时自适应平滑推导严格单调 DTS 与非负 CTS。
#[derive(Debug, Clone)]
pub struct BFrameTimeManager {
    /// 是否已确认为包含 B 帧的时序
    pub has_b_frames: bool,
    /// 观察到的最大输入相对 PTS (毫秒)
    pub max_input_rel_pts: u32,
    /// 上一次输出的 DTS (毫秒)
    pub last_dts: u32,
    /// 是否已完成第一帧初始化
    pub initialized: bool,
    /// 估算的平均帧间隔 (默认 40ms 对应 25fps)
    pub estimated_interval: u32,
    /// 上一次输入的原始 PTS
    pub prev_raw_pts: Option<i64>,
    /// CTS 延迟补偿量 (确保 target_pts >= DTS)
    pub cts_delay: u32,
}

impl Default for BFrameTimeManager {
    fn default() -> Self {
        Self {
            has_b_frames: false,
            max_input_rel_pts: 0,
            last_dts: 0,
            initialized: false,
            estimated_interval: 40,
            prev_raw_pts: None,
            cts_delay: 0,
        }
    }
}

impl BFrameTimeManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// 重置状态（如发生 Lagged 跳帧或流重连）
    pub fn reset(&mut self) {
        self.max_input_rel_pts = 0;
        self.last_dts = 0;
        self.initialized = false;
        self.prev_raw_pts = None;
        // 保留 has_b_frames 与 cts_delay，避免重连后重新经历 B 帧探测过渡期
    }

    /// 计算当前帧的 (dts_ms, cts_ms)
    pub fn calculate_dts_cts(&mut self, pkt_pts_ms: i64, base_pts_ms: i64) -> (u32, u32) {
        let rel_pts = (pkt_pts_ms.saturating_sub(base_pts_ms)).max(0) as u32;

        // 1. 动态自适应估算平均帧间隔 (FPS 自适应)
        if let Some(prev) = self.prev_raw_pts {
            let diff = (pkt_pts_ms - prev).abs();
            if (15..=120).contains(&diff) {
                self.estimated_interval =
                    ((self.estimated_interval * 3 + diff as u32) / 4).clamp(15, 100);
            }
        }
        self.prev_raw_pts = Some(pkt_pts_ms);

        // 2. 检测是否存在 B 帧时序回跳 (当前帧相对 PTS 小于历史观察到的最大相对 PTS)
        if self.initialized && rel_pts < self.max_input_rel_pts {
            let backward = self.max_input_rel_pts - rel_pts;
            if backward > 0 && backward <= 1000 {
                self.has_b_frames = true;
                // 维持稳定的 CTS 时延偏置 (通常为 200ms，足以覆盖 2~3 个 B 帧回跳且不破坏相对时序)
                self.cts_delay = self.cts_delay.max(200).max(backward * 2);
            }
        }
        if rel_pts > self.max_input_rel_pts {
            self.max_input_rel_pts = rel_pts;
        }

        // 3. 计算 DTS 与 CTS
        if !self.initialized {
            self.initialized = true;
            self.last_dts = 0;
            (0, 0)
        } else if !self.has_b_frames {
            // 直通模式 (无 B 帧)：DTS 严格单调递增紧跟 PTS，CTS = 0
            let dts = rel_pts.max(self.last_dts);
            self.last_dts = dts;
            (dts, 0)
        } else {
            // 自适应 B 帧模式：推导单调严格递增 DTS，并计算非负 CTS
            let mut dts = self.last_dts + self.estimated_interval;
            if dts <= self.last_dts {
                dts = self.last_dts + 1;
            }

            let target_pts = rel_pts + self.cts_delay;
            if dts > target_pts {
                dts = target_pts;
            }

            self.last_dts = dts;
            let cts = target_pts.saturating_sub(dts);
            (dts, cts)
        }
    }
}

/// FLV 实时流生成管道状态机（统一封装 Sequence Header、GOP 注入、参数集解析与 B 帧自适应时序矫正）
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
    pub bframe_mgr: BFrameTimeManager,
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
                let (dts, cts) = self.bframe_mgr.calculate_dts_cts(gop_pkt.pts_ms, first_pts);
                let tag = FlvMuxer::packet_to_flv_tag_with_dts_cts(gop_pkt, dts, cts);
                if !tag.is_empty() {
                    tags.push(tag);
                    self.frame_count += 1;
                    self.last_flv_ts = dts;
                }
            }
            self.last_gop_pts = cache.gop_packets.last().map(|p| p.pts_ms).unwrap_or(0);
            self.has_first_keyframe = true;
        }

        tags
    }

    /// 处理消费端 Lagged 事件（重置关键帧对齐标记）
    pub fn handle_lagged(&mut self) {
        self.has_first_keyframe = false;
        self.bframe_mgr.reset();
    }

    /// 处理实时收到的单个 EncodedPacket，返回待下发的 FLV Tags（可能包含 Sequence Header + 视频帧 Tag）
    pub fn process_packet(&mut self, pkt: &EncodedPacket) -> Vec<Bytes> {
        // 过滤 GOP 缓存中已发送的历史帧
        if pkt.pts_ms <= self.last_gop_pts {
            return Vec::new();
        }

        let mut out_tags = Vec::new();
        let nalus = crate::sps::split_annex_b_nalus(&pkt.payload);
        if nalus.is_empty() {
            return Vec::new();
        }

        match pkt.codec {
            CodecType::H264 => {
                for nalu in &nalus {
                    if !nalu.is_empty() {
                        match nalu[0] & 0x1F {
                            7 => self.sps_buf = Some(Bytes::copy_from_slice(nalu)),
                            8 => self.pps_buf = Some(Bytes::copy_from_slice(nalu)),
                            _ => {}
                        }
                    }
                }

                if !self.sent_sequence_header {
                    if let (Some(sps), Some(pps)) = (&self.sps_buf, &self.pps_buf) {
                        if let Some(seq_tag) = FlvMuxer::build_h264_sequence_header(sps, pps) {
                            out_tags.push(seq_tag);
                            self.sent_sequence_header = true;
                        }
                    }
                }
            }
            CodecType::H265 => {
                for nalu in &nalus {
                    if nalu.len() >= 2 {
                        match (nalu[0] >> 1) & 0x3F {
                            32 => self.vps_buf = Some(Bytes::copy_from_slice(nalu)),
                            33 => self.sps_buf = Some(Bytes::copy_from_slice(nalu)),
                            34 => self.pps_buf = Some(Bytes::copy_from_slice(nalu)),
                            _ => {}
                        }
                    }
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
        let (dts, cts) = self.bframe_mgr.calculate_dts_cts(pkt.pts_ms, base_pts);
        let tag = FlvMuxer::packet_to_flv_tag_with_dts_cts(pkt, dts, cts);
        if !tag.is_empty() {
            self.frame_count += 1;
            self.last_flv_ts = dts;
            out_tags.push(tag);
        }
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

    #[test]
    fn test_flv_stream_pipeline_compound_annex_b_keyframe() {
        let mut pipeline = FlvStreamPipeline::new();
        // 模拟 Retina 输出的复合关键帧（包含 Annex B SPS + PPS + IDR）
        let compound_payload = [
            // SPS (Type 7)
            0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1E, // PPS (Type 8)
            0x00, 0x00, 0x00, 0x01, 0x68, 0xCE, // IDR (Type 5)
            0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0xAA,
        ];
        let compound_pkt = EncodedPacket {
            pts_ms: 1000,
            is_keyframe: true,
            codec: CodecType::H264,
            payload: Bytes::copy_from_slice(&compound_payload),
        };

        let tags = pipeline.process_packet(&compound_pkt);
        // 应该先产生 Sequence Header Tag，随后紧跟 IDR Video Tag（不包含 SPS/PPS）
        assert_eq!(tags.len(), 2);
        assert!(pipeline.sent_sequence_header);
        assert!(pipeline.has_first_keyframe);
        assert_eq!(tags[0][0], 0x09); // Video tag
        assert_eq!(tags[0][11], 0x17); // Keyframe + AVC
        assert_eq!(tags[0][12], 0x00); // Sequence Header (AVCDecoderConfigurationRecord)

        assert_eq!(tags[1][0], 0x09); // Video tag
        assert_eq!(tags[1][11], 0x17); // Keyframe + AVC
        assert_eq!(tags[1][12], 0x01); // AVCPacketType = 1 (NALU)
                                       // 验证 IDR 载荷符合 AVCC 格式：4 字节长度 (3) + 3 字节 IDR 数据 [0x65, 0x88, 0xAA]
                                       // 排除末尾 4 字节的 FLV PreviousTagSize
        let body_slice = &tags[1][16..tags[1].len() - 4];
        assert_eq!(body_slice, &[0x00, 0x00, 0x00, 0x03, 0x65, 0x88, 0xAA]);
    }

    #[test]
    fn test_bframe_time_manager_without_b_frames() {
        let mut mgr = BFrameTimeManager::new();
        let base_pts = 1000;
        let p0 = mgr.calculate_dts_cts(1000, base_pts);
        let p1 = mgr.calculate_dts_cts(1040, base_pts);
        let p2 = mgr.calculate_dts_cts(1080, base_pts);

        assert!(!mgr.has_b_frames);
        assert_eq!(p0, (0, 0));
        assert_eq!(p1, (40, 0));
        assert_eq!(p2, (80, 0));
    }

    #[test]
    fn test_bframe_time_manager_with_b_frames() {
        let mut mgr = BFrameTimeManager::new();
        let base_pts = 1000;
        // 模拟 25fps 下 IBBPBBP 结构到达顺序 (解码/网络到达顺序)
        // I0(0ms), P3(120ms), B1(40ms), B2(80ms), P6(240ms), B4(160ms), B5(200ms)
        let stream = vec![
            1000, // I0
            1120, // P3 (提前到达)
            1040, // B1 (回跳)
            1080, // B2
            1240, // P6
            1160, // B4
            1200, // B5
        ];

        let results: Vec<(u32, u32)> = stream
            .into_iter()
            .map(|pts| mgr.calculate_dts_cts(pts, base_pts))
            .collect();

        assert!(mgr.has_b_frames, "应该自适应检测到 B 帧存在");

        // 验证 DTS 严格单调递增
        for i in 1..results.len() {
            assert!(
                results[i].0 > results[i - 1].0,
                "DTS 必须严格单调递增: dts[{}]={} <= dts[{}]={}",
                i,
                results[i].0,
                i - 1,
                results[i - 1].0
            );
        }

        // 验证 CTS 全部非负
        for (i, &(_, cts)) in results.iter().enumerate() {
            assert!(cts < 1000, "CTS 偏移在合理范围内 [{}]", i);
        }

        // 验证进入稳态后 (Packet 4, 5, 6)，显示时间 PTS = DTS + CTS 的播放顺序
        let p6_pts = results[4].0 + results[4].1;
        let b4_pts = results[5].0 + results[5].1;
        let b5_pts = results[6].0 + results[6].1;

        assert!(
            b4_pts < b5_pts && b5_pts < p6_pts,
            "播放顺序应精准恢复为人眼期望的 B4({}) -> B5({}) -> P6({})",
            b4_pts,
            b5_pts,
            p6_pts
        );
    }

    #[test]
    fn test_flv_video_tag_with_composition_time() {
        let pkt = EncodedPacket {
            pts_ms: 1120,
            is_keyframe: false,
            codec: CodecType::H264,
            payload: Bytes::from_static(&[0x00, 0x00, 0x00, 0x01, 0x41, 0x01]),
        };

        // 指定 DTS = 40ms, CTS = 80ms
        let tag = FlvMuxer::packet_to_flv_tag_with_dts_cts(&pkt, 40, 80);
        assert_eq!(tag[0], 0x09); // Video tag
                                  // Tag Header DTS 时间戳 (3 字节)
        assert_eq!(tag[4], 0x00);
        assert_eq!(tag[5], 0x00);
        assert_eq!(tag[6], 40);

        // Video Tag Header:
        // byte 11: FrameType (0x27)
        // byte 12: AVCPacketType (0x01)
        // byte 13..15: CompositionTime (80ms -> 0x00, 0x00, 0x50)
        assert_eq!(tag[11], 0x27);
        assert_eq!(tag[12], 0x01);
        assert_eq!(tag[13], 0x00);
        assert_eq!(tag[14], 0x00);
        assert_eq!(tag[15], 0x50); // 80 in hex
    }
}
