//! Annex-B ↔ AVCC NALU 转换 与 SPS/PPS/VPS 提取
//!
//! fMP4 的 `mdat` 存储 AVCC 格式（4 字节 length prefix），
//! 而 RTSP/摄像头输出的是 Annex-B 格式（startcode prefix）。

use bytes::Bytes;
use types::CodecType;

use crate::sps::split_annex_b_nalus;

/// 从 Annex-B keyframe payload 中提取参数集。
///
/// 返回 `(sps, pps, vps)` 裸 NALU bytes（无 startcode）。
/// H.264 无 VPS，返回 `None`。
pub fn extract_parameter_sets(data: &[u8], codec: CodecType) -> ParameterSets {
    let nalus = split_annex_b_nalus(data);
    let mut sps = None;
    let mut pps = None;
    let mut vps = None;

    for nalu in nalus {
        if nalu.is_empty() {
            continue;
        }
        match codec {
            CodecType::H264 => {
                let nalu_type = nalu[0] & 0x1F;
                match nalu_type {
                    7 => sps = Some(Bytes::copy_from_slice(nalu)), // SPS
                    8 => pps = Some(Bytes::copy_from_slice(nalu)), // PPS
                    _ => {}
                }
            }
            CodecType::H265 => {
                let nalu_type = (nalu[0] >> 1) & 0x3F;
                match nalu_type {
                    32 => vps = Some(Bytes::copy_from_slice(nalu)), // VPS
                    33 => sps = Some(Bytes::copy_from_slice(nalu)), // SPS
                    34 => pps = Some(Bytes::copy_from_slice(nalu)), // PPS
                    _ => {}
                }
            }
            _ => {}
        }
    }

    ParameterSets { sps, pps, vps }
}

/// 提取的参数集
#[derive(Debug, Clone)]
pub struct ParameterSets {
    pub sps: Option<Bytes>,
    pub pps: Option<Bytes>,
    pub vps: Option<Bytes>,
}

impl ParameterSets {
    /// 参数集是否完整（满足建立解码上下文的最低要求）
    pub fn is_complete(&self, codec: CodecType) -> bool {
        match codec {
            CodecType::H264 => self.sps.is_some() && self.pps.is_some(),
            CodecType::H265 => self.vps.is_some() && self.sps.is_some() && self.pps.is_some(),
            _ => false,
        }
    }

    /// 构建 avcC box body（ISO 14496-15 §5.3.3.1）
    pub fn build_avcc(&self) -> Option<Vec<u8>> {
        let sps = self.sps.as_ref()?;
        let pps = self.pps.as_ref()?;
        if sps.len() < 3 {
            return None;
        }

        let mut buf = Vec::with_capacity(11 + sps.len() + pps.len());
        buf.push(0x01); // configurationVersion
        buf.push(sps[1]); // AVCProfileIndication (profile_idc)
        buf.push(sps[2]); // profile_compatibility
        buf.push(sps[3]); // AVCLevelIndication (level_idc)
        buf.push(0xFF); // lengthSizeMinusOne = 3 (4 bytes) | reserved 6 bits = 0x3F
        buf.push(0xE1); // numOfSequenceParameterSets = 1 | reserved 3 bits = 0xE0

        // SPS
        buf.extend_from_slice(&(sps.len() as u16).to_be_bytes());
        buf.extend_from_slice(sps);

        // PPS
        buf.push(0x01); // numOfPictureParameterSets
        buf.extend_from_slice(&(pps.len() as u16).to_be_bytes());
        buf.extend_from_slice(pps);

        Some(buf)
    }

    /// 构建 hvcC box body（ISO 14496-15 §8.3.3.1）
    ///
    /// 简化版本：仅包含 VPS/SPS/PPS 各一条，足够播放器初始化解码器。
    pub fn build_hvcc(&self) -> Option<Vec<u8>> {
        let vps = self.vps.as_ref()?;
        let sps = self.sps.as_ref()?;
        let pps = self.pps.as_ref()?;

        let mut buf = Vec::with_capacity(23 + vps.len() + sps.len() + pps.len());

        // HEVCDecoderConfigurationRecord (简化)
        buf.push(0x01); // configurationVersion
        // general_profile_space(2) + general_tier_flag(1) + general_profile_idc(5)
        buf.push(sps.get(2).copied().unwrap_or(0) & 0x1F); // just profile_idc from SPS NAL body
        buf.extend_from_slice(&[0x00; 4]); // general_profile_compatibility_flags
        buf.extend_from_slice(&[0x00; 6]); // general_constraint_indicator_flags
        buf.push(sps.get(3).copied().unwrap_or(0)); // general_level_idc
        buf.extend_from_slice(&0xF000u16.to_be_bytes()); // min_spatial_segmentation_idc (reserved)
        buf.push(0xFC); // parallelismType (reserved)
        buf.push(0xFC | 1); // chromaFormat = 1 (4:2:0) | reserved
        buf.push(0xF8); // bitDepthLumaMinus8 | reserved
        buf.push(0xF8); // bitDepthChromaMinus8 | reserved
        buf.extend_from_slice(&0u16.to_be_bytes()); // avgFrameRate (0 = unspecified)
        // constantFrameRate(2) + numTemporalLayers(3) + temporalIdNested(1) + lengthSizeMinusOne(2)
        buf.push(0x0F); // lengthSizeMinusOne=3, others=0
        buf.push(3); // numOfArrays = 3 (VPS, SPS, PPS)

        // Array: VPS (type=32)
        buf.push(0x20 | 0x80); // array_completeness=1 | NAL_unit_type=32
        buf.extend_from_slice(&1u16.to_be_bytes()); // numNalus
        buf.extend_from_slice(&(vps.len() as u16).to_be_bytes());
        buf.extend_from_slice(vps);

        // Array: SPS (type=33)
        buf.push(0x21 | 0x80);
        buf.extend_from_slice(&1u16.to_be_bytes());
        buf.extend_from_slice(&(sps.len() as u16).to_be_bytes());
        buf.extend_from_slice(sps);

        // Array: PPS (type=34)
        buf.push(0x22 | 0x80);
        buf.extend_from_slice(&1u16.to_be_bytes());
        buf.extend_from_slice(&(pps.len() as u16).to_be_bytes());
        buf.extend_from_slice(pps);

        Some(buf)
    }
}

/// 将一个 Annex-B 格式的帧 payload 转换为 AVCC 格式（4 字节 length prefix）。
///
/// 过滤掉参数集 NALU（SPS/PPS/VPS），仅保留 slice NALU。
/// 返回转换后的总字节切片。
pub fn annex_b_to_avcc(data: &[u8], codec: CodecType) -> Vec<u8> {
    let nalus = split_annex_b_nalus(data);
    let mut out = Vec::with_capacity(data.len());

    for nalu in nalus {
        if nalu.is_empty() {
            continue;
        }

        // 跳过参数集 NALU（已写入 moov 的 avcC/hvcC）
        let skip = match codec {
            CodecType::H264 => {
                let nalu_type = nalu[0] & 0x1F;
                matches!(nalu_type, 7 | 8) // SPS, PPS
            }
            CodecType::H265 => {
                let nalu_type = (nalu[0] >> 1) & 0x3F;
                matches!(nalu_type, 32..=34) // VPS, SPS, PPS
            }
            _ => false,
        };

        if skip {
            continue;
        }

        // 写入 4 字节 length prefix + NALU body
        let len = nalu.len() as u32;
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(nalu);
    }

    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_h264_parameter_sets() {
        // SPS(type=7) + PPS(type=8) + IDR(type=5)
        let data = b"\x00\x00\x00\x01\x67\x64\x00\x29\x00\x00\x00\x01\x68\xEE\x3C\x80\x00\x00\x00\x01\x65idr_data";
        let ps = extract_parameter_sets(data, CodecType::H264);
        assert!(ps.sps.is_some());
        assert!(ps.pps.is_some());
        assert!(ps.vps.is_none());
        assert!(ps.is_complete(CodecType::H264));
        assert_eq!(ps.sps.unwrap()[0] & 0x1F, 7);
        assert_eq!(ps.pps.unwrap()[0] & 0x1F, 8);
    }

    #[test]
    fn test_extract_h265_parameter_sets() {
        // VPS(type=32) + SPS(type=33) + PPS(type=34) — each 2-byte NAL header
        // VPS: 0x40 0x01 (nalu_type=32), SPS: 0x42 0x01 (nalu_type=33), PPS: 0x44 0x01 (nalu_type=34)
        let mut data = Vec::new();
        data.extend_from_slice(b"\x00\x00\x00\x01\x40\x01vps_body");
        data.extend_from_slice(b"\x00\x00\x00\x01\x42\x01\x00\x00sps_body");
        data.extend_from_slice(b"\x00\x00\x00\x01\x44\x01pps_body");

        let ps = extract_parameter_sets(&data, CodecType::H265);
        assert!(ps.is_complete(CodecType::H265));
    }

    #[test]
    fn test_build_avcc() {
        let ps = ParameterSets {
            // SPS: type=7, profile=100, compat=0, level=41, ...
            sps: Some(Bytes::from_static(b"\x67\x64\x00\x29extra")),
            pps: Some(Bytes::from_static(b"\x68\xEE\x3C\x80")),
            vps: None,
        };
        let avcc = ps.build_avcc().unwrap();
        assert_eq!(avcc[0], 0x01); // configurationVersion
        assert_eq!(avcc[1], 0x64); // profile_idc = 100
        assert_eq!(avcc[3], 0x29); // level_idc = 41
        assert_eq!(avcc[4], 0xFF); // lengthSizeMinusOne = 3
        assert_eq!(avcc[5], 0xE1); // numSPS = 1
    }

    #[test]
    fn test_annex_b_to_avcc_filters_parameter_sets() {
        // SPS + PPS + IDR
        let data = b"\x00\x00\x00\x01\x67sps\x00\x00\x00\x01\x68pps\x00\x00\x00\x01\x65idr_slice";
        let avcc = annex_b_to_avcc(data, CodecType::H264);

        // Should only contain the IDR slice, not SPS/PPS
        assert!(!avcc.is_empty());
        // First 4 bytes = length prefix of IDR NALU
        let nalu_len = u32::from_be_bytes(avcc[0..4].try_into().unwrap());
        assert_eq!(nalu_len as usize, avcc.len() - 4);
        // IDR NALU type
        assert_eq!(avcc[4] & 0x1F, 5);
    }

    #[test]
    fn test_annex_b_to_avcc_non_keyframe() {
        // P-frame only (type=1)
        let data = b"\x00\x00\x00\x01\x41pframe_data";
        let avcc = annex_b_to_avcc(data, CodecType::H264);
        let nalu_len = u32::from_be_bytes(avcc[0..4].try_into().unwrap());
        assert_eq!(nalu_len as usize + 4, avcc.len());
        assert_eq!(avcc[4] & 0x1F, 1);
    }
}
