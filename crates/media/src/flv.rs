use bytes::{BufMut, Bytes, BytesMut};
use types::{CodecType, EncodedPacket, StreamTag};

use crate::dispatcher::KeyframeCache;

pub const MAX_FLV_TIMESTAMP_MS: i64 = u32::MAX as i64;

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

/// FLV 容器封装器（支持标准 H.264 与 Enhanced FLV H.265 / HEVC，以及 AAC 音频）
#[derive(Debug)]
pub struct FlvMuxer;

impl FlvMuxer {
    /// 生成标准 9 字节 FLV 文件头 + 4 字节 PreviousTagSize0。
    ///
    /// 默认生成视频流；`include_audio` 为 `true` 时同时声明音频轨道。
    pub fn flv_header(include_audio: bool) -> Bytes {
        Self::flv_header_tracks(include_audio, true)
    }

    /// 生成指定音视频轨道的 FLV 文件头。
    ///
    /// FLV TypeFlags 的 bit 2 表示音频、bit 0 表示视频。音频-only
    /// 通道用于在 WebCodecs 渲染 H.265 视频时独立播放 AAC 音轨。
    pub fn flv_header_tracks(include_audio: bool, include_video: bool) -> Bytes {
        let mut b = BytesMut::with_capacity(13);
        let type_flags =
            (if include_audio { 0x04 } else { 0x00 }) | (if include_video { 0x01 } else { 0x00 });

        // "FLV" + version 1 + Flags + DataOffset (9) + PreviousTagSize0 (0)
        b.extend_from_slice(&[
            b'F', b'L', b'V', 0x01, // Signature & Version
            type_flags, 0x00, 0x00, 0x00, 0x09, // DataOffset
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

    /// 生成指定时间戳的 H.264 AVC Sequence Header (AVCDecoderConfigurationRecord / SPS + PPS)
    pub fn build_h264_sequence_header_with_ts(
        sps: &[u8],
        pps: &[u8],
        timestamp_ms: u32,
    ) -> Option<Bytes> {
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

        Some(Self::wrap_tag(0x09, timestamp_ms, &body))
    }

    /// 生成 H.264 AVC Sequence Header (默认时间戳 0)
    pub fn build_h264_sequence_header(sps: &[u8], pps: &[u8]) -> Option<Bytes> {
        Self::build_h264_sequence_header_with_ts(sps, pps, 0)
    }

    /// 生成指定时间戳的 Enhanced FLV H.265 Sequence Header (HEVCDecoderConfigurationRecord / VPS + SPS + PPS)
    pub fn build_h265_sequence_header_with_ts(
        vps: Option<&[u8]>,
        sps: &[u8],
        pps: &[u8],
        timestamp_ms: u32,
    ) -> Option<Bytes> {
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

        Some(Self::wrap_tag(0x09, timestamp_ms, &body))
    }

    /// 生成 Enhanced FLV H.265 Sequence Header (默认时间戳 0)
    pub fn build_h265_sequence_header(vps: Option<&[u8]>, sps: &[u8], pps: &[u8]) -> Option<Bytes> {
        Self::build_h265_sequence_header_with_ts(vps, sps, pps, 0)
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
            CodecType::Aac => None,
        }
    }

    /// 将单个 EncodedPacket 按照指定的 DTS 与 CTS 封装为 FLV Video Tag
    ///
    /// - `dts_ms`: FLV Tag Header 记录的解码时间戳（单调递增）
    /// - `cts_ms`: FLV Video Header 记录的合成时间偏移（CTS = PTS - DTS，必须 >= 0）
    pub fn packet_to_flv_tag_with_dts_cts(pkt: &EncodedPacket, dts_ms: u32, cts_ms: u32) -> Bytes {
        if !pkt.codec.is_video() || pkt.stream_tag == StreamTag::Audio {
            return Bytes::new();
        }

        let nalus = crate::sps::split_annex_b_nalus(&pkt.payload);
        if nalus.is_empty() {
            return Bytes::new();
        }

        let is_vcl = |nalu: &[u8]| match pkt.codec {
            CodecType::H264 => !nalu.is_empty() && !matches!(nalu[0] & 0x1F, 7..=9),
            CodecType::H265 => nalu.len() >= 2 && !(32..=35).contains(&((nalu[0] >> 1) & 0x3F)),
            CodecType::Aac => false,
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
            CodecType::Aac => return Bytes::new(),
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
        let raw_rel_ts = pkt.pts_ms.saturating_sub(base_pts_ms).max(0);
        let rel_ts = raw_rel_ts.min(MAX_FLV_TIMESTAMP_MS) as u32;
        let rel_ts = rel_ts.max(*last_timestamp_ms);
        *last_timestamp_ms = rel_ts;
        Self::packet_to_flv_tag_with_dts_cts(pkt, rel_ts, 0)
    }

    /// 将单个 EncodedPacket 封装为 FLV Video Tag
    pub fn packet_to_flv_tag(pkt: &EncodedPacket, base_pts_ms: i64) -> Bytes {
        let mut dummy_ts = 0;
        Self::packet_to_flv_tag_with_filter(pkt, base_pts_ms, &mut dummy_ts)
    }

    /// 从 ADTS 帧头解析采样率索引 (4-bit SampleRateIndex)
    ///
    /// ADTS 频率索引映射 (ISO 14496-3 Table 1.16):
    /// 0=96kHz, 1=88.2kHz, 2=64kHz, 3=48kHz, 4=44.1kHz,
    /// 5=32kHz, 6=24kHz, 7=22.05kHz, 8=16kHz, 9=12kHz,
    /// 10=11.025kHz, 11=8kHz, 12=7.35kHz
    fn adts_sampling_freq_index(sampling_freq: u32) -> u8 {
        match sampling_freq {
            96000 => 0,
            88200 => 1,
            64000 => 2,
            48000 => 3,
            44100 => 4,
            32000 => 5,
            24000 => 6,
            22050 => 7,
            16000 => 8,
            12000 => 9,
            11025 => 10,
            8000 => 11,
            7350 => 12,
            _ => 4, // 默认 44.1kHz
        }
    }

    /// 构建 AAC AudioSpecificConfig (2 字节，用于 FLV AudioSequenceHeader)
    ///
    /// AAC-LC (AudioObjectType=2) 编码为 5 bit:
    /// ```text
    /// bits:   [audioObjectType 5bit][sampleRateIndex 4bit][channelConfig 4bit][padding 3bit]
    /// ```
    fn build_aac_audio_specific_config(sample_rate: u32, channels: u8) -> [u8; 2] {
        let aot: u8 = 2; // AAC-LC (most common in surveillance cameras)
        let freq_idx = Self::adts_sampling_freq_index(sample_rate);
        let chan_config = channels.min(7); // FLV channel config: 1-7

        // 5-bit AudioObjectType + 4-bit SampleRateIndex = 9 bits across 2 bytes
        // Byte 0: AOT[4:0] (5 bits) + FreqIdx[3:1] (3 bits)
        // Byte 1: FreqIdx[0] (1 bit) + ChanConfig[3:0] (4 bits) + padding[2:0] (3 bits)
        let byte0 = (aot << 3) | (freq_idx >> 1);
        let byte1 = ((freq_idx & 0x01) << 7) | (chan_config << 3);
        [byte0, byte1]
    }

    /// 从 ADTS 帧头 (7 字节) 解析采样率与声道数
    ///
    /// ADTS 头部关键字段布局 (ISO 14496-3 AudioTransport / ISO 13818-7):
    /// ```text
    /// Byte 0: [sync_word:8=0xFF]
    /// Byte 1: [sync_word:4=0xF][ID:1][Layer:2][ProtectionAbsent:1]
    /// Byte 2: [Profile:2][SamplingFreqIndex:4][Private:1][ChannelConfig_high:1]
    /// Byte 3: [ChannelConfig_low:2][Original:1][Home:1][CopyrightID:1][CopyrightStart:1][FrameLength_high:2]
    /// ```
    fn parse_adts_header(adts: &[u8]) -> Option<(u32, u8)> {
        if adts.len() < 7 || (adts[0] != 0xFF || (adts[1] & 0xF0) != 0xF0) {
            return None;
        }
        // SamplingFreqIndex: bits 5..=2 of byte 2 (4 bits)
        let freq_idx = (adts[2] >> 2) & 0x0F;
        // ChannelConfiguration: bit 0 of byte 2 (MSB) + bits 7..=6 of byte 3 (2 LSBs) = 3 bits total
        let channels = ((adts[2] & 0x01) << 2) | ((adts[3] >> 6) & 0x03);

        let sample_rate = match freq_idx {
            0 => 96000,
            1 => 88200,
            2 => 64000,
            3 => 48000,
            4 => 44100,
            5 => 32000,
            6 => 24000,
            7 => 22050,
            8 => 16000,
            9 => 12000,
            10 => 11025,
            11 => 8000,
            12 => 7350,
            _ => 44100,
        };
        Some((sample_rate, channels))
    }

    /// 构建 AAC Audio Sequence Header FLV Tag
    ///
    /// FLV AudioSequenceHeader 包含 AudioSpecificConfig，
    /// 供 mpegts.js / flv.js 初始化 AAC 解码器。
    ///
    /// Audio Tag Header (2 bytes):
    /// - Byte 0: SoundFormat(4bit=10=AAC) | SoundRate(2bit=3=44kHz) | SoundSize(1bit=1=16bit) | SoundType(1bit=1=Stereo)
    ///   注：依据 Adobe Flash Video Spec v10.1 (Annex E.4.2.1)，对于 AAC 编码，
    ///   SoundFormat 必须为 10，SoundRate/SoundSize/SoundType 固定为 3/1/1（0xAF 占位符），
    ///   真实采样率与声道数由后续的 AudioSpecificConfig 权威指定。
    /// - Byte 1: AACPacketType (0 = AAC sequence header)
    pub fn build_aac_sequence_header(sample_rate: u32, channels: u8, timestamp_ms: u32) -> Bytes {
        let asc = Self::build_aac_audio_specific_config(sample_rate, channels);
        let mut body = BytesMut::with_capacity(4);
        // Audio Tag Header: AAC(10) | 44kHz(3) | 16bit(1) | Stereo(1) = 0xAF (Adobe FLV 规范占位符)
        body.put_u8(0xAF);
        // AACPacketType = 0 (AAC sequence header)
        body.put_u8(0x00);
        // AudioSpecificConfig (2 bytes)
        body.put_slice(&asc);

        Self::wrap_tag(0x08, timestamp_ms, &body)
    }

    /// 将一帧 ADTS 封装的 AAC 音频数据转换为 FLV Audio Tag
    ///
    /// 1. 解析 ADTS 帧头获取采样率/声道数 (校验 ADTS 同步字有效性)
    /// 2. 依据 ProtectionAbsent 标志剥离 7 或 9 字节 ADTS 头部，提取原始 AAC 帧数据
    /// 3. 封装为 FLV Audio Tag: [AudioTagHeader(2B)] + [Raw AAC Frame Data]
    ///
    /// 返回 `None` 表示 ADTS 帧头无效，应静默丢弃。
    pub fn adts_to_flv_audio_tag(adts_data: &[u8], timestamp_ms: u32) -> Option<Bytes> {
        if adts_data.len() < 7 {
            return None;
        }

        let (_sample_rate, _channels) = Self::parse_adts_header(adts_data)?;

        // ADTS 头部长度：bit 0 of byte 1 为 protection_absent: 1=7字节(无CRC), 0=9字节(带CRC)
        let header_len = if adts_data[1] & 0x01 == 0 { 9 } else { 7 };
        if adts_data.len() <= header_len {
            return None;
        }
        let raw_aac = &adts_data[header_len..];

        let mut body = BytesMut::with_capacity(2 + raw_aac.len());
        // Audio Tag Header: AAC(10) | 44kHz(3) | 16bit(1) | Stereo(1) = 0xAF (Adobe FLV 规范占位符)
        body.put_u8(0xAF);
        // AACPacketType = 1 (AAC raw data)
        body.put_u8(0x01);
        body.put_slice(raw_aac);

        Some(Self::wrap_tag(0x08, timestamp_ms, &body))
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
        let rel_pts = pkt_pts_ms
            .saturating_sub(base_pts_ms)
            .clamp(0, MAX_FLV_TIMESTAMP_MS) as u32;

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
    /// 当前生效的活跃参数集指纹 (Active SPS/PPS/VPS)，用于动态检测摄像头白天/黑夜模式切换与分辨率突变
    pub active_sps: Option<Bytes>,
    pub active_pps: Option<Bytes>,
    pub active_vps: Option<Bytes>,
    pub frame_count: u64,
    pub bframe_mgr: BFrameTimeManager,
    /// 是否向此通道输出音频帧；仅 Hero 主预览窗口开启，避免多路声音污染
    pub include_audio: bool,
    /// 已发送 AAC Audio Sequence Header，防止重复发送
    pub sent_audio_header: bool,
    /// 缓存从首个音频帧 ADTS 头部解析出的采样率 (Hz)，用于后续帧的 AudioSpecificConfig
    pub audio_sample_rate: Option<u32>,
    /// 缓存声道数
    pub audio_channels: Option<u8>,
    /// 上一次输出的音频时间戳 (FLV DTS，毫秒)，用于确保音频时间戳单调非递减
    pub last_audio_pts: u32,
}

impl FlvStreamPipeline {
    pub fn new(include_audio: bool) -> Self {
        Self {
            include_audio,
            ..Default::default()
        }
    }

    /// 处理音频帧：首次解析 ADTS 头部并发送 AAC Sequence Header，后续直接封装为 FLV Audio Tag
    ///
    /// 仅在 `include_audio = true` 时有效；否则直接返回空 Vec。
    /// 音频时间戳严格对齐 `base_pts_ms`，且保持单调非递减。
    pub fn process_audio_packet(&mut self, pkt: &EncodedPacket) -> Vec<Bytes> {
        if !self.include_audio || pkt.stream_tag != StreamTag::Audio || !pkt.codec.is_audio() {
            return Vec::new();
        }

        let adts_data = pkt.payload.as_ref();
        let base_pts = *self.base_pts_ms.get_or_insert(pkt.pts_ms);
        let raw_rel_pts = pkt.pts_ms.saturating_sub(base_pts).max(0);
        if raw_rel_pts > MAX_FLV_TIMESTAMP_MS {
            tracing::warn!(
                pts_ms = pkt.pts_ms,
                base_pts_ms = base_pts,
                "AAC FLV 时间戳超过 u32 上限，重置封装时间基准并等待新的关键帧"
            );
            self.reset_after_discontinuity();
            return Vec::new();
        }
        let dts = (raw_rel_pts as u32).max(self.last_audio_pts);
        self.last_audio_pts = dts;

        // 1. 首帧：解析 ADTS 头部获取采样率与声道数，发送 AAC Sequence Header
        if !self.sent_audio_header {
            if let Some((sample_rate, channels)) = FlvMuxer::parse_adts_header(adts_data) {
                self.audio_sample_rate = Some(sample_rate);
                self.audio_channels = Some(channels);
                let seq_tag = FlvMuxer::build_aac_sequence_header(sample_rate, channels, dts);
                self.sent_audio_header = true;
                return vec![seq_tag];
            }
            // ADTS 帧头无效，静默丢弃
            return Vec::new();
        }

        // 2. 后续帧：直接封装为 FLV Audio Tag
        match FlvMuxer::adts_to_flv_audio_tag(adts_data, dts) {
            Some(tag) => vec![tag],
            None => Vec::new(),
        }
    }

    /// 从 KeyframeCache 注入首屏 Sequence Header 与完整 GOP 关键帧包
    pub fn inject_cache(&mut self, cache: &KeyframeCache, fallback_codec: CodecType) -> Vec<Bytes> {
        let mut tags = Vec::new();
        let codec = cache.codec.unwrap_or(fallback_codec);

        if let Some(seq_tag) = FlvMuxer::build_sequence_header_from_cache(codec, cache) {
            tags.push(seq_tag);
            self.sent_sequence_header = true;
            self.sps_buf = cache.sps.clone();
            self.pps_buf = cache.pps.clone();
            self.vps_buf = cache.vps.clone();
            self.active_sps = cache.sps.clone();
            self.active_pps = cache.pps.clone();
            self.active_vps = cache.vps.clone();
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

    /// 明确的解码不连续点重置。与旧版 Lagged 重置相同，但名称表达了 Replay/SourceReset 契约。
    pub fn reset_after_discontinuity(&mut self) {
        self.sent_sequence_header = false;
        self.has_first_keyframe = false;
        self.base_pts_ms = None;
        self.last_flv_ts = 0;
        self.last_gop_pts = 0;
        self.sps_buf = None;
        self.pps_buf = None;
        self.vps_buf = None;
        self.active_sps = None;
        self.active_pps = None;
        self.active_vps = None;
        self.last_audio_pts = 0;
        self.sent_audio_header = false;
        self.audio_sample_rate = None;
        self.audio_channels = None;
        self.bframe_mgr.reset();
    }

    /// 处理消费端 Lagged 事件（保留兼容调用，内部统一走显式不连续点重置）。
    pub fn handle_lagged(&mut self) {
        self.reset_after_discontinuity();
    }

    /// 处理实时收到的单个 EncodedPacket，返回待下发的 FLV Tags（可能包含 Sequence Header + 视频帧 Tag）
    pub fn process_packet(&mut self, pkt: &EncodedPacket) -> Vec<Bytes> {
        // 过滤音频包或非视频包
        if pkt.stream_tag == StreamTag::Audio || !pkt.codec.is_video() {
            return Vec::new();
        }

        // 过滤 GOP 缓存中已发送的历史帧
        if pkt.pts_ms <= self.last_gop_pts {
            return Vec::new();
        }

        let mut out_tags = Vec::new();
        let nalus = crate::sps::split_annex_b_nalus(&pkt.payload);
        if nalus.is_empty() {
            return Vec::new();
        }

        // 1. 提取当前数据包中携带的参数集（自动剥离可能携带的 Annex B 起始码）
        match pkt.codec {
            CodecType::H264 => {
                for nalu in &nalus {
                    if !nalu.is_empty() {
                        let clean = strip_nalu_start_code(nalu);
                        match clean[0] & 0x1F {
                            7 => self.sps_buf = Some(Bytes::copy_from_slice(clean)),
                            8 => self.pps_buf = Some(Bytes::copy_from_slice(clean)),
                            _ => {}
                        }
                    }
                }
            }
            CodecType::H265 => {
                for nalu in &nalus {
                    let clean = strip_nalu_start_code(nalu);
                    if clean.len() >= 2 {
                        match (clean[0] >> 1) & 0x3F {
                            32 => self.vps_buf = Some(Bytes::copy_from_slice(clean)),
                            33 => self.sps_buf = Some(Bytes::copy_from_slice(clean)),
                            34 => self.pps_buf = Some(Bytes::copy_from_slice(clean)),
                            _ => {}
                        }
                    }
                }
            }
            CodecType::Aac => return Vec::new(),
        }

        // 2. 检测参数集指纹是否发生动态突变 (如安防 IPC 白天/黑夜模式切换、分辨率 1080P -> 720P 切换)
        let sps_changed = self.sps_buf.is_some() && self.sps_buf != self.active_sps;
        let pps_changed = self.pps_buf.is_some() && self.pps_buf != self.active_pps;
        let vps_changed = match pkt.codec {
            CodecType::H265 => self.vps_buf.is_some() && self.vps_buf != self.active_vps,
            CodecType::H264 | CodecType::Aac => false,
        };

        let is_mutation = self.sent_sequence_header && (sps_changed || pps_changed || vps_changed);
        let is_initial =
            !self.sent_sequence_header && self.sps_buf.is_some() && self.pps_buf.is_some();

        // 3. 若尚未下发首包 Sequence Header，在提取到完整参数集后立即构建下发 (默认时间戳 0)
        if is_initial {
            if let (Some(sps), Some(pps)) = (&self.sps_buf, &self.pps_buf) {
                let seq_tag = match pkt.codec {
                    CodecType::H264 => FlvMuxer::build_h264_sequence_header(sps, pps),
                    CodecType::H265 => {
                        FlvMuxer::build_h265_sequence_header(self.vps_buf.as_deref(), sps, pps)
                    }
                    CodecType::Aac => None,
                };

                if let Some(tag) = seq_tag {
                    out_tags.push(tag);
                    self.sent_sequence_header = true;
                    self.active_sps = Some(sps.clone());
                    self.active_pps = Some(pps.clone());
                    self.active_vps = self.vps_buf.clone();
                }
            }
        }

        // 4. 严格对齐首个关键帧：未发送 Sequence Header 或尚未收到第一个关键帧前，丢弃非关键帧
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

        // 5. 计算当前数据帧的 (DTS, CTS) 时间戳
        let base_pts = *self.base_pts_ms.get_or_insert(pkt.pts_ms);
        let raw_rel_pts = pkt.pts_ms.saturating_sub(base_pts).max(0);
        if raw_rel_pts > MAX_FLV_TIMESTAMP_MS {
            tracing::warn!(
                pts_ms = pkt.pts_ms,
                base_pts_ms = base_pts,
                "视频 FLV 时间戳超过 u32 上限，重置封装时间基准并等待新的关键帧"
            );
            self.reset_after_discontinuity();
            return out_tags;
        }
        let (dts, cts) = self.bframe_mgr.calculate_dts_cts(pkt.pts_ms, base_pts);

        // 6. 若检测到参数突变 (Mutation)，立即在关键帧前注入更新的 Sequence Header Tag (时间戳对齐当前 DTS)
        if is_mutation {
            if let (Some(sps), Some(pps)) = (&self.sps_buf, &self.pps_buf) {
                let seq_tag = match pkt.codec {
                    CodecType::H264 => FlvMuxer::build_h264_sequence_header_with_ts(sps, pps, dts),
                    CodecType::H265 => FlvMuxer::build_h265_sequence_header_with_ts(
                        self.vps_buf.as_deref(),
                        sps,
                        pps,
                        dts,
                    ),
                    CodecType::Aac => None,
                };

                if let Some(tag) = seq_tag {
                    let dim_change = match pkt.codec {
                        CodecType::H264 => {
                            let old_info = self
                                .active_sps
                                .as_deref()
                                .and_then(|s| crate::sps::parse_h264_sps(s).ok());
                            let new_info = crate::sps::parse_h264_sps(sps).ok();
                            match (old_info, new_info) {
                                (Some(o), Some(n)) => {
                                    format!("{}x{} -> {}x{}", o.width, o.height, n.width, n.height)
                                }
                                _ => "参数变更".to_string(),
                            }
                        }
                        CodecType::H265 => {
                            let old_info = self
                                .active_sps
                                .as_deref()
                                .and_then(|s| crate::sps::parse_h265_sps(s).ok());
                            let new_info = crate::sps::parse_h265_sps(sps).ok();
                            match (old_info, new_info) {
                                (Some(o), Some(n)) => {
                                    format!("{}x{} -> {}x{}", o.width, o.height, n.width, n.height)
                                }
                                _ => "参数变更".to_string(),
                            }
                        }
                        CodecType::Aac => "音频".to_string(),
                    };

                    tracing::info!(
                        codec = ?pkt.codec,
                        dts,
                        resolution = %dim_change,
                        frame_count = self.frame_count,
                        "检测到视频流动态 SPS/PPS 参数突变 (白天/黑夜切换或分辨率变更)，优先下发更新 Sequence Header"
                    );

                    out_tags.push(tag);
                    self.active_sps = Some(sps.clone());
                    self.active_pps = Some(pps.clone());
                    self.active_vps = self.vps_buf.clone();
                }
            }
        }

        // 7. 封装并输出当前数据帧 Video Tag
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
        let header = FlvMuxer::flv_header(false);
        assert_eq!(header.len(), 13);
        assert_eq!(&header[..3], b"FLV");
        assert_eq!(header[4], 0x01); // Video only

        let header_audio = FlvMuxer::flv_header(true);
        assert_eq!(header_audio[4], 0x05); // Audio + Video (bit 2 + bit 0)

        let header_audio_only = FlvMuxer::flv_header_tracks(true, false);
        assert_eq!(header_audio_only[4], 0x04); // Audio only
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
    fn test_flv_timestamp_rebase_avoids_u32_wrap() {
        let keyframe = EncodedPacket {
            pts_ms: 1_000,
            is_keyframe: true,
            codec: CodecType::H264,
            payload: Bytes::from_static(
                b"\x00\x00\x00\x01\x67\x42\x00\x1e\x00\x00\x00\x01\x68\xce\x00\x00\x00\x01\x65\x88",
            ),
            ..Default::default()
        };
        let delta = EncodedPacket {
            pts_ms: 1_000 + MAX_FLV_TIMESTAMP_MS + 1,
            is_keyframe: false,
            codec: CodecType::H264,
            payload: Bytes::from_static(b"\x00\x00\x00\x01\x41\x01"),
            ..Default::default()
        };
        let mut pipeline = FlvStreamPipeline::new(false);
        assert!(!pipeline.process_packet(&keyframe).is_empty());
        assert!(pipeline.process_packet(&delta).is_empty());
        assert!(!pipeline.has_first_keyframe);
        assert_eq!(pipeline.base_pts_ms, None);

        let rebased_keyframe = EncodedPacket {
            pts_ms: delta.pts_ms + 40,
            ..keyframe
        };
        assert!(!pipeline.process_packet(&rebased_keyframe).is_empty());
        assert_eq!(pipeline.last_flv_ts, 0);
    }
    #[test]
    fn test_packet_to_flv_tag_timestamp_monotonic() {
        let pkt1 = EncodedPacket {
            pts_ms: 1000,
            is_keyframe: true,
            codec: CodecType::H264,
            payload: Bytes::from_static(&[0x65, 0x88]),
            ..Default::default()
        };
        let pkt2 = EncodedPacket {
            pts_ms: 900, // 异常回退时间戳
            is_keyframe: false,
            codec: CodecType::H264,
            payload: Bytes::from_static(&[0x41, 0x00]),
            ..Default::default()
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
        let mut pipeline = FlvStreamPipeline::new(false);
        let cache = KeyframeCache {
            codec: Some(CodecType::H264),
            sps: Some(Bytes::from_static(&[0x67, 0x42, 0x00, 0x1E])),
            pps: Some(Bytes::from_static(&[0x68, 0xCE])),
            gop_packets: vec![Arc::new(EncodedPacket {
                pts_ms: 500,
                is_keyframe: true,
                codec: CodecType::H264,
                payload: Bytes::from_static(&[0x65, 0x88]),
                ..Default::default()
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
            ..Default::default()
        };
        let live_tags = pipeline.process_packet(&new_pkt);
        assert_eq!(live_tags.len(), 1);
    }

    #[test]
    fn test_flv_stream_pipeline_compound_annex_b_keyframe() {
        let mut pipeline = FlvStreamPipeline::new(false);
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
            ..Default::default()
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
            ..Default::default()
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

    #[test]
    fn test_dynamic_sps_mutation_resolution_switch() {
        let mut pipeline = FlvStreamPipeline::new(false);

        // 真实 1080P SPS
        let sps_1080p = [
            0x67, 0x64, 0x00, 0x29, 0xac, 0x72, 0x84, 0x40, 0x78, 0x02, 0x27, 0xe5, 0xc0, 0x44,
            0x00, 0x00, 0x03, 0x00, 0x04, 0x00, 0x00, 0x03, 0x00, 0xf0, 0x3c, 0x60, 0xc6, 0x58,
        ];
        // 真实 720P SPS
        let sps_720p = [
            0x67, 0x42, 0x00, 0x1f, 0x96, 0x35, 0x40, 0xa0, 0x0b, 0x76, 0x02, 0xd4, 0x04, 0x04,
            0x05, 0x00,
        ];
        let pps = [0x68, 0xce, 0x3c, 0x80];
        let idr = [0x65, 0x88, 0xaa];

        // 辅助闭包：拼接 Annex B 复合包
        let make_compound_keyframe = |sps: &[u8], pts: i64| -> EncodedPacket {
            let mut payload = Vec::new();
            // 00 00 00 01 + SPS
            payload.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
            payload.extend_from_slice(sps);
            // 00 00 00 01 + PPS
            payload.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
            payload.extend_from_slice(&pps);
            // 00 00 00 01 + IDR
            payload.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
            payload.extend_from_slice(&idr);

            EncodedPacket {
                pts_ms: pts,
                is_keyframe: true,
                codec: CodecType::H264,
                payload: Bytes::from(payload),
                ..Default::default()
            }
        };

        // 1. 发送首包 1080P 关键帧
        let pkt1 = make_compound_keyframe(&sps_1080p, 1000);
        let tags1 = pipeline.process_packet(&pkt1);
        // 应产生 首包 Sequence Header + 1080P IDR 视频 Tag
        assert_eq!(tags1.len(), 2);
        assert_eq!(tags1[0][12], 0x00); // Sequence Header
        assert_eq!(tags1[1][12], 0x01); // Video Frame
        assert_eq!(
            pipeline.active_sps.as_deref(),
            Some(&sps_1080p[..]),
            "活跃 SPS 应记录为 1080P"
        );

        // 2. 发送普通 P 帧
        let p_pkt = EncodedPacket {
            pts_ms: 1040,
            is_keyframe: false,
            codec: CodecType::H264,
            payload: Bytes::from_static(&[0x00, 0x00, 0x00, 0x01, 0x41, 0x01]),
            ..Default::default()
        };
        let p_tags = pipeline.process_packet(&p_pkt);
        assert_eq!(p_tags.len(), 1); // 仅视频帧

        // 3. 发送相同分辨率 (1080P) 的下一个关键帧 (参数集未变)
        let pkt2 = make_compound_keyframe(&sps_1080p, 1080);
        let tags2 = pipeline.process_packet(&pkt2);
        // 参数相同，不应重复产生 Sequence Header Tag，仅产生 IDR 视频 Tag
        assert_eq!(
            tags2.len(),
            1,
            "SPS 未发生突变时，禁止冗余发送 Sequence Header"
        );
        assert_eq!(tags2[0][12], 0x01); // Video Frame

        // 4. 模拟工控 IPC 动态切换分辨率 (1080P -> 720P)
        let pkt_720p = make_compound_keyframe(&sps_720p, 2000);
        let tags_720p = pipeline.process_packet(&pkt_720p);
        // 关键帧检测到 active_sps 突变，必须优先输出全新 Sequence Header 并紧跟 720P 视频帧
        assert_eq!(
            tags_720p.len(),
            2,
            "SPS 突变时必须输出更新的 Sequence Header Tag + 关键帧"
        );
        assert_eq!(tags_720p[0][12], 0x00); // 更新后的 Sequence Header
        assert_eq!(tags_720p[1][12], 0x01); // 720P IDR 视频帧
        assert_eq!(
            pipeline.active_sps.as_deref(),
            Some(&sps_720p[..]),
            "活跃 SPS 必须无缝热更新为 720P"
        );

        // 验证更新后的 Sequence Header DTS 时间戳与随后的关键帧严格同步对齐 (2000 - 1000 = 1000ms)
        let seq_dts = ((tags_720p[0][4] as u32) << 16)
            | ((tags_720p[0][5] as u32) << 8)
            | (tags_720p[0][6] as u32);
        let frame_dts = ((tags_720p[1][4] as u32) << 16)
            | ((tags_720p[1][5] as u32) << 8)
            | (tags_720p[1][6] as u32);
        assert_eq!(
            seq_dts, frame_dts,
            "突变插入的 Sequence Header 时间戳必须对齐当前帧 DTS，保持单调性"
        );
        assert_eq!(seq_dts, 1000);

        // 5. 发送下一个 720P 关键帧 (参数已与 active_sps 保持一致)
        let pkt_720p_next = make_compound_keyframe(&sps_720p, 2040);
        let tags_720p_next = pipeline.process_packet(&pkt_720p_next);
        assert_eq!(
            tags_720p_next.len(),
            1,
            "720P 稳态后不应再次重复发送 Sequence Header"
        );

        // 6. 再次动态切回 1080P
        let pkt_1080p_return = make_compound_keyframe(&sps_1080p, 3000);
        let tags_return = pipeline.process_packet(&pkt_1080p_return);
        assert_eq!(tags_return.len(), 2, "切回 1080P 时再次触发更新");
        assert_eq!(tags_return[0][12], 0x00);
        assert_eq!(pipeline.active_sps.as_deref(), Some(&sps_1080p[..]));
    }

    #[test]
    fn test_inject_cache_mutation_flow() {
        let mut pipeline = FlvStreamPipeline::new(false);

        let sps_v1 = Bytes::from_static(&[0x67, 0x42, 0x00, 0x1E]);
        let pps = Bytes::from_static(&[0x68, 0xCE]);
        let cache = KeyframeCache {
            codec: Some(CodecType::H264),
            sps: Some(sps_v1.clone()),
            pps: Some(pps.clone()),
            ..Default::default()
        };

        // 通过 inject_cache 初始拉起
        let init_tags = pipeline.inject_cache(&cache, CodecType::H264);
        assert_eq!(init_tags.len(), 1);
        assert_eq!(pipeline.active_sps.as_ref(), Some(&sps_v1));

        // 收到相同参数包，不发 Sequence Header
        let pkt_same = EncodedPacket {
            pts_ms: 100,
            is_keyframe: true,
            codec: CodecType::H264,
            payload: Bytes::from_static(&[
                0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1E, 0x00, 0x00, 0x00, 0x01, 0x68, 0xCE,
                0x00, 0x00, 0x00, 0x01, 0x65, 0x88,
            ]),
            ..Default::default()
        };
        let tags_same = pipeline.process_packet(&pkt_same);
        assert_eq!(tags_same.len(), 1); // 仅视频帧

        // 收到突变参数包，立即发出新 Sequence Header
        let sps_v2 = [0x67, 0x64, 0x00, 0x28];
        let pkt_mutated = EncodedPacket {
            pts_ms: 200,
            is_keyframe: true,
            codec: CodecType::H264,
            payload: Bytes::from_static(&[
                0x00, 0x00, 0x00, 0x01, 0x67, 0x64, 0x00, 0x28, 0x00, 0x00, 0x00, 0x01, 0x68, 0xCE,
                0x00, 0x00, 0x00, 0x01, 0x65, 0x88,
            ]),
            ..Default::default()
        };
        let tags_mutated = pipeline.process_packet(&pkt_mutated);
        assert_eq!(tags_mutated.len(), 2);
        assert_eq!(tags_mutated[0][12], 0x00); // New Sequence Header
        assert_eq!(pipeline.active_sps.as_deref(), Some(&sps_v2[..]));
    }

    #[test]
    fn test_dynamic_h265_vps_sps_pps_mutation() {
        let mut pipeline = FlvStreamPipeline::new(false);

        // 构造两个不同 SPS 的 H.265 复合关键帧
        let vps = [0x40, 0x01, 0x0c, 0x01, 0xff];
        let sps_v1 = [
            0x42, 0x01, 0x01, 0x01, 0x60, 0x00, 0x00, 0x03, 0x00, 0x00, 0x03, 0x00, 0x00, 0x03,
            0x00, 0x78,
        ];
        let sps_v2 = [
            0x42, 0x01, 0x01, 0x02, 0x60, 0x00, 0x00, 0x03, 0x00, 0x00, 0x03, 0x00, 0x00, 0x03,
            0x00, 0x90,
        ];
        let pps = [0x44, 0x01, 0xc0];
        let idr = [0x26, 0x01, 0xaf];

        let make_h265_pkt = |sps: &[u8], pts: i64| -> EncodedPacket {
            let mut payload = Vec::new();
            payload.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
            payload.extend_from_slice(&vps);
            payload.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
            payload.extend_from_slice(sps);
            payload.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
            payload.extend_from_slice(&pps);
            payload.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
            payload.extend_from_slice(&idr);

            EncodedPacket {
                pts_ms: pts,
                is_keyframe: true,
                codec: CodecType::H265,
                payload: Bytes::from(payload),
                ..Default::default()
            }
        };

        // 1. 首包 H.265 关键帧
        let tags1 = pipeline.process_packet(&make_h265_pkt(&sps_v1, 1000));
        assert_eq!(tags1.len(), 2);
        assert_eq!(tags1[0][11], 0x90); // Enhanced FLV Sequence Header
        assert_eq!(&tags1[0][12..16], b"hvc1");
        assert_eq!(pipeline.active_sps.as_deref(), Some(&sps_v1[..]));

        // 2. 相同 SPS H.265 帧
        let tags2 = pipeline.process_packet(&make_h265_pkt(&sps_v1, 1040));
        assert_eq!(tags2.len(), 1); // 仅视频帧

        // 3. 突变 SPS H.265 帧
        let tags_mutated = pipeline.process_packet(&make_h265_pkt(&sps_v2, 2000));
        assert_eq!(tags_mutated.len(), 2);
        assert_eq!(tags_mutated[0][11], 0x90);
        assert_eq!(pipeline.active_sps.as_deref(), Some(&sps_v2[..]));
    }

    #[test]
    fn test_aac_audio_specific_config_44100_stereo() {
        let asc = FlvMuxer::build_aac_audio_specific_config(44100, 2);
        // AAC-LC (AOT=2) -> 5-bit: 00010
        // SampleRateIndex(44100)=4 -> 4-bit: 0100
        // ChannelConfig(2) -> 4-bit: 0010
        // Bits: 00010_0100_0010_000
        // Byte 0: 00010_010 = 0x12
        // Byte 1: 0_0010_000 = 0x10
        assert_eq!(asc[0], 0x12);
        assert_eq!(asc[1], 0x10);
    }

    #[test]
    fn test_aac_audio_specific_config_48000_mono() {
        let asc = FlvMuxer::build_aac_audio_specific_config(48000, 1);
        // AOT=2 -> 00010, FreqIdx(48kHz)=3 -> 0011, Chan(1) -> 0001
        // Bits: 00010_0011_0001_000
        // Byte 0: 00010_001 = 0x11
        // Byte 1: 1_0001_000 = 0x88
        assert_eq!(asc[0], 0x11);
        assert_eq!(asc[1], 0x88);
    }

    #[test]
    fn test_parse_adts_header_valid() {
        // Valid ADTS frame: sync word 0xFFF, AAC-LC, 44100Hz, Stereo
        let mut adts = vec![0xFF, 0xF1]; // MPEG-4, Layer 0, no CRC
                                         // Byte 2: profile(1=01), freq_idx(4=0100), private(0), chan_config_high(0)
                                         // = 0b01_0100_0_0 = 0x50
        adts.push(0x50);
        // Byte 3: chan_config_low(10 for channel 2), orig(0), home(0), copyright(0), start(0), frame_len_high(00)
        // = 0b10_0_0_0_0_00 = 0x80
        adts.push(0x80);
        adts.extend_from_slice(&[0x00, 0x00, 0x00]); // rest of header
        let (sr, ch) = FlvMuxer::parse_adts_header(&adts).expect("parse adts");
        assert_eq!(sr, 44100);
        assert_eq!(ch, 2);
    }

    #[test]
    fn test_parse_adts_header_invalid_sync() {
        let bad = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        assert!(FlvMuxer::parse_adts_header(&bad).is_none());
    }

    #[test]
    fn test_adts_to_flv_audio_tag_roundtrip() {
        // Construct a minimal valid ADTS header (7 bytes) + 2 bytes raw AAC
        let mut adts = vec![0xFF, 0xF1]; // MPEG-4, no CRC
        adts.push(0x50); // profile=1(AAC-LC), freq_idx=4(44100Hz), private=0, chan_high=0
        adts.push(0x80); // chan_config_low=10(stereo), orig/home/copyright=0
        adts.push(0x00);
        adts.push(0x02); // frame_length = 9 (7 header + 2 data)
        adts.push(0x00);
        adts.extend_from_slice(&[0xDE, 0x02]); // raw AAC payload

        let tag = FlvMuxer::adts_to_flv_audio_tag(&adts, 1000).expect("build audio tag");
        // FLV Tag: TagType(1B=0x08) + DataSize(3B) + Timestamp(4B) + StreamID(3B) + Payload + PrevTagSize(4B)
        assert_eq!(tag[0], 0x08); // Audio tag
        assert_eq!(tag[11], 0xAF); // AAC + 44kHz + 16bit + Stereo
        assert_eq!(tag[12], 0x01); // AACPacketType = 1 (raw)
        assert_eq!(&tag[13..tag.len() - 4], &[0xDE, 0x02]); // raw AAC data
    }

    #[test]
    fn test_flv_stream_pipeline_audio() {
        let mut pipeline = FlvStreamPipeline::new(true);
        assert!(pipeline.include_audio);

        // 模拟一个 ADTS 帧
        let mut adts = vec![0xFF, 0xF1];
        adts.push(0x50); // profile=1(AAC-LC), freq_idx=4(44100Hz), private=0, chan_high=0
        adts.push(0x80); // chan_config_low=10(stereo), orig/home/copyright=0
        adts.push(0x00);
        adts.push(0x09);
        adts.push(0x00);
        adts.extend_from_slice(&[0xAA, 0xBB, 0xCC]);

        let pkt = EncodedPacket {
            pts_ms: 1000,
            is_keyframe: false,
            codec: CodecType::Aac,
            payload: Bytes::from(adts.clone()),
            stream_tag: StreamTag::Audio,
        };

        let tags = pipeline.process_audio_packet(&pkt);
        assert_eq!(tags.len(), 1); // AAC Sequence Header
        assert_eq!(tags[0][0], 0x08); // Audio tag
        assert_eq!(tags[0][11], 0xAF);
        assert_eq!(tags[0][12], 0x00); // AACPacketType = 0 (sequence header)
        assert!(pipeline.sent_audio_header);

        // 第二帧：应发送 raw audio tag
        let pkt2 = EncodedPacket {
            pts_ms: 1040,
            is_keyframe: false,
            codec: CodecType::Aac,
            payload: Bytes::from(adts.clone()),
            stream_tag: StreamTag::Audio,
        };
        let tags2 = pipeline.process_audio_packet(&pkt2);
        assert_eq!(tags2.len(), 1);
        assert_eq!(tags2[0][12], 0x01); // AACPacketType = 1 (raw)
    }

    #[test]
    fn test_flv_stream_pipeline_audio_disabled() {
        let mut pipeline = FlvStreamPipeline::new(false);
        let pkt = EncodedPacket {
            pts_ms: 1000,
            is_keyframe: false,
            codec: CodecType::Aac,
            payload: Bytes::from_static(&[0xFF, 0xF1, 0x50, 0x00, 0x00, 0x09, 0x00, 0xAA, 0xBB]),
            stream_tag: StreamTag::Audio,
        };
        let tags = pipeline.process_audio_packet(&pkt);
        assert!(tags.is_empty()); // 音频关闭时不应产生任何 tag
    }

    #[test]
    fn test_parse_adts_header_9byte_crc() {
        // protection_absent = 0 (byte 1 bit 0 = 0 -> 9 bytes header with CRC)
        let mut adts = vec![0xFF, 0xF0]; // ID=0, layer=0, protection_absent=0
        adts.push(0x50); // profile=1, freq_idx=4 (44.1kHz), private=0, chan_high=0
        adts.push(0x80); // chan_low=2
        adts.extend_from_slice(&[0x00, 0x00, 0x00]); // bytes 4-6
        adts.extend_from_slice(&[0x12, 0x34]); // 2 bytes CRC (bytes 7-8)
        adts.extend_from_slice(&[0xAA, 0xBB]); // raw AAC data

        let (sr, ch) = FlvMuxer::parse_adts_header(&adts).expect("parse 9-byte adts header");
        assert_eq!(sr, 44100);
        assert_eq!(ch, 2);

        let tag = FlvMuxer::adts_to_flv_audio_tag(&adts, 500).expect("tag with 9-byte header");
        // Tag payload should skip 9 bytes of ADTS header and contain only raw AAC [0xAA, 0xBB]
        assert_eq!(&tag[13..tag.len() - 4], &[0xAA, 0xBB]);
    }

    #[test]
    fn test_flv_stream_pipeline_audio_video_time_sync() {
        let mut pipeline = FlvStreamPipeline::new(true);

        // 先来一个视频关键帧，确定 base_pts_ms
        let base_pts = 1_700_000_000_000i64;
        let video_pkt = EncodedPacket {
            pts_ms: base_pts,
            is_keyframe: true,
            codec: CodecType::H264,
            payload: Bytes::from_static(&[
                0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1E, // SPS
                0x00, 0x00, 0x00, 0x01, 0x68, 0xCE, // PPS
                0x00, 0x00, 0x00, 0x01, 0x65, 0x88, // IDR
            ]),
            stream_tag: StreamTag::Video,
        };
        let _ = pipeline.process_packet(&video_pkt);
        assert_eq!(pipeline.base_pts_ms, Some(base_pts));

        // 后续音频包的 DTS 必须相对于 base_pts 计算，而不是绝对大整数
        let mut adts = vec![0xFF, 0xF1, 0x50, 0x80, 0x00, 0x09, 0x00];
        adts.extend_from_slice(&[0xAA, 0xBB]);
        let audio_pkt = EncodedPacket {
            pts_ms: base_pts + 40, // +40ms
            is_keyframe: false,
            codec: CodecType::Aac,
            payload: Bytes::from(adts),
            stream_tag: StreamTag::Audio,
        };
        let audio_tags = pipeline.process_audio_packet(&audio_pkt);
        assert_eq!(audio_tags.len(), 1); // 首帧 sequence header
                                         // FLV Tag Header bytes 4..7 为 24位 timestamp + 8位 extended timestamp
        let tag = &audio_tags[0];
        let tag_dts = (tag[4] as u32) << 16 | (tag[5] as u32) << 8 | (tag[6] as u32);
        assert_eq!(tag_dts, 40); // 相对 base_pts_ms 偏移 40ms，而非 1_700_000_000_040 溢出值
    }

    #[test]
    fn test_process_packet_ignores_audio() {
        let mut pipeline = FlvStreamPipeline::new(true);
        let audio_pkt = EncodedPacket {
            pts_ms: 1000,
            is_keyframe: false,
            codec: CodecType::Aac,
            payload: Bytes::from_static(b"fake audio data"),
            stream_tag: StreamTag::Audio,
        };
        // 传入音频包绝不应 panic，必须安全返回空
        let tags = pipeline.process_packet(&audio_pkt);
        assert!(tags.is_empty());
    }

    #[test]
    fn test_parse_adts_header_various_frequencies_and_channels() {
        // Profile 1 (AAC-LC, 0b01)
        // 48000 Hz: freq_idx = 3 (0b0011) -> byte 2: (0b01 << 6) | (0b0011 << 2) = 0x40 | 0x0C = 0x4C
        // Mono: channel 1 -> byte 2 bit 0 = 0, byte 3 bits 7..6 = 0b01 -> byte 3 = 0x40
        let adts_48k_mono = [0xFF, 0xF1, 0x4C, 0x40, 0x00, 0x09, 0x00];
        let (sr, ch) = FlvMuxer::parse_adts_header(&adts_48k_mono).expect("parse 48k mono");
        assert_eq!(sr, 48000);
        assert_eq!(ch, 1);

        // 16000 Hz: freq_idx = 8 (0b1000) -> byte 2: (0b01 << 6) | (0b1000 << 2) = 0x40 | 0x20 = 0x60
        // Stereo: channel 2 -> byte 2 bit 0 = 0, byte 3 bits 7..6 = 0b10 -> byte 3 = 0x80
        let adts_16k_stereo = [0xFF, 0xF1, 0x60, 0x80, 0x00, 0x09, 0x00];
        let (sr, ch) = FlvMuxer::parse_adts_header(&adts_16k_stereo).expect("parse 16k stereo");
        assert_eq!(sr, 16000);
        assert_eq!(ch, 2);
    }

    #[test]
    fn test_handle_lagged_resets_audio_state() {
        let mut pipeline = FlvStreamPipeline::new(true);
        let mut adts = vec![0xFF, 0xF1, 0x50, 0x80, 0x00, 0x09, 0x00];
        adts.extend_from_slice(&[0xAA, 0xBB]);
        let pkt = EncodedPacket {
            pts_ms: 1000,
            is_keyframe: false,
            codec: CodecType::Aac,
            payload: Bytes::from(adts.clone()),
            stream_tag: StreamTag::Audio,
        };

        let tags = pipeline.process_audio_packet(&pkt);
        assert_eq!(tags.len(), 1);
        assert!(pipeline.sent_audio_header);

        pipeline.handle_lagged();
        assert!(!pipeline.sent_audio_header);
        assert_eq!(pipeline.last_audio_pts, 0);

        // 重置后下一帧应重新生成 AAC sequence header
        let tags2 = pipeline.process_audio_packet(&pkt);
        assert_eq!(tags2.len(), 1);
        assert_eq!(tags2[0][12], 0x00); // AAC Sequence Header
    }
}
