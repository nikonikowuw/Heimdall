//! MPEG-PS (Program Stream) 容错流式解复用引擎
//!
//! 适配国标 GB/T 28181-2016 规范中通过 RTP/PS 传输的音视频码流：
//! 1. 流式解析 Pack Header (0xBA)、System Header (0xBB)、PSM (0xBC)、PES (0xE0..=0xEF, 0xC0..=0xDF)；
//! 2. 支持无 PSM 时的启发式编码探测 (Heuristic Codec Sniffing)；
//! 3. 33 位 90kHz PTS 回环展开 (PTS Unwrapping) 转换为平滑递增的 13 位毫秒时标；
//! 4. 切分 Annex-B NALU，抽取 SPS/PPS/VPS 参数集并标记关键帧；
//! 5. 严格隔离音频 PES 包，避免非视频数据冲撞硬件硬解管线。

use bytes::{Buf, Bytes, BytesMut};
use types::{CodecType, EncodedPacket, StreamTag};

use crate::sps::{is_keyframe_or_parameter_set, split_annex_b_nalus};

/// 单帧 ES 缓冲最大安全阈值（10 MB，防止非标安防 IPC 畸形包造成 OOM）
pub const MAX_ES_BUFFER_BYTES: usize = 10 * 1024 * 1024;

/// 33 位 PTS 回环上限 (2^33 ticks, 90kHz 约为 8,589,934,592 刻度 / 约 26.5 小时)
pub const PTS_33BIT_MAX_TICKS: i64 = 1i64 << 33;

/// 判定发生回环的时间戳阈值 (若连续两次差值反向超过 2^32，判定为自然回环)
pub const PTS_ROLLOVER_THRESHOLD: i64 = 1i64 << 32;

/// MPEG-PS 33-bit 90kHz 时间戳回环展开器
#[derive(Debug, Clone)]
pub struct PtsUnwrapper {
    last_raw_ticks: Option<i64>,
    rollover_count: i64,
    last_pts_ms: i64,
}

impl Default for PtsUnwrapper {
    fn default() -> Self {
        Self::new()
    }
}

impl PtsUnwrapper {
    pub fn new() -> Self {
        Self {
            last_raw_ticks: None,
            rollover_count: 0,
            last_pts_ms: 0,
        }
    }

    /// 将 33 位的 90kHz PTS 原始刻度转换为单调非递减的毫秒时间戳
    pub fn unwrap(&mut self, raw_ticks: i64) -> i64 {
        let raw_33 = raw_ticks & (PTS_33BIT_MAX_TICKS - 1);

        if let Some(prev) = self.last_raw_ticks {
            // 当 raw 比 prev 小超过半程时，判定发生正向回环
            if prev - raw_33 > PTS_ROLLOVER_THRESHOLD {
                self.rollover_count += 1;
            } else if raw_33 - prev > PTS_ROLLOVER_THRESHOLD && self.rollover_count > 0 {
                // 异常回跳容错
                self.rollover_count -= 1;
            }
        }

        self.last_raw_ticks = Some(raw_33);
        let linear_ticks = raw_33 + self.rollover_count * PTS_33BIT_MAX_TICKS;
        // 90kHz to ms: (ticks * 1000) / 90000 = ticks / 90
        let calculated_ms = linear_ticks / 90;

        // 保持平滑非递减
        let pts_ms = calculated_ms.max(self.last_pts_ms);
        self.last_pts_ms = pts_ms;
        pts_ms
    }

    /// 重置回环计数器
    pub fn reset(&mut self) {
        self.last_raw_ticks = None;
        self.rollover_count = 0;
        self.last_pts_ms = 0;
    }
}

/// MPEG-PS 流式解复用器
#[derive(Debug)]
pub struct PsDemuxer {
    /// 缓存的未完成组装的字节流
    buffer: BytesMut,
    /// 当前已确定的视频编码（若未知，则由启发式嗅探器探测）
    detected_codec: Option<CodecType>,
    /// PTS 解回环器
    pts_unwrapper: PtsUnwrapper,
    /// 当前视频 PES 包累积的 ES 数据
    current_video_es: BytesMut,
    /// 当前视频帧的 PTS (毫秒)
    current_video_pts_ms: i64,
}

impl Default for PsDemuxer {
    fn default() -> Self {
        Self::new()
    }
}

impl PsDemuxer {
    pub fn new() -> Self {
        Self {
            buffer: BytesMut::with_capacity(64 * 1024),
            detected_codec: None,
            pts_unwrapper: PtsUnwrapper::new(),
            current_video_es: BytesMut::with_capacity(32 * 1024),
            current_video_pts_ms: 0,
        }
    }

    /// 设置预先知道的编码格式（如由 SDP rtpmap 给出）
    pub fn set_codec(&mut self, codec: CodecType) {
        self.detected_codec = Some(codec);
    }

    /// 重置解复用器内部状态（源流切换或丢包重连时调用）
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.current_video_es.clear();
        self.current_video_pts_ms = 0;
        self.pts_unwrapper.reset();
    }

    /// 发生丢包断流时重置未完成帧，防止拼接出损坏的宏块
    pub fn reset_on_discontinuity(&mut self) {
        self.current_video_es.clear();
    }

    /// 输入一段 MPEG-PS 二进制数据，解复用输出抽取出的 EncodedPacket 序列
    pub fn demux(&mut self, data: &[u8]) -> Vec<EncodedPacket> {
        self.buffer.extend_from_slice(data);
        let mut packets = Vec::new();

        while self.buffer.len() >= 4 {
            // 1. 扫描 0x000001 起始码 (Start Code)
            let start_pos = match find_start_code(&self.buffer) {
                Some(pos) => pos,
                None => {
                    // 没有起始码，保留最后 3 字节以备跨包拼接，其余丢弃
                    if self.buffer.len() > 3 {
                        let discard = self.buffer.len() - 3;
                        self.buffer.advance(discard);
                    }
                    break;
                }
            };

            // 滑动跳过起始码之前的无效数据
            if start_pos > 0 {
                self.buffer.advance(start_pos);
            }

            if self.buffer.len() < 4 {
                break;
            }

            let stream_id = self.buffer[3];

            match stream_id {
                // Pack Header: 0x000001BA
                0xBA => {
                    if self.buffer.len() < 14 {
                        break; // 头部不完整，等待更多数据
                    }
                    // MPEG-2 pack header 长度至少 14 字节，最后包含 3 位 stuffing length
                    let stuffing_len = (self.buffer[13] & 0x07) as usize;
                    let total_header_len = 14 + stuffing_len;
                    if self.buffer.len() < total_header_len {
                        break;
                    }

                    // 遇到新的 Pack Header 时，若有残留的完整视频帧，先刷出
                    if let Some(pkt) = self.flush_current_video_frame() {
                        packets.push(pkt);
                    }

                    self.buffer.advance(total_header_len);
                }

                // System Header: 0x000001BB
                0xBB => {
                    if self.buffer.len() < 6 {
                        break;
                    }
                    let header_len = u16::from_be_bytes([self.buffer[4], self.buffer[5]]) as usize;
                    let total_len = 6 + header_len;
                    if self.buffer.len() < total_len {
                        break;
                    }
                    self.buffer.advance(total_len);
                }

                // Program Stream Map (PSM): 0x000001BC
                0xBC => {
                    if self.buffer.len() < 6 {
                        break;
                    }
                    let psm_len = u16::from_be_bytes([self.buffer[4], self.buffer[5]]) as usize;
                    let total_len = 6 + psm_len;
                    if self.buffer.len() < total_len {
                        break;
                    }
                    let chunk = self.buffer.split_to(total_len);
                    self.parse_psm(&chunk);
                }

                // Video PES packet: 0x000001E0 ..= 0x000001EF
                0xE0..=0xEF => {
                    if self.buffer.len() < 6 {
                        break;
                    }
                    let pes_len = u16::from_be_bytes([self.buffer[4], self.buffer[5]]) as usize;

                    // pes_len == 0 表示无界视频流（通常直到下一个起始码）
                    if pes_len == 0 {
                        // 在后续缓冲区中寻找下一个起始码
                        if let Some(next_start) = find_start_code(&self.buffer[4..]) {
                            let chunk_len = 4 + next_start;
                            let chunk = self.buffer.split_to(chunk_len);
                            if let Some(pkt) = self.parse_video_pes(&chunk) {
                                packets.push(pkt);
                            }
                            continue;
                        } else {
                            // 等待更多数据
                            break;
                        }
                    }

                    let total_len = 6 + pes_len;
                    if self.buffer.len() < total_len {
                        break;
                    }

                    let chunk = self.buffer.split_to(total_len);
                    if let Some(pkt) = self.parse_video_pes(&chunk) {
                        packets.push(pkt);
                    }
                }

                // Audio PES packet: 0x000001C0 ..= 0x000001DF
                0xC0..=0xDF => {
                    if self.buffer.len() < 6 {
                        break;
                    }
                    let pes_len = u16::from_be_bytes([self.buffer[4], self.buffer[5]]) as usize;
                    let total_len = 6 + pes_len;
                    if self.buffer.len() < total_len {
                        break;
                    }
                    let chunk = self.buffer.split_to(total_len);
                    if let Some(pkt) = self.parse_audio_pes(&chunk) {
                        packets.push(pkt);
                    }
                }

                // 其他控制或填充流 (0xBD Private, 0xBE Padding)
                _ => {
                    if self.buffer.len() < 6 {
                        break;
                    }
                    let payload_len = u16::from_be_bytes([self.buffer[4], self.buffer[5]]) as usize;
                    let total_len = 6 + payload_len;
                    if self.buffer.len() < total_len {
                        // 若 payload_len 畸变过大（大于 64KB），则跳过起始码单字节滑动
                        if payload_len > 65535 {
                            self.buffer.advance(4);
                        }
                        break;
                    }
                    self.buffer.advance(total_len);
                }
            }
        }

        packets
    }

    /// 强制刷新当前未输出的视频帧
    pub fn flush(&mut self) -> Option<EncodedPacket> {
        self.flush_current_video_frame()
    }

    /// 解析 PSM 获取音视频编码映射
    fn parse_psm(&mut self, psm_bytes: &[u8]) {
        if psm_bytes.len() < 10 {
            return;
        }
        // PSM 结构：00 00 01 BC | psm_len (2B) | flags (2B) | program_stream_info_length (2B)
        let prog_info_len = u16::from_be_bytes([psm_bytes[8], psm_bytes[9]]) as usize;
        let es_map_offset = 10 + prog_info_len;
        if psm_bytes.len() < es_map_offset + 2 {
            return;
        }

        let es_map_len =
            u16::from_be_bytes([psm_bytes[es_map_offset], psm_bytes[es_map_offset + 1]]) as usize;
        let mut curr = es_map_offset + 2;
        let end = (curr + es_map_len).min(psm_bytes.len());

        while curr + 4 <= end {
            let stream_type = psm_bytes[curr];
            let elementary_stream_id = psm_bytes[curr + 1];
            let es_info_len =
                u16::from_be_bytes([psm_bytes[curr + 2], psm_bytes[curr + 3]]) as usize;

            if (0xE0..=0xEF).contains(&elementary_stream_id) {
                match stream_type {
                    0x1B => self.detected_codec = Some(CodecType::H264),
                    0x24 => self.detected_codec = Some(CodecType::H265),
                    _ => {}
                }
            }
            curr += 4 + es_info_len;
        }
    }

    /// 解析视频 PES 包，若遇到新 PTS 则刷出上一完整帧
    fn parse_video_pes(&mut self, pes_data: &[u8]) -> Option<EncodedPacket> {
        if pes_data.len() < 9 {
            return None;
        }

        // PES 头部标志判断
        let flags2 = pes_data[7];
        let pes_header_data_len = pes_data[8] as usize;
        let payload_offset = 9 + pes_header_data_len;

        if pes_data.len() < payload_offset {
            return None;
        }

        let mut flushed_packet = None;

        // 提取 PTS: flags2 的高 2 位表示 PTS/DTS 存在标识 (0x80 表示仅 PTS, 0xC0 表示 PTS 与 DTS)
        let has_pts = (flags2 & 0x80) != 0;
        if has_pts && pes_data.len() >= 14 && pes_header_data_len >= 5 {
            let b0 = pes_data[9] as i64;
            let b1 = pes_data[10] as i64;
            let b2 = pes_data[11] as i64;
            let b3 = pes_data[12] as i64;
            let b4 = pes_data[13] as i64;

            let pts_ticks = ((b0 & 0x0E) << 29)
                | ((b1 & 0xFF) << 22)
                | ((b2 & 0xFE) << 14)
                | ((b3 & 0xFF) << 7)
                | ((b4 & 0xFE) >> 1);

            let unwrapped_pts = self.pts_unwrapper.unwrap(pts_ticks);

            // 若当前帧已有积累的 ES，且遇到了新的 PTS，说明进入了新的一帧，立即刷出上一帧
            if !self.current_video_es.is_empty() && unwrapped_pts != self.current_video_pts_ms {
                flushed_packet = self.flush_current_video_frame();
            }
            self.current_video_pts_ms = unwrapped_pts;
        }

        let es_payload = &pes_data[payload_offset..];
        if self.current_video_es.len() + es_payload.len() <= MAX_ES_BUFFER_BYTES {
            self.current_video_es.extend_from_slice(es_payload);
        } else {
            // 超出安全上限，清空防止 OOM
            self.current_video_es.clear();
        }

        flushed_packet
    }

    /// 解析音频 PES 包
    fn parse_audio_pes(&mut self, pes_data: &[u8]) -> Option<EncodedPacket> {
        if pes_data.len() < 9 {
            return None;
        }

        let flags2 = pes_data[7];
        let pes_header_data_len = pes_data[8] as usize;
        let payload_offset = 9 + pes_header_data_len;

        if pes_data.len() <= payload_offset {
            return None;
        }

        let mut pts_ms = self.current_video_pts_ms;
        if (flags2 & 0x80) != 0 && pes_data.len() >= 14 {
            let b0 = pes_data[9] as i64;
            let b1 = pes_data[10] as i64;
            let b2 = pes_data[11] as i64;
            let b3 = pes_data[12] as i64;
            let b4 = pes_data[13] as i64;

            let pts_ticks = ((b0 & 0x0E) << 29)
                | ((b1 & 0xFF) << 22)
                | ((b2 & 0xFE) << 14)
                | ((b3 & 0xFF) << 7)
                | ((b4 & 0xFE) >> 1);

            pts_ms = (pts_ticks & (PTS_33BIT_MAX_TICKS - 1)) / 90;
        }

        let audio_payload = Bytes::copy_from_slice(&pes_data[payload_offset..]);

        Some(EncodedPacket {
            pts_ms,
            is_keyframe: false,
            codec: CodecType::H264, // 音频流复用默认占位，由 stream_tag 区分
            stream_tag: StreamTag::Audio,
            payload: audio_payload,
        })
    }

    /// 将累积的视频 ES 打包为标准 Annex-B 的 EncodedPacket
    fn flush_current_video_frame(&mut self) -> Option<EncodedPacket> {
        if self.current_video_es.is_empty() {
            return None;
        }

        let es_data = self.current_video_es.split().freeze();

        // 启发式嗅探视频编码（若 PSM 缺失）
        if self.detected_codec.is_none() {
            self.detected_codec = sniff_video_codec(&es_data);
        }
        let codec = self.detected_codec.unwrap_or(CodecType::H264);

        // 判定是否包含关键帧 / 参数集
        let is_keyframe = is_keyframe_or_parameter_set(&es_data, codec);

        Some(EncodedPacket {
            pts_ms: self.current_video_pts_ms,
            is_keyframe,
            codec,
            stream_tag: StreamTag::Video,
            payload: es_data,
        })
    }
}

/// 在字节流中寻找 0x000001 起始码
fn find_start_code(data: &[u8]) -> Option<usize> {
    if data.len() < 3 {
        return None;
    }
    (0..=(data.len() - 3)).find(|&i| data[i] == 0x00 && data[i + 1] == 0x00 && data[i + 2] == 0x01)
}

/// 启发式特征嗅探视频编码格式 (H.264 vs H.265)
pub fn sniff_video_codec(es_bytes: &[u8]) -> Option<CodecType> {
    let nalus = split_annex_b_nalus(es_bytes);
    for nalu in nalus {
        if nalu.is_empty() {
            continue;
        }
        let first_byte = nalu[0];

        // 检查 H.264 特征: NAL Type = first_byte & 0x1F
        // 7 = SPS, 8 = PPS, 5 = IDR
        let h264_type = first_byte & 0x1F;
        if h264_type == 7 || h264_type == 8 || h264_type == 5 {
            return Some(CodecType::H264);
        }

        // 检查 H.265 特征: NAL Type = (first_byte >> 1) & 0x3F
        // 32 = VPS, 33 = SPS, 34 = PPS, 19..=20 = IDR
        let h265_type = (first_byte >> 1) & 0x3F;
        if h265_type == 32 || h265_type == 33 || h265_type == 34 || (19..=20).contains(&h265_type) {
            return Some(CodecType::H265);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pts_unwrapper_linear_and_rollover() {
        let mut unwrapper = PtsUnwrapper::new();

        // 正常线性递增：90000 ticks = 1000 ms
        assert_eq!(unwrapper.unwrap(0), 0);
        assert_eq!(unwrapper.unwrap(90_000), 1000);
        assert_eq!(unwrapper.unwrap(180_000), 2000);

        // 模拟接近 33-bit 上限 (PTS_33BIT_MAX_TICKS = 8_589_934_592)
        let near_max = PTS_33BIT_MAX_TICKS - 90_000;
        let pts_near_max = unwrapper.unwrap(near_max);

        // 模拟回环：时标翻转归零 (90_000 ticks)
        let after_rollover = unwrapper.unwrap(90_000);
        assert!(
            after_rollover > pts_near_max,
            "回环后展开时间戳必须单调递增"
        );
    }

    #[test]
    fn test_sniff_h264_and_h265() {
        // H.264 SPS: 00 00 00 01 67 42 00 1f ...
        let h264_frame = [0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1F];
        assert_eq!(sniff_video_codec(&h264_frame), Some(CodecType::H264));

        // H.265 VPS: 00 00 00 01 40 01 0c ...
        let h265_frame = [0x00, 0x00, 0x00, 0x01, 0x40, 0x01, 0x0C];
        assert_eq!(sniff_video_codec(&h265_frame), Some(CodecType::H265));
    }

    #[test]
    fn test_ps_demuxer_pack_and_pes() {
        let mut demuxer = PsDemuxer::new();

        // 构建一个模拟的 MPEG-PS 数据包流
        // 1. Pack Header (14 字节)
        let mut data = vec![
            0x00, 0x00, 0x01, 0xBA, // start code
            0x44, 0x00, 0x04, 0x00, 0x04, 0x01, 0x01, 0x89, 0xC3, 0xF8, // SCR + stuffing (0)
        ];

        // 2. Video PES Packet: 0x000001E0, 长度 = 3 (头部) + 5 (PTS) + 8 (NALU) = 16 字节
        let pes_payload = [0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1F]; // H.264 SPS
        let pes_len = (3 + 5 + pes_payload.len()) as u16;

        data.extend_from_slice(&[0x00, 0x00, 0x01, 0xE0]);
        data.extend_from_slice(&pes_len.to_be_bytes());
        data.push(0x80); // flags 1
        data.push(0x80); // flags 2: PTS present
        data.push(0x05); // header data len = 5

        // PTS = 90000 ticks (1000ms):
        let pts_bytes = [0x21, 0x00, 0x05, 0xBF, 0x21];
        data.extend_from_slice(&pts_bytes);
        data.extend_from_slice(&pes_payload);

        let _packets = demuxer.demux(&data);
        // Pack 尚未遇到下一个 Header，强制 flush
        let flushed = demuxer.flush().expect("flushed packet");

        assert_eq!(flushed.stream_tag, StreamTag::Video);
        assert_eq!(flushed.codec, CodecType::H264);
        assert!(flushed.is_keyframe);
        assert_eq!(flushed.pts_ms, 1000);
    }
}
