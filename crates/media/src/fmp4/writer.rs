//! fMP4 写入器状态机
//!
//! `FMP4Writer` 接收 `EncodedPacket` 流，自动完成：
//! 1. 首个 keyframe → 提取 SPS/PPS/VPS → 写 `ftyp` + `moov` 初始化段
//! 2. 后续每个 GOP（keyframe 到下一个 keyframe 之间）→ 写一个 `moof` + `mdat` fragment
//! 3. `finish()` → flush 最后一个未闭合的 fragment

use std::io::{self, Write};
use std::sync::Arc;

use types::{CodecType, EncodedPacket};

use super::boxes::{self, SampleEntry, VIDEO_TIMESCALE};
use super::nalu::{annex_b_to_avcc, extract_parameter_sets, ParameterSets};
use crate::sps::{parse_h264_sps, parse_h265_sps};

/// Fragment 写入后返回的统计信息
#[derive(Debug, Clone)]
pub struct FragmentInfo {
    pub sequence_number: u32,
    pub sample_count: usize,
    pub total_bytes: usize,
    pub duration_ms: u64,
}

/// 当前 GOP 正在积累的 sample 数据
struct PendingFragment {
    samples: Vec<SampleEntry>,
    mdat_payload: Vec<u8>,
    base_pts_ms: i64,
    last_pts_ms: i64,
}

impl PendingFragment {
    fn new(base_pts_ms: i64) -> Self {
        Self {
            samples: Vec::with_capacity(64),
            mdat_payload: Vec::with_capacity(128 * 1024), // 128KB 初始容量
            base_pts_ms,
            last_pts_ms: base_pts_ms,
        }
    }
}

/// fMP4 写入器
///
/// 泛型 `W` 为底层 IO writer（通常是 `BufWriter<File>`）。
pub struct FMP4Writer<W: Write> {
    writer: W,
    codec: CodecType,
    sequence_number: u32,
    initialized: bool,
    params: Option<ParameterSets>,
    pending: Option<PendingFragment>,
    /// 文件起始 PTS（用于计算 base_decode_time）
    origin_pts_ms: Option<i64>,
    /// 已写入的总字节数
    bytes_written: u64,
    /// 已写入的 fragment 数
    fragments_written: u32,
}

impl<W: Write> std::fmt::Debug for FMP4Writer<W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FMP4Writer")
            .field("codec", &self.codec)
            .field("sequence_number", &self.sequence_number)
            .field("initialized", &self.initialized)
            .field("bytes_written", &self.bytes_written)
            .field("fragments_written", &self.fragments_written)
            .finish_non_exhaustive()
    }
}

impl<W: Write> FMP4Writer<W> {
    /// 创建一个新的 fMP4 写入器。
    ///
    /// `codec` 必须在创建时确定（H264 或 H265），不可中途切换。
    pub fn new(writer: W, codec: CodecType) -> Self {
        Self {
            writer,
            codec,
            sequence_number: 0,
            initialized: false,
            params: None,
            pending: None,
            origin_pts_ms: None,
            bytes_written: 0,
            fragments_written: 0,
        }
    }

    /// 向写入器推送一个压缩包。
    ///
    /// 自动处理初始化段写入和 GOP fragment 切分。
    pub fn push(&mut self, packet: &Arc<EncodedPacket>) -> io::Result<Option<FragmentInfo>> {
        // 过滤非视频包
        if !packet.codec.is_video() || packet.codec != self.codec {
            return Ok(None);
        }

        // 如果尚未初始化，等待首个 keyframe
        if !self.initialized {
            if !packet.is_keyframe {
                return Ok(None); // 丢弃 keyframe 之前的 P/B 帧
            }

            // 提取参数集
            let ps = extract_parameter_sets(&packet.payload, self.codec);
            if !ps.is_complete(self.codec) {
                return Ok(None); // 参数集不完整，继续等待
            }

            self.write_init_segment(&ps)?;
            self.params = Some(ps);
            self.initialized = true;
            self.origin_pts_ms = Some(packet.pts_ms);
        }

        let mut flushed_info = None;

        // 新 keyframe → flush 上一个 pending fragment，开始新 fragment
        if packet.is_keyframe {
            if let Some(pending) = self.pending.take() {
                flushed_info = Some(self.flush_fragment(pending)?);
            }
            self.pending = Some(PendingFragment::new(packet.pts_ms));
        }

        // 如果 pending 不存在（不该发生，因为上面 keyframe 会创建），跳过
        let Some(pending) = self.pending.as_mut() else {
            return Ok(flushed_info);
        };

        // 将 Annex-B 转为 AVCC 格式
        let avcc_data = annex_b_to_avcc(&packet.payload, self.codec);
        if avcc_data.is_empty() {
            return Ok(flushed_info);
        }

        // 计算 sample duration（和上一个 sample 的 PTS 差值，转换到 timescale）
        let duration_ms = if pending.samples.is_empty() {
            // 第一个 sample，duration 暂时用 0，后续在 flush 时修正
            0
        } else {
            (packet.pts_ms - pending.last_pts_ms).max(0)
        };
        let duration_ts = ms_to_timescale(duration_ms);

        // 修正前一个 sample 的 duration（如果它还是 0）
        if pending.samples.len() == 1 && pending.samples[0].duration == 0 && duration_ts > 0 {
            pending.samples[0].duration = duration_ts;
        }

        pending.samples.push(SampleEntry {
            duration: duration_ts,
            size: avcc_data.len() as u32,
            is_keyframe: packet.is_keyframe,
        });

        pending.mdat_payload.extend_from_slice(&avcc_data);
        pending.last_pts_ms = packet.pts_ms;

        Ok(flushed_info)
    }

    /// 关闭写入器：flush 最后一个 pending fragment。
    ///
    /// 返回最终的统计信息。调用后 writer 被消费。
    pub fn finish(mut self) -> io::Result<FinishInfo> {
        let mut last_fragment = None;
        if let Some(pending) = self.pending.take() {
            if !pending.samples.is_empty() {
                last_fragment = Some(self.flush_fragment(pending)?);
            }
        }
        self.writer.flush()?;

        Ok(FinishInfo {
            bytes_written: self.bytes_written,
            fragments_written: self.fragments_written,
            last_fragment,
        })
    }

    /// 写入初始化段（ftyp + moov）。
    fn write_init_segment(&mut self, ps: &ParameterSets) -> io::Result<()> {
        // 从 SPS 解析宽高
        let (width, height) = self.parse_dimensions(ps)?;

        // 构建 codec private data
        let codec_private = match self.codec {
            CodecType::H264 => ps.build_avcc(),
            CodecType::H265 => ps.build_hvcc(),
            _ => None,
        }
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "failed to build codec config"))?;

        let mut buf = Vec::with_capacity(512);
        boxes::write_ftyp(&mut buf)?;
        boxes::write_moov(&mut buf, self.codec, width as u16, height as u16, &codec_private)?;

        self.writer.write_all(&buf)?;
        self.bytes_written += buf.len() as u64;

        Ok(())
    }

    /// flush 一个完整的 pending fragment 到文件。
    fn flush_fragment(&mut self, mut pending: PendingFragment) -> io::Result<FragmentInfo> {
        // 修正最后一个 sample 的 duration（使用前一个 sample 的 duration 兜底）
        if pending.samples.last().is_some_and(|s| s.duration == 0) {
            let fallback = pending
                .samples
                .iter()
                .rev()
                .skip(1)
                .find(|s| s.duration > 0)
                .map(|s| s.duration)
                .unwrap_or(ms_to_timescale(33));
            if let Some(last) = pending.samples.last_mut() {
                last.duration = fallback;
            }
        }

        self.sequence_number += 1;

        let origin = self.origin_pts_ms.unwrap_or(0);
        let base_decode_time = ms_to_timescale_u64((pending.base_pts_ms - origin).max(0));

        let sample_count = pending.samples.len();
        let duration_ms = if sample_count > 0 {
            (pending.last_pts_ms - pending.base_pts_ms).max(0) as u64
        } else {
            0
        };

        let mut buf = Vec::with_capacity(pending.mdat_payload.len() + 256);
        boxes::write_fragment(
            &mut buf,
            self.sequence_number,
            base_decode_time,
            &pending.samples,
            &pending.mdat_payload,
        )?;

        let total_bytes = buf.len();
        self.writer.write_all(&buf)?;
        self.bytes_written += total_bytes as u64;
        self.fragments_written += 1;

        Ok(FragmentInfo {
            sequence_number: self.sequence_number,
            sample_count,
            total_bytes,
            duration_ms,
        })
    }

    /// 从 SPS 解析视频宽高
    fn parse_dimensions(&self, ps: &ParameterSets) -> io::Result<(u32, u32)> {
        let sps = ps
            .sps
            .as_ref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing SPS"))?;

        let info = match self.codec {
            CodecType::H264 => parse_h264_sps(sps),
            CodecType::H265 => parse_h265_sps(sps),
            _ => Err(crate::error::MediaError::SpsParse("unsupported codec".into())),
        }
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

        Ok((info.width, info.height))
    }

    /// 获取已写入的总字节数
    pub fn bytes_written(&self) -> u64 {
        self.bytes_written
    }

    /// 获取已写入的 fragment 数
    pub fn fragments_written(&self) -> u32 {
        self.fragments_written
    }

    /// 是否已完成初始化（已写入 ftyp + moov）
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }
}

/// `finish()` 返回的最终统计
#[derive(Debug)]
pub struct FinishInfo {
    pub bytes_written: u64,
    pub fragments_written: u32,
    pub last_fragment: Option<FragmentInfo>,
}

/// 毫秒 → timescale (90kHz)
#[inline]
fn ms_to_timescale(ms: i64) -> u32 {
    ((ms.max(0) as u64 * VIDEO_TIMESCALE as u64) / 1000) as u32
}

/// 毫秒 → timescale (90kHz)，u64 版本
#[inline]
fn ms_to_timescale_u64(ms: i64) -> u64 {
    (ms.max(0) as u64 * VIDEO_TIMESCALE as u64) / 1000
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use bytes::Bytes;

    /// 构造带真实 SPS/PPS 的 keyframe payload (Annex-B)
    fn make_h264_keyframe(pts_ms: i64) -> Arc<EncodedPacket> {
        // SPS: 0x67 0x64 0x00 0x29 (High Profile, Level 4.1) — minimal for 1920x1080
        // 来自项目 sps.rs 测试中已验证的 1080P SPS
        let sps = [
            0x67, 0x64, 0x00, 0x29, 0xac, 0x72, 0x84, 0x40, 0x78, 0x02, 0x27, 0xe5, 0xc0, 0x44,
            0x00, 0x00, 0x03, 0x00, 0x04, 0x00, 0x00, 0x03, 0x00, 0xf0, 0x3c, 0x60, 0xc6, 0x58,
        ];
        let pps = [0x68, 0xEE, 0x3C, 0x80];
        let idr_body = [0x65, 0x88, 0x84, 0x00, 0x33, 0xFF]; // fake IDR slice

        let mut payload = Vec::new();
        payload.extend_from_slice(b"\x00\x00\x00\x01");
        payload.extend_from_slice(&sps);
        payload.extend_from_slice(b"\x00\x00\x00\x01");
        payload.extend_from_slice(&pps);
        payload.extend_from_slice(b"\x00\x00\x00\x01");
        payload.extend_from_slice(&idr_body);

        Arc::new(EncodedPacket {
            pts_ms,
            is_keyframe: true,
            codec: CodecType::H264,
            payload: Bytes::from(payload),
            stream_tag: types::StreamTag::Video,
        })
    }

    fn make_h264_pframe(pts_ms: i64) -> Arc<EncodedPacket> {
        let mut payload = Vec::new();
        payload.extend_from_slice(b"\x00\x00\x00\x01");
        payload.extend_from_slice(&[0x41, 0x9A, 0x00, 0x33, 0xFF]); // fake P-slice

        Arc::new(EncodedPacket {
            pts_ms,
            is_keyframe: false,
            codec: CodecType::H264,
            payload: Bytes::from(payload),
            stream_tag: types::StreamTag::Video,
        })
    }

    #[test]
    fn test_writer_produces_valid_fmp4() {
        let mut output = Vec::new();
        let mut writer = FMP4Writer::new(&mut output, CodecType::H264);

        // GOP 1: keyframe + 3 P-frames
        writer.push(&make_h264_keyframe(1000)).unwrap();
        writer.push(&make_h264_pframe(1033)).unwrap();
        writer.push(&make_h264_pframe(1066)).unwrap();
        writer.push(&make_h264_pframe(1100)).unwrap();

        // GOP 2: new keyframe (triggers flush of GOP 1)
        let frag_info = writer.push(&make_h264_keyframe(2000)).unwrap();
        assert!(frag_info.is_some(), "GOP 1 应在 GOP 2 首帧时被 flush");
        let frag = frag_info.unwrap();
        assert_eq!(frag.sequence_number, 1);
        assert_eq!(frag.sample_count, 4);

        // finish flushes GOP 2
        let finish = writer.finish().unwrap();
        assert_eq!(finish.fragments_written, 2);
        assert!(finish.bytes_written > 0);
        assert!(finish.last_fragment.is_some());

        // 验证 output 包含正确的 box 结构
        assert_eq!(&output[4..8], b"ftyp");
        assert!(output.windows(4).any(|w| w == b"moov"));
        assert!(output.windows(4).any(|w| w == b"moof"));
        assert!(output.windows(4).any(|w| w == b"mdat"));
    }

    #[test]
    fn test_writer_skips_pframes_before_first_keyframe() {
        let mut output = Vec::new();
        let mut writer = FMP4Writer::new(&mut output, CodecType::H264);

        // P-frames before any keyframe should be silently skipped
        assert!(writer.push(&make_h264_pframe(100)).unwrap().is_none());
        assert!(writer.push(&make_h264_pframe(133)).unwrap().is_none());
        assert!(!writer.is_initialized());

        // Keyframe should trigger initialization
        writer.push(&make_h264_keyframe(200)).unwrap();
        assert!(writer.is_initialized());

        writer.finish().unwrap();
        assert!(!output.is_empty());
    }

    #[test]
    fn test_writer_single_gop() {
        let mut output = Vec::new();
        let mut writer = FMP4Writer::new(&mut output, CodecType::H264);

        writer.push(&make_h264_keyframe(0)).unwrap();
        writer.push(&make_h264_pframe(33)).unwrap();
        writer.push(&make_h264_pframe(66)).unwrap();

        // No second keyframe, so nothing flushed yet
        let finish = writer.finish().unwrap();
        assert_eq!(finish.fragments_written, 1); // finish flushes the pending fragment
        assert!(finish.last_fragment.is_some());
        assert_eq!(finish.last_fragment.unwrap().sample_count, 3);
    }

    #[test]
    fn test_writer_filters_audio_packets() {
        let mut output = Vec::new();
        let mut writer = FMP4Writer::new(&mut output, CodecType::H264);

        let audio_pkt = Arc::new(EncodedPacket {
            pts_ms: 0,
            is_keyframe: false,
            codec: CodecType::Aac,
            payload: Bytes::from_static(b"\xFF\xF1audio"),
            stream_tag: types::StreamTag::Audio,
        });

        writer.push(&make_h264_keyframe(0)).unwrap();
        assert!(writer.push(&audio_pkt).unwrap().is_none());
        writer.finish().unwrap();
    }
}
