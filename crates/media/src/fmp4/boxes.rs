//! MP4 box 序列化原语
//!
//! 每个 `write_*` 函数将一个完整的 box 写入 `impl Write`。
//! 所有整数按 big-endian 写入（ISO 14496-12 规范）。

use std::io::{self, Write};

use types::CodecType;

// ──────────────────────────── 常量 ────────────────────────────

const TIMESCALE: u32 = 90_000; // 视频通用 timescale

// ──────────────────────────── 工具宏 ────────────────────────────

/// 写入一个 box：先计算长度，再写 header + body。
///
/// body 闭包接收 `&mut Vec<u8>` 参数，写入内容到该 buffer。
macro_rules! write_box {
    ($outer:expr, $box_type:expr, |$bw:ident| $body:block) => {{
        let mut __wb_body: Vec<u8> = Vec::new();
        {
            let $bw = &mut __wb_body;
            $body
        }
        let __wb_sz = 8u32 + __wb_body.len() as u32;
        $outer.write_all(&__wb_sz.to_be_bytes())?;
        $outer.write_all($box_type)?;
        $outer.write_all(&__wb_body)?;
    }};
}

// ──────────────────────────── 文件头 ────────────────────────────

/// 写入 `ftyp` box。
pub fn write_ftyp(w: &mut impl Write) -> io::Result<()> {
    write_box!(w, b"ftyp", |w| {
        w.write_all(b"isom")?; // major brand
        w.write_all(&1u32.to_be_bytes())?; // minor version
        w.write_all(b"isom")?; // compatible brands
        w.write_all(b"iso5")?;
        w.write_all(b"iso6")?;
        w.write_all(b"msdh")?;
        w.write_all(b"msix")?;
    });
    Ok(())
}

// ──────────────────────────── moov ────────────────────────────

/// 写入初始化段 `moov`（含空 sample table + `mvex`）。
///
/// - `codec`: H264 或 H265
/// - `width`, `height`: 视频尺寸（像素）
/// - `codec_private`: avcC 或 hvcC 的完整 box body（不含 box header）
pub fn write_moov(
    w: &mut impl Write,
    codec: CodecType,
    width: u16,
    height: u16,
    codec_private: &[u8],
) -> io::Result<()> {
    write_box!(w, b"moov", |w| {
        // ── mvhd ──
        write_box!(w, b"mvhd", |w| {
            w.write_all(&0u32.to_be_bytes())?; // version + flags
            w.write_all(&0u32.to_be_bytes())?; // creation_time
            w.write_all(&0u32.to_be_bytes())?; // modification_time
            w.write_all(&TIMESCALE.to_be_bytes())?; // timescale
            w.write_all(&0u32.to_be_bytes())?; // duration (unknown for fMP4)
            w.write_all(&0x00010000u32.to_be_bytes())?; // rate = 1.0
            w.write_all(&0x0100u16.to_be_bytes())?; // volume = 1.0
            w.write_all(&[0u8; 10])?; // reserved
                                      // unity matrix 3x3 (36 bytes)
            for &v in &[0x00010000u32, 0, 0, 0, 0x00010000, 0, 0, 0, 0x40000000] {
                w.write_all(&v.to_be_bytes())?;
            }
            w.write_all(&[0u8; 24])?; // pre_defined
            w.write_all(&2u32.to_be_bytes())?; // next_track_ID
        });

        // ── trak ──
        write_box!(w, b"trak", |w| {
            // tkhd
            write_box!(w, b"tkhd", |w| {
                // version=0, flags=track_enabled|track_in_movie (0x000003)
                w.write_all(&0x00000003u32.to_be_bytes())?;
                w.write_all(&0u32.to_be_bytes())?; // creation_time
                w.write_all(&0u32.to_be_bytes())?; // modification_time
                w.write_all(&1u32.to_be_bytes())?; // track_ID = 1
                w.write_all(&0u32.to_be_bytes())?; // reserved
                w.write_all(&0u32.to_be_bytes())?; // duration
                w.write_all(&[0u8; 8])?; // reserved
                w.write_all(&0u16.to_be_bytes())?; // layer
                w.write_all(&0u16.to_be_bytes())?; // alternate_group
                w.write_all(&0u16.to_be_bytes())?; // volume (0 for video)
                w.write_all(&0u16.to_be_bytes())?; // reserved
                                                   // unity matrix
                for &v in &[0x00010000u32, 0, 0, 0, 0x00010000, 0, 0, 0, 0x40000000] {
                    w.write_all(&v.to_be_bytes())?;
                }
                // width, height (16.16 fixed-point)
                w.write_all(&((width as u32) << 16).to_be_bytes())?;
                w.write_all(&((height as u32) << 16).to_be_bytes())?;
            });

            // mdia
            write_box!(w, b"mdia", |w| {
                // mdhd
                write_box!(w, b"mdhd", |w| {
                    w.write_all(&0u32.to_be_bytes())?; // version + flags
                    w.write_all(&0u32.to_be_bytes())?; // creation_time
                    w.write_all(&0u32.to_be_bytes())?; // modification_time
                    w.write_all(&TIMESCALE.to_be_bytes())?; // timescale
                    w.write_all(&0u32.to_be_bytes())?; // duration
                    w.write_all(&0x55C4u16.to_be_bytes())?; // language = 'und'
                    w.write_all(&0u16.to_be_bytes())?; // pre_defined
                });

                // hdlr
                write_box!(w, b"hdlr", |w| {
                    w.write_all(&0u32.to_be_bytes())?; // version + flags
                    w.write_all(&0u32.to_be_bytes())?; // pre_defined
                    w.write_all(b"vide")?; // handler_type
                    w.write_all(&[0u8; 12])?; // reserved
                    w.write_all(b"VideoHandler\0")?; // name (null-terminated)
                });

                // minf
                write_box!(w, b"minf", |w| {
                    // vmhd
                    write_box!(w, b"vmhd", |w| {
                        w.write_all(&0x00000001u32.to_be_bytes())?; // version=0, flags=1
                        w.write_all(&0u16.to_be_bytes())?; // graphicsmode
                        w.write_all(&[0u8; 6])?; // opcolor
                    });

                    // dinf → dref
                    write_box!(w, b"dinf", |w| {
                        write_box!(w, b"dref", |w| {
                            w.write_all(&0u32.to_be_bytes())?; // version + flags
                            w.write_all(&1u32.to_be_bytes())?; // entry_count = 1
                            write_box!(w, b"url ", |w| {
                                w.write_all(&0x00000001u32.to_be_bytes())?; // flags = self-contained
                            });
                        });
                    });

                    // stbl (空 sample table，fMP4 必须)
                    write_box!(w, b"stbl", |w| {
                        write_stsd(w, codec, width, height, codec_private)?;

                        // stts (空)
                        write_box!(w, b"stts", |w| {
                            w.write_all(&0u32.to_be_bytes())?; // version + flags
                            w.write_all(&0u32.to_be_bytes())?; // entry_count = 0
                        });
                        // stsc (空)
                        write_box!(w, b"stsc", |w| {
                            w.write_all(&0u32.to_be_bytes())?;
                            w.write_all(&0u32.to_be_bytes())?;
                        });
                        // stsz (空)
                        write_box!(w, b"stsz", |w| {
                            w.write_all(&0u32.to_be_bytes())?; // version + flags
                            w.write_all(&0u32.to_be_bytes())?; // sample_size
                            w.write_all(&0u32.to_be_bytes())?; // sample_count
                        });
                        // stco (空)
                        write_box!(w, b"stco", |w| {
                            w.write_all(&0u32.to_be_bytes())?;
                            w.write_all(&0u32.to_be_bytes())?;
                        });
                    });
                });
            });
        });

        // ── mvex ──
        write_box!(w, b"mvex", |w| {
            write_box!(w, b"trex", |w| {
                w.write_all(&0u32.to_be_bytes())?; // version + flags
                w.write_all(&1u32.to_be_bytes())?; // track_ID = 1
                w.write_all(&1u32.to_be_bytes())?; // default_sample_description_index
                w.write_all(&0u32.to_be_bytes())?; // default_sample_duration
                w.write_all(&0u32.to_be_bytes())?; // default_sample_size
                w.write_all(&0u32.to_be_bytes())?; // default_sample_flags
            });
        });
    });
    Ok(())
}

/// 写入 `stsd` (Sample Description Box)，内含 `avc1` 或 `hvc1`。
fn write_stsd(
    w: &mut impl Write,
    codec: CodecType,
    width: u16,
    height: u16,
    codec_private: &[u8],
) -> io::Result<()> {
    write_box!(w, b"stsd", |w| {
        w.write_all(&0u32.to_be_bytes())?; // version + flags
        w.write_all(&1u32.to_be_bytes())?; // entry_count = 1

        let sample_entry_type = match codec {
            CodecType::H264 => b"avc1",
            CodecType::H265 => b"hvc1",
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "unsupported codec",
                ))
            }
        };
        let codec_config_box_type = match codec {
            CodecType::H264 => b"avcC",
            CodecType::H265 => b"hvcC",
            _ => unreachable!(),
        };

        write_box!(w, sample_entry_type, |w| {
            w.write_all(&[0u8; 6])?; // reserved
            w.write_all(&1u16.to_be_bytes())?; // data_reference_index = 1
            w.write_all(&[0u8; 16])?; // pre_defined + reserved
            w.write_all(&width.to_be_bytes())?; // width
            w.write_all(&height.to_be_bytes())?; // height
            w.write_all(&0x00480000u32.to_be_bytes())?; // horiz resolution 72 dpi
            w.write_all(&0x00480000u32.to_be_bytes())?; // vert resolution 72 dpi
            w.write_all(&0u32.to_be_bytes())?; // reserved
            w.write_all(&1u16.to_be_bytes())?; // frame_count = 1
            w.write_all(&[0u8; 32])?; // compressorname
            w.write_all(&0x0018u16.to_be_bytes())?; // depth = 24
            w.write_all(&0xFFFFu16.to_be_bytes())?; // pre_defined = -1

            // avcC / hvcC
            write_box!(w, codec_config_box_type, |w| {
                w.write_all(codec_private)?;
            });
        });
    });
    Ok(())
}

// ──────────────────────────── fragment (moof + mdat) ────────────────────────────

/// 单个 sample 的元数据（用于构建 `trun`）。
#[derive(Debug)]
pub struct SampleEntry {
    pub duration: u32, // timescale 单位
    pub size: u32,     // AVCC length-prefixed payload 字节数
    pub is_keyframe: bool,
    /// 显示时刻相对解码时刻的补偿（timescale 单位）。
    ///
    /// 无重排序时为 0；含 B 帧时为负（重排帧早于其解码槽位显示）。
    /// 对应 ISO 14496-12 的 `sample_composition_time_offset`。
    pub composition_offset: i32,
}

/// 写入 `trun` box。
///
/// 采样点按**解码顺序**排列（MP4 强制要求），显示顺序由 composition offset 表达。
fn write_trun(w: &mut impl Write, samples: &[SampleEntry]) -> io::Result<()> {
    // flags: data-offset(0x1) + duration(0x100) + size(0x200) + flags(0x400) = 0x701
    // 存在重排序时追加 composition-time-offset(0x800)
    let has_composition = samples.iter().any(|s| s.composition_offset != 0);
    // 负偏移只能由 version 1 承载，全部非负时保持 version 0 以获得最广的解析器兼容
    let needs_signed = samples.iter().any(|s| s.composition_offset < 0);
    let trun_flags: u32 = if has_composition { 0x000F01 } else { 0x000701 };
    let trun_version: u32 = if needs_signed { 1 } else { 0 };

    let sample_count = samples.len() as u32;

    // trun body = version+flags(4) + sample_count(4) + data_offset(4)
    //           + sample_count * (duration(4) + size(4) + flags(4) [+ composition(4)])
    let per_sample_bytes = if has_composition { 16u32 } else { 12u32 };
    let trun_body_size = 4 + 4 + 4 + sample_count * per_sample_bytes;
    let trun_box_size = 8 + trun_body_size;

    // data_offset 先写 0 作为 placeholder，由 write_fragment 调用 fixup_trun_data_offset 回填

    w.write_all(&trun_box_size.to_be_bytes())?;
    w.write_all(b"trun")?;
    w.write_all(&((trun_version << 24) | trun_flags).to_be_bytes())?;
    w.write_all(&sample_count.to_be_bytes())?;
    w.write_all(&0u32.to_be_bytes())?; // data_offset placeholder (index: 12 from trun start)

    for s in samples {
        w.write_all(&s.duration.to_be_bytes())?;
        w.write_all(&s.size.to_be_bytes())?;
        // sample_flags: ISO 14496-12 §8.8.3
        // keyframe: sample_depends_on=2 (does not depend) → 0x02000000
        // non-key:  sample_depends_on=1 (depends) + is_non_sync → 0x01010000
        let flags: u32 = if s.is_keyframe {
            0x02000000
        } else {
            0x01010000
        };
        w.write_all(&flags.to_be_bytes())?;
        if has_composition {
            w.write_all(&s.composition_offset.to_be_bytes())?;
        }
    }

    Ok(())
}

/// 修正 `moof` body 中 `trun` 的 `data_offset` 字段。
///
/// `data_offset` = moof box 总大小 + 8 (mdat header)。
/// 在 moof body buffer 中找到 trun box，定位到 data_offset 位置并回填。
pub fn fixup_trun_data_offset(moof_body: &mut [u8], moof_total_size: u32) {
    let data_offset = moof_total_size + 8; // moof size + mdat header (8 bytes)

    // 在 moof body 中查找 "trun" box
    // moof body 布局: mfhd(16 bytes body) | traf( tfhd | tfdt | trun )
    // 简单扫描 "trun" 四字节标签
    if let Some(pos) = find_box_tag(moof_body, b"trun") {
        // trun box: [size:4][type:4][version+flags:4][sample_count:4][data_offset:4]
        // data_offset 在 trun type 后偏移 8 字节 (version+flags + sample_count)
        let offset_pos = pos + 4 + 4 + 4; // type(4) + version_flags(4) + sample_count(4)
        if offset_pos + 4 <= moof_body.len() {
            moof_body[offset_pos..offset_pos + 4].copy_from_slice(&data_offset.to_be_bytes());
        }
    }
}

/// 在字节切片中查找 box type tag，返回 tag 起始位置（即 size 字段之后）。
fn find_box_tag(data: &[u8], tag: &[u8; 4]) -> Option<usize> {
    data.windows(4)
        .enumerate()
        .find(|(_, w)| *w == tag)
        .map(|(i, _)| i)
}

/// 写入一个完整的 fragment（`moof` + `mdat`），并修正 `trun` 的 `data_offset`。
///
/// 先缓冲 moof 到内存，修正 data_offset 后再写入。
pub fn write_fragment(
    w: &mut impl Write,
    sequence_number: u32,
    base_decode_time: u64,
    samples: &[SampleEntry],
    mdat_payload: &[u8],
) -> io::Result<()> {
    // 1. 构建 moof body 到内存
    let mut moof_body = Vec::new();

    // mfhd
    write_box!(&mut moof_body, b"mfhd", |w| {
        w.write_all(&0u32.to_be_bytes())?;
        w.write_all(&sequence_number.to_be_bytes())?;
    });

    // traf
    write_box!(&mut moof_body, b"traf", |w| {
        // tfhd
        write_box!(w, b"tfhd", |w| {
            w.write_all(&0x00020000u32.to_be_bytes())?;
            w.write_all(&1u32.to_be_bytes())?;
        });
        // tfdt
        write_box!(w, b"tfdt", |w| {
            w.write_all(&0x01000000u32.to_be_bytes())?;
            w.write_all(&base_decode_time.to_be_bytes())?;
        });
        // trun
        write_trun(w, samples)?;
    });

    // 2. 修正 data_offset
    let moof_total_size = 8 + moof_body.len() as u32;
    fixup_trun_data_offset(&mut moof_body, moof_total_size);

    // 3. 写出 moof
    w.write_all(&moof_total_size.to_be_bytes())?;
    w.write_all(b"moof")?;
    w.write_all(&moof_body)?;

    // 4. 写出 mdat
    let mdat_size = 8u32 + mdat_payload.len() as u32;
    w.write_all(&mdat_size.to_be_bytes())?;
    w.write_all(b"mdat")?;
    w.write_all(mdat_payload)?;

    Ok(())
}

// ──────────────────────────── 公共常量 ────────────────────────────

/// fMP4 视频 timescale（90kHz）
pub const VIDEO_TIMESCALE: u32 = TIMESCALE;

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_ftyp_structure() {
        let mut buf = Vec::new();
        write_ftyp(&mut buf).unwrap();
        // ftyp box: size(4) + "ftyp"(4) + major_brand(4) + minor_version(4) + 5 compatible brands(20)
        assert_eq!(buf.len(), 36);
        assert_eq!(&buf[4..8], b"ftyp");
        assert_eq!(&buf[8..12], b"isom");
    }

    #[test]
    fn test_moov_contains_expected_boxes() {
        let avcc = build_test_avcc();
        let mut buf = Vec::new();
        write_moov(&mut buf, CodecType::H264, 1920, 1080, &avcc).unwrap();

        let data = &buf;
        assert!(find_box_tag(data, b"moov").is_some());
        assert!(find_box_tag(data, b"mvhd").is_some());
        assert!(find_box_tag(data, b"trak").is_some());
        assert!(find_box_tag(data, b"tkhd").is_some());
        assert!(find_box_tag(data, b"mdia").is_some());
        assert!(find_box_tag(data, b"hdlr").is_some());
        assert!(find_box_tag(data, b"stbl").is_some());
        assert!(find_box_tag(data, b"mvex").is_some());
        assert!(find_box_tag(data, b"trex").is_some());
        assert!(find_box_tag(data, b"avcC").is_some());
        assert!(find_box_tag(data, b"avc1").is_some());
    }

    #[test]
    fn test_moov_h265() {
        let hvcc = vec![0x01, 0x01, 0x60, 0x00, 0x00, 0x00, 0x00]; // minimal hvcC
        let mut buf = Vec::new();
        write_moov(&mut buf, CodecType::H265, 3840, 2160, &hvcc).unwrap();
        assert!(find_box_tag(&buf, b"hvc1").is_some());
        assert!(find_box_tag(&buf, b"hvcC").is_some());
    }

    #[test]
    fn test_fragment_structure() {
        let samples = vec![
            SampleEntry {
                duration: 3000,
                size: 100,
                is_keyframe: true,
                composition_offset: 0,
            },
            SampleEntry {
                duration: 3000,
                size: 50,
                is_keyframe: false,
                composition_offset: 0,
            },
        ];
        let mdat_payload = vec![0u8; 150];

        let mut buf = Vec::new();
        write_fragment(&mut buf, 1, 0, &samples, &mdat_payload).unwrap();

        assert!(find_box_tag(&buf, b"moof").is_some());
        assert!(find_box_tag(&buf, b"mfhd").is_some());
        assert!(find_box_tag(&buf, b"traf").is_some());
        assert!(find_box_tag(&buf, b"tfhd").is_some());
        assert!(find_box_tag(&buf, b"tfdt").is_some());
        assert!(find_box_tag(&buf, b"trun").is_some());
        assert!(find_box_tag(&buf, b"mdat").is_some());

        // mdat size = 8 + 150 = 158
        let mdat_pos = find_box_tag(&buf, b"mdat").unwrap();
        let mdat_size = u32::from_be_bytes(buf[mdat_pos - 4..mdat_pos].try_into().unwrap());
        assert_eq!(mdat_size, 158);
    }

    fn build_test_avcc() -> Vec<u8> {
        // Minimal avcC body:
        // version=1, profile=100, compat=0, level=41, lengthSize=4,
        // numSPS=1, spsLen=4, sps=[67 64 00 29],
        // numPPS=1, ppsLen=4, pps=[68 EE 3C 80]
        vec![
            0x01, 100, 0x00, 41, 0xFF, // lengthSizeMinusOne = 3 (4 bytes) | reserved
            0xE1, // numSPS = 1 | reserved
            0x00, 0x04, 0x67, 0x64, 0x00, 0x29, // SPS
            0x01, // numPPS
            0x00, 0x04, 0x68, 0xEE, 0x3C, 0x80, // PPS
        ]
    }
}
