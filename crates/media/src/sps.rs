use types::CodecType;

use crate::error::MediaError;

/// SPS 解析得出的视频参数元数据
#[derive(Debug, Clone, PartialEq)]
pub struct SpsInfo {
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub profile_idc: u8,
    pub level_idc: u8,
}

/// 将字节切片中的 Annex B NALU 单元拆分（支持 0x00000001 与 0x000001 起始码）
pub fn split_annex_b_nalus(data: &[u8]) -> Vec<&[u8]> {
    let len = data.len();
    if len < 3 {
        return if data.is_empty() {
            Vec::new()
        } else {
            vec![data]
        };
    }

    let mut start_codes = Vec::new();
    let mut i = 0;
    while i < len - 2 {
        if data[i] == 0 && data[i + 1] == 0 {
            if i + 3 < len && data[i + 2] == 0 && data[i + 3] == 1 {
                start_codes.push((i, i + 4));
                i += 4;
                continue;
            } else if data[i + 2] == 1 {
                start_codes.push((i, i + 3));
                i += 3;
                continue;
            }
        }
        i += 1;
    }

    if start_codes.is_empty() {
        return vec![data];
    }

    let mut nalus = Vec::with_capacity(start_codes.len());
    for (idx, &(_, payload_start)) in start_codes.iter().enumerate() {
        let payload_end = if idx + 1 < start_codes.len() {
            start_codes[idx + 1].0
        } else {
            len
        };
        if payload_start < payload_end {
            nalus.push(&data[payload_start..payload_end]);
        }
    }

    nalus
}

/// 移除 H.264 / H.265 NALU 中的防竞争字节 (0x00 0x00 0x03 -> 0x00 0x00)
pub fn remove_emulation_prevention(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut zero_count = 0;

    for &b in data {
        if zero_count >= 2 && b == 0x03 {
            // 跳过防竞争字节 0x03，重置 0 计数
            zero_count = 0;
            continue;
        }
        if b == 0x00 {
            zero_count += 1;
        } else {
            zero_count = 0;
        }
        out.push(b);
    }
    out
}

/// 按位读取辅助结构体
#[derive(Debug)]
pub struct BitReader<'a> {
    data: &'a [u8],
    bit_pos: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, bit_pos: 0 }
    }

    pub fn bits_left(&self) -> usize {
        let total_bits = self.data.len() * 8;
        total_bits.saturating_sub(self.bit_pos)
    }

    pub fn read_bit(&mut self) -> Result<u32, MediaError> {
        if self.bit_pos >= self.data.len() * 8 {
            return Err(MediaError::SpsParse("比特流读取越界".into()));
        }
        let byte_idx = self.bit_pos / 8;
        let bit_idx = 7 - (self.bit_pos % 8);
        self.bit_pos += 1;
        Ok(((self.data[byte_idx] >> bit_idx) & 1) as u32)
    }

    pub fn read_bits(&mut self, n: usize) -> Result<u32, MediaError> {
        if n == 0 {
            return Ok(0);
        }
        if n > 32 {
            return Err(MediaError::SpsParse("单次读取比特数不能超过 32 位".into()));
        }
        let mut val = 0u32;
        for _ in 0..n {
            val = (val << 1) | self.read_bit()?;
        }
        Ok(val)
    }

    /// 读取无符号指数哥伦布编码 (Unsigned Exponential-Golomb / ue)
    pub fn read_ue(&mut self) -> Result<u32, MediaError> {
        let mut leading_zeros = 0;
        while self.read_bit()? == 0 {
            leading_zeros += 1;
            if leading_zeros > 31 {
                return Err(MediaError::SpsParse("指数哥伦布编码前导 0 过多".into()));
            }
        }

        if leading_zeros == 0 {
            return Ok(0);
        }

        let info = self.read_bits(leading_zeros)?;
        Ok((1 << leading_zeros) - 1 + info)
    }

    /// 读取有符号指数哥伦布编码 (Signed Exponential-Golomb / se)
    pub fn read_se(&mut self) -> Result<i32, MediaError> {
        let code_num = self.read_ue()?;
        if code_num % 2 == 0 {
            Ok(-((code_num / 2) as i32))
        } else {
            Ok(code_num.div_ceil(2) as i32)
        }
    }
}

/// 解析 H.264 SPS (Sequence Parameter Set)
pub fn parse_h264_sps(raw_bytes: &[u8]) -> Result<SpsInfo, MediaError> {
    let mut data = raw_bytes;

    // 剥离 NALU 起始码 00 00 01 或 00 00 00 01
    if data.starts_with(&[0, 0, 0, 1]) {
        data = &data[4..];
    } else if data.starts_with(&[0, 0, 1]) {
        data = &data[3..];
    }

    if data.is_empty() {
        return Err(MediaError::SpsParse("SPS 数据为空".into()));
    }

    // 检查 NAL 头部：若首字节的 nal_unit_type == 7 (SPS)，跳过 1 字节 NAL Header
    let nal_unit_type = data[0] & 0x1F;
    if nal_unit_type == 7 {
        data = &data[1..];
    }

    let unescaped = remove_emulation_prevention(data);
    if unescaped.len() < 3 {
        return Err(MediaError::SpsParse("SPS 数据长度不足".into()));
    }

    let mut reader = BitReader::new(&unescaped);

    let profile_idc = reader.read_bits(8)? as u8;
    let _constraint_set_flags = reader.read_bits(8)?;
    let level_idc = reader.read_bits(8)? as u8;
    let _seq_parameter_set_id = reader.read_ue()?;

    let mut chroma_format_idc = 1; // 缺省为 4:2:0
    if matches!(
        profile_idc,
        100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135
    ) {
        chroma_format_idc = reader.read_ue()?;
        if chroma_format_idc == 3 {
            let _separate_colour_plane_flag = reader.read_bit()?;
        }
        let _bit_depth_luma_minus8 = reader.read_ue()?;
        let _bit_depth_chroma_minus8 = reader.read_ue()?;
        let _qpprime_y_zero_transform_bypass_flag = reader.read_bit()?;
        let seq_scaling_matrix_present_flag = reader.read_bit()?;
        if seq_scaling_matrix_present_flag != 0 {
            let count = if chroma_format_idc != 3 { 8 } else { 12 };
            for _ in 0..count {
                let seq_scaling_list_present_flag = reader.read_bit()?;
                if seq_scaling_list_present_flag != 0 {
                    let size_of_scaling_list = 16;
                    let mut last_scale = 8;
                    let mut next_scale = 8;
                    for _ in 0..size_of_scaling_list {
                        if next_scale != 0 {
                            let delta_scale = reader.read_se()?;
                            next_scale = (last_scale + delta_scale + 256) % 256;
                        }
                        last_scale = if next_scale == 0 {
                            last_scale
                        } else {
                            next_scale
                        };
                    }
                }
            }
        }
    }

    let _log2_max_frame_num_minus4 = reader.read_ue()?;
    let pic_order_cnt_type = reader.read_ue()?;

    if pic_order_cnt_type == 0 {
        let _log2_max_pic_order_cnt_lsb_minus4 = reader.read_ue()?;
    } else if pic_order_cnt_type == 1 {
        let _delta_pic_order_always_zero_flag = reader.read_bit()?;
        let _offset_for_non_ref_pic = reader.read_se()?;
        let _offset_for_top_to_bottom_field = reader.read_se()?;
        let num_ref_frames_in_pic_order_cnt_cycle = reader.read_ue()?;
        for _ in 0..num_ref_frames_in_pic_order_cnt_cycle {
            let _ = reader.read_se()?;
        }
    }

    let _max_num_ref_frames = reader.read_ue()?;
    let _gaps_in_frame_num_value_allowed_flag = reader.read_bit()?;

    let pic_width_in_mbs_minus1 = reader.read_ue()?;
    let pic_height_in_map_units_minus1 = reader.read_ue()?;
    let frame_mbs_only_flag = reader.read_bit()?;

    if frame_mbs_only_flag == 0 {
        let _mb_adaptive_frame_field_flag = reader.read_bit()?;
    }

    let _direct_8x8_inference_flag = reader.read_bit()?;
    let frame_cropping_flag = reader.read_bit()?;

    let mut crop_left = 0u32;
    let mut crop_right = 0u32;
    let mut crop_top = 0u32;
    let mut crop_bottom = 0u32;

    if frame_cropping_flag != 0 {
        crop_left = reader.read_ue()?;
        crop_right = reader.read_ue()?;
        crop_top = reader.read_ue()?;
        crop_bottom = reader.read_ue()?;
    }

    let crop_unit_x = match chroma_format_idc {
        1 | 2 => 2,
        3 => 1,
        _ => 1,
    };
    let crop_unit_y = match chroma_format_idc {
        1 => 2 * (2 - frame_mbs_only_flag),
        2 | 3 => 2 - frame_mbs_only_flag,
        _ => 2 - frame_mbs_only_flag,
    };

    let width =
        ((pic_width_in_mbs_minus1 + 1) * 16).saturating_sub((crop_left + crop_right) * crop_unit_x);
    let height = (((2 - frame_mbs_only_flag) * (pic_height_in_map_units_minus1 + 1)) * 16)
        .saturating_sub((crop_top + crop_bottom) * crop_unit_y);

    let mut fps = 25.0f64; // 缺省 25fps
    let vui_parameters_present_flag = reader.read_bit().unwrap_or(0);
    if vui_parameters_present_flag != 0 {
        let aspect_ratio_info_present_flag = reader.read_bit().unwrap_or(0);
        if aspect_ratio_info_present_flag != 0 {
            let aspect_ratio_idc = reader.read_bits(8).unwrap_or(0);
            if aspect_ratio_idc == 255 {
                let _sar_width = reader.read_bits(16).unwrap_or(0);
                let _sar_height = reader.read_bits(16).unwrap_or(0);
            }
        }

        let overscan_info_present_flag = reader.read_bit().unwrap_or(0);
        if overscan_info_present_flag != 0 {
            let _overscan_appropriate_flag = reader.read_bit().unwrap_or(0);
        }

        let video_signal_type_present_flag = reader.read_bit().unwrap_or(0);
        if video_signal_type_present_flag != 0 {
            let _video_format = reader.read_bits(3).unwrap_or(0);
            let _video_full_range_flag = reader.read_bit().unwrap_or(0);
            let colour_description_present_flag = reader.read_bit().unwrap_or(0);
            if colour_description_present_flag != 0 {
                let _colour_primaries = reader.read_bits(8).unwrap_or(0);
                let _transfer_characteristics = reader.read_bits(8).unwrap_or(0);
                let _matrix_coefficients = reader.read_bits(8).unwrap_or(0);
            }
        }

        let chroma_loc_info_present_flag = reader.read_bit().unwrap_or(0);
        if chroma_loc_info_present_flag != 0 {
            let _chroma_sample_loc_type_top_field = reader.read_ue().unwrap_or(0);
            let _chroma_sample_loc_type_bottom_field = reader.read_ue().unwrap_or(0);
        }

        let timing_info_present_flag = reader.read_bit().unwrap_or(0);
        if timing_info_present_flag != 0 {
            let num_units_in_tick = reader.read_bits(32).unwrap_or(0);
            let time_scale = reader.read_bits(32).unwrap_or(0);
            let _fixed_frame_rate_flag = reader.read_bit().unwrap_or(0);
            if num_units_in_tick > 0 && time_scale > 0 {
                let calculated_fps = (time_scale as f64) / (2.0 * num_units_in_tick as f64);
                if calculated_fps > 1.0 && calculated_fps < 240.0 {
                    fps = calculated_fps;
                }
            }
        }
    }

    Ok(SpsInfo {
        codec: "h264".to_string(),
        width,
        height,
        fps,
        profile_idc,
        level_idc,
    })
}

/// 解析 H.265 (HEVC) SPS
pub fn parse_h265_sps(raw_bytes: &[u8]) -> Result<SpsInfo, MediaError> {
    let mut data = raw_bytes;

    if data.starts_with(&[0, 0, 0, 1]) {
        data = &data[4..];
    } else if data.starts_with(&[0, 0, 1]) {
        data = &data[3..];
    }

    if data.is_empty() {
        return Err(MediaError::SpsParse("H.265 SPS 数据为空".into()));
    }

    // H.265 NAL Header 为 2 字节，SPS 的 nal_unit_type 为 33 (0x21)
    let nal_unit_type = (data[0] >> 1) & 0x3F;
    if nal_unit_type == 33 {
        data = &data[2..];
    }

    let unescaped = remove_emulation_prevention(data);
    if unescaped.len() < 12 {
        return Err(MediaError::SpsParse("H.265 SPS 长度不足".into()));
    }

    let mut reader = BitReader::new(&unescaped);

    let _sps_video_parameter_set_id = reader.read_bits(4)?;
    let sps_max_sub_layers_minus1 = reader.read_bits(3)?;
    let _sps_temporal_id_nesting_flag = reader.read_bit()?;

    // Profile Tier Level 解析 (简明跳转)
    let _general_profile_space = reader.read_bits(2)?;
    let _general_tier_flag = reader.read_bit()?;
    let general_profile_idc = reader.read_bits(5)? as u8;
    let _general_profile_compatibility_flags = reader.read_bits(32)?;
    let _general_constraint_flags_hi = reader.read_bits(32)?;
    let _general_constraint_flags_lo = reader.read_bits(16)?;
    let general_level_idc = reader.read_bits(8)? as u8;

    let mut sub_layer_profile_present_flag = Vec::new();
    let mut sub_layer_level_present_flag = Vec::new();
    for _ in 0..sps_max_sub_layers_minus1 {
        sub_layer_profile_present_flag.push(reader.read_bit()? != 0);
        sub_layer_level_present_flag.push(reader.read_bit()? != 0);
    }
    if sps_max_sub_layers_minus1 > 0 {
        for _ in sps_max_sub_layers_minus1..8 {
            let _ = reader.read_bits(2)?;
        }
    }
    for i in 0..sps_max_sub_layers_minus1 as usize {
        if sub_layer_profile_present_flag[i] {
            let _ = reader.read_bits(32)?;
            let _ = reader.read_bits(32)?;
            let _ = reader.read_bits(24)?;
        }
        if sub_layer_level_present_flag[i] {
            let _ = reader.read_bits(8)?;
        }
    }

    let _sps_seq_parameter_set_id = reader.read_ue()?;
    let chroma_format_idc = reader.read_ue()?;
    if chroma_format_idc == 3 {
        let _separate_colour_plane_flag = reader.read_bit()?;
    }

    let pic_width_in_luma_samples = reader.read_ue()?;
    let pic_height_in_luma_samples = reader.read_ue()?;
    let conformance_window_flag = reader.read_bit()?;

    let mut conf_win_left_offset = 0u32;
    let mut conf_win_right_offset = 0u32;
    let mut conf_win_top_offset = 0u32;
    let mut conf_win_bottom_offset = 0u32;

    if conformance_window_flag != 0 {
        conf_win_left_offset = reader.read_ue()?;
        conf_win_right_offset = reader.read_ue()?;
        conf_win_top_offset = reader.read_ue()?;
        conf_win_bottom_offset = reader.read_ue()?;
    }

    let sub_width_c = if chroma_format_idc == 1 || chroma_format_idc == 2 {
        2
    } else {
        1
    };
    let sub_height_c = if chroma_format_idc == 1 { 2 } else { 1 };

    let width = pic_width_in_luma_samples
        .saturating_sub((conf_win_left_offset + conf_win_right_offset) * sub_width_c);
    let height = pic_height_in_luma_samples
        .saturating_sub((conf_win_top_offset + conf_win_bottom_offset) * sub_height_c);

    Ok(SpsInfo {
        codec: "h265".to_string(),
        width,
        height,
        fps: 25.0,
        profile_idc: general_profile_idc,
        level_idc: general_level_idc,
    })
}

/// 快速判定切片中是否包含关键帧 (IDR) 或序列参数集 (SPS/PPS/VPS)
///
/// 零堆分配，通过解析 Annex B NALU 头部直接识别帧类型，用于实时丢帧防花屏判定
pub fn is_keyframe_or_parameter_set(data: &[u8], codec: CodecType) -> bool {
    for nalu in split_annex_b_nalus(data) {
        if let Some(&first) = nalu.first() {
            let matches_header = match codec {
                // 5: IDR, 7: SPS, 8: PPS
                CodecType::H264 => matches!(first & 0x1F, 5 | 7 | 8),
                // 16..=21: IRAP (BLA/IDR/CRA), 32: VPS, 33: SPS, 34: PPS
                CodecType::H265 => matches!((first >> 1) & 0x3F, 16..=21 | 32..=34),
            };
            if matches_header {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn test_remove_emulation_prevention() {
        let raw = vec![0x00, 0x00, 0x03, 0x01, 0x00, 0x00, 0x03, 0x02];
        let cleaned = remove_emulation_prevention(&raw);
        assert_eq!(cleaned, vec![0x00, 0x00, 0x01, 0x00, 0x00, 0x02]);
    }

    #[test]
    fn test_parse_1080p_h264_sps() {
        // 标准 1080P @ 30fps H.264 SPS (High Profile, Level 4.1)
        // 00 00 00 01 67 64 00 29 ac 72 84 40 78 02 27 e5 c0 44 00 00 03 00 04 00 00 03 00 f0 3c 60 c6 58
        let sps_bytes = [
            0x00, 0x00, 0x00, 0x01, 0x67, 0x64, 0x00, 0x29, 0xac, 0x72, 0x84, 0x40, 0x78, 0x02,
            0x27, 0xe5, 0xc0, 0x44, 0x00, 0x00, 0x03, 0x00, 0x04, 0x00, 0x00, 0x03, 0x00, 0xf0,
            0x3c, 0x60, 0xc6, 0x58,
        ];

        let res = parse_h264_sps(&sps_bytes).expect("parse 1080p sps");
        assert_eq!(res.codec, "h264");
        assert_eq!(res.width, 1920);
        assert_eq!(res.height, 1080);
        assert_eq!(res.profile_idc, 100); // High profile
        assert_eq!(res.level_idc, 41); // Level 4.1
        assert_eq!(res.fps.round() as u32, 30);
    }

    #[test]
    fn test_parse_720p_h264_sps() {
        // 典型 720P (1280x720) SPS (Baseline/Main Profile)
        // 0x67, 0x42, 0x00, 0x1f, 0x96, 0x35, 0x40, 0xa0, 0x0b, 0x76, 0x02, 0xd4, 0x04, 0x04, 0x05, 0x00
        let sps_bytes = [
            0x67, 0x42, 0x00, 0x1f, 0x96, 0x35, 0x40, 0xa0, 0x0b, 0x76, 0x02, 0xd4, 0x04, 0x04,
            0x05, 0x00,
        ];

        let res = parse_h264_sps(&sps_bytes).expect("parse 720p sps");
        assert_eq!(res.codec, "h264");
        assert_eq!(res.width, 1280);
        assert_eq!(res.height, 720);
        assert_eq!(res.profile_idc, 66); // Baseline profile
    }

    #[test]
    fn test_parse_1080p_h265_sps() {
        // 标准 1080P (1920x1080) HEVC SPS (Main Profile)
        let sps_b64 = "QgEBAWAAAAMAsAAAAwAAAwB4oAPAgBDllmZpJMreEAAAAEAg";
        let sps_bytes = base64::engine::general_purpose::STANDARD
            .decode(sps_b64)
            .expect("decode b64");

        let res = parse_h265_sps(&sps_bytes).expect("parse 1080p h265 sps");
        assert_eq!(res.codec, "h265");
        assert_eq!(res.width, 1920);
        assert_eq!(res.height, 1080);
        assert_eq!(res.profile_idc, 1); // Main Profile
    }

    #[test]
    fn test_split_annex_b_nalus() {
        let multi_nalu_data =
            b"\x00\x00\x00\x01\x67sps_data\x00\x00\x01\x68pps_data\x00\x00\x00\x01\x65idr_data";
        let nalus = split_annex_b_nalus(multi_nalu_data);

        assert_eq!(nalus.len(), 3);
        assert_eq!(nalus[0], b"\x67sps_data");
        assert_eq!(nalus[1], b"\x68pps_data");
        assert_eq!(nalus[2], b"\x65idr_data");
    }

    #[test]
    fn test_is_keyframe_or_parameter_set() {
        // H.264 IDR (0x65 -> nalu_type 5)
        assert!(is_keyframe_or_parameter_set(
            b"\x00\x00\x00\x01\x65idr",
            CodecType::H264
        ));
        // H.264 SPS (0x67 -> nalu_type 7)
        assert!(is_keyframe_or_parameter_set(
            b"\x00\x00\x01\x67sps",
            CodecType::H264
        ));
        // H.264 P 帧 (0x41 -> nalu_type 1)
        assert!(!is_keyframe_or_parameter_set(
            b"\x00\x00\x00\x01\x41pframe",
            CodecType::H264
        ));

        // H.265 IDR (0x26 -> nalu_type 19)
        assert!(is_keyframe_or_parameter_set(
            b"\x00\x00\x00\x01\x26idr",
            CodecType::H265
        ));
        // H.265 P 帧 (0x02 -> nalu_type 1)
        assert!(!is_keyframe_or_parameter_set(
            b"\x00\x00\x00\x01\x02trail",
            CodecType::H265
        ));
    }
}
