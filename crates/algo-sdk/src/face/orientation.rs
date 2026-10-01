//! EXIF Orientation 解析与物理像素方向归一化
//!
//! 提供无依赖、零内存拷贝的 EXIF Orientation 标签解析 (Tag 0x0112)，
//! 并支持可选对 `DynamicImage` 执行物理转正。

/// 尝试从图像原始二进制数据中解析 EXIF Orientation 标签值 (1 ~ 8)。
///
/// 若未找到 Orientation 标签、格式非标准或解析出现任何异常，安全回退到默认值 `1` (Normal，无旋转)。
pub fn parse_exif_orientation(data: &[u8]) -> u32 {
    if data.len() < 8 {
        return 1;
    }

    // 1. JPEG APP1 扫描
    if data.starts_with(&[0xFF, 0xD8]) {
        if let Some(orientation) = parse_jpeg_exif_orientation(data) {
            return orientation;
        }
    }

    // 2. 裸 TIFF 数据头检测 (II* 或 MM*)
    if data.starts_with(b"II*\0") || data.starts_with(b"MM\0*") {
        if let Some(orientation) = parse_tiff_orientation(data) {
            return orientation;
        }
    }

    // 3. PNG 的 eXIf 块直接包含 TIFF 头，不带 "Exif\0\0" 前缀。
    if let Some(orientation) = parse_png_exif_orientation(data) {
        return orientation;
    }

    // 4. 其他带 "Exif\0\0" 前缀的嵌入 EXIF 数据。
    let search_limit = data.len().min(64 * 1024);
    if let Some(pos) = find_subsequence(&data[..search_limit], b"Exif\0\0") {
        if let Some(tiff_data) = data.get(pos + 6..) {
            if let Some(orientation) = parse_tiff_orientation(tiff_data) {
                return orientation;
            }
        }
    }

    1
}

fn parse_png_exif_orientation(data: &[u8]) -> Option<u32> {
    const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if !data.starts_with(PNG_SIGNATURE) {
        return None;
    }

    let mut cursor = PNG_SIGNATURE.len();
    while cursor < data.len() {
        let header_end = cursor.checked_add(8)?;
        let header = data.get(cursor..header_end)?;
        let chunk_len = usize::try_from(u32::from_be_bytes([
            header[0], header[1], header[2], header[3],
        ]))
        .ok()?;
        let chunk_type = &header[4..8];
        let data_end = header_end.checked_add(chunk_len)?;
        let chunk_end = data_end.checked_add(4)?;
        let chunk_data = data.get(header_end..data_end)?;
        data.get(data_end..chunk_end)?;

        if chunk_type == b"eXIf" {
            let tiff_data = chunk_data.strip_prefix(b"Exif\0\0").unwrap_or(chunk_data);
            if let Some(orientation) = parse_tiff_orientation(tiff_data) {
                return Some(orientation);
            }
        }
        if chunk_type == b"IEND" {
            break;
        }
        cursor = chunk_end;
    }

    None
}

fn parse_jpeg_exif_orientation(data: &[u8]) -> Option<u32> {
    let mut cursor = 2; // 跳过 SOI (0xFF 0xD8)

    while cursor + 4 <= data.len() {
        if data[cursor] != 0xFF {
            cursor += 1;
            continue;
        }

        while cursor < data.len() && data[cursor] == 0xFF {
            cursor += 1;
        }
        if cursor >= data.len() {
            break;
        }

        let marker = data[cursor];
        cursor += 1;

        if marker == 0xDA || marker == 0xD9 {
            break;
        }

        if (0xD0..=0xD7).contains(&marker) || marker == 0x01 || marker == 0x00 {
            continue;
        }

        if cursor + 2 > data.len() {
            break;
        }
        let length = u16::from_be_bytes([data[cursor], data[cursor + 1]]) as usize;
        if length < 2 {
            break;
        }
        let payload_start = cursor + 2;
        let payload_end = cursor + length;
        if payload_end > data.len() {
            break;
        }

        if marker == 0xE1 {
            let app1_payload = &data[payload_start..payload_end];
            if app1_payload.starts_with(b"Exif\0\0") {
                if let Some(orientation) = parse_tiff_orientation(&app1_payload[6..]) {
                    return Some(orientation);
                }
            }
        }

        cursor = payload_end;
    }

    None
}

fn parse_tiff_orientation(tiff: &[u8]) -> Option<u32> {
    if tiff.len() < 8 {
        return None;
    }

    let is_le = match &tiff[0..2] {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };

    let read_u16 = |offset: usize| -> Option<u16> {
        let bytes = tiff.get(offset..offset + 2)?;
        Some(if is_le {
            u16::from_le_bytes([bytes[0], bytes[1]])
        } else {
            u16::from_be_bytes([bytes[0], bytes[1]])
        })
    };

    let read_u32 = |offset: usize| -> Option<u32> {
        let bytes = tiff.get(offset..offset + 4)?;
        Some(if is_le {
            u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
        } else {
            u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
        })
    };

    if read_u16(2)? != 42 {
        return None;
    }

    let ifd0_offset = read_u32(4)? as usize;
    if ifd0_offset < 8 || ifd0_offset + 2 > tiff.len() {
        return None;
    }

    let entry_count = read_u16(ifd0_offset)? as usize;
    let max_entries = entry_count.min(256);
    let mut entry_offset = ifd0_offset + 2;

    for _ in 0..max_entries {
        if entry_offset + 12 > tiff.len() {
            break;
        }

        let tag = read_u16(entry_offset)?;
        if tag == 0x0112 {
            let val = read_u16(entry_offset + 8)? as u32;
            if (1..=8).contains(&val) {
                return Some(val);
            }
        }

        entry_offset += 12;
    }

    None
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(any(feature = "image", feature = "testing-image", test))]
/// 根据 EXIF Orientation 标签值将 DynamicImage 执行物理旋转/翻转至标准正立方向 (Orientation 1)
pub fn apply_orientation(img: image::DynamicImage, orientation: u32) -> image::DynamicImage {
    match orientation {
        2 => img.fliph(),
        3 => img.rotate180(),
        4 => img.flipv(),
        5 => img.rotate90().fliph(),
        6 => img.rotate90(),
        7 => img.rotate270().fliph(),
        8 => img.rotate270(),
        _ => img,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造合成的包含 APP1 EXIF Orientation 的最小合法 JPEG 字节流
    fn build_synthetic_exif_jpeg(orientation: u16, little_endian: bool) -> Vec<u8> {
        let mut jpeg = vec![0xFF, 0xD8]; // SOI

        let mut tiff = Vec::new();
        if little_endian {
            tiff.extend_from_slice(b"II");
            tiff.extend_from_slice(&42u16.to_le_bytes());
            tiff.extend_from_slice(&8u32.to_le_bytes()); // IFD0 紧随其后 (offset 8)
            tiff.extend_from_slice(&1u16.to_le_bytes()); // 1 entry
            tiff.extend_from_slice(&0x0112u16.to_le_bytes());
            tiff.extend_from_slice(&3u16.to_le_bytes()); // SHORT
            tiff.extend_from_slice(&1u32.to_le_bytes()); // count
            tiff.extend_from_slice(&orientation.to_le_bytes());
            tiff.extend_from_slice(&[0, 0]); // padding to 4 bytes
        } else {
            tiff.extend_from_slice(b"MM");
            tiff.extend_from_slice(&42u16.to_be_bytes());
            tiff.extend_from_slice(&8u32.to_be_bytes());
            tiff.extend_from_slice(&1u16.to_be_bytes());
            tiff.extend_from_slice(&0x0112u16.to_be_bytes());
            tiff.extend_from_slice(&3u16.to_be_bytes());
            tiff.extend_from_slice(&1u32.to_be_bytes());
            tiff.extend_from_slice(&orientation.to_be_bytes());
            tiff.extend_from_slice(&[0, 0]);
        }

        // APP1 payload: "Exif\0\0" + tiff
        let mut app1_payload = Vec::new();
        app1_payload.extend_from_slice(b"Exif\0\0");
        app1_payload.extend_from_slice(&tiff);

        let app1_len = (app1_payload.len() + 2) as u16;
        jpeg.push(0xFF);
        jpeg.push(0xE1);
        jpeg.extend_from_slice(&app1_len.to_be_bytes());
        jpeg.extend_from_slice(&app1_payload);

        jpeg.extend_from_slice(&[0xFF, 0xD9]); // EOI
        jpeg
    }

    #[test]
    fn test_parse_synthetic_exif_little_endian() {
        for orient in 1..=8 {
            let data = build_synthetic_exif_jpeg(orient, true);
            assert_eq!(parse_exif_orientation(&data), orient as u32);
        }
    }

    #[test]
    fn test_parse_synthetic_exif_big_endian() {
        for orient in 1..=8 {
            let data = build_synthetic_exif_jpeg(orient, false);
            assert_eq!(parse_exif_orientation(&data), orient as u32);
        }
    }

    #[test]
    fn test_corrupted_data_does_not_panic() {
        assert_eq!(parse_exif_orientation(&[]), 1);
        assert_eq!(parse_exif_orientation(&[0xFF, 0xD8]), 1);
        assert_eq!(parse_exif_orientation(&[0xFF, 0xD8, 0xFF, 0xE1]), 1);
        assert_eq!(
            parse_exif_orientation(&[0xFF, 0xD8, 0xFF, 0xE1, 0x00, 0x10]),
            1
        );
        assert_eq!(parse_exif_orientation(&[0x00; 100]), 1);
    }

    #[test]
    fn test_parse_png_exif_orientation() {
        use image::ImageEncoder;

        let mut tiff = Vec::new();
        tiff.extend_from_slice(b"II");
        tiff.extend_from_slice(&42u16.to_le_bytes());
        tiff.extend_from_slice(&8u32.to_le_bytes());
        tiff.extend_from_slice(&1u16.to_le_bytes());
        tiff.extend_from_slice(&0x0112u16.to_le_bytes());
        tiff.extend_from_slice(&3u16.to_le_bytes());
        tiff.extend_from_slice(&1u32.to_le_bytes());
        tiff.extend_from_slice(&6u16.to_le_bytes());
        tiff.extend_from_slice(&[0, 0]);

        let pixels = image::RgbImage::from_pixel(40, 20, image::Rgb([128, 64, 32]));
        let mut png = Vec::new();
        let mut encoder = image::codecs::png::PngEncoder::new(&mut png);
        encoder
            .set_exif_metadata(tiff)
            .expect("PNG encoder should accept EXIF metadata");
        encoder
            .write_image(&pixels, 40, 20, image::ExtendedColorType::Rgb8)
            .expect("synthetic PNG encoding should succeed");

        assert_eq!(parse_exif_orientation(&png), 6);
    }

    #[test]
    fn test_apply_orientation_changes_dimensions_for_90_degree() {
        let rgb = image::RgbImage::from_pixel(10, 20, image::Rgb([255, 0, 0]));
        let dynamic = image::DynamicImage::ImageRgb8(rgb);

        let rotated = apply_orientation(dynamic, 6);
        assert_eq!(rotated.width(), 20);
        assert_eq!(rotated.height(), 10);
    }

    #[test]
    fn test_apply_orientation_identity_for_normal() {
        let rgb = image::RgbImage::from_pixel(10, 20, image::Rgb([255, 0, 0]));
        let dynamic = image::DynamicImage::ImageRgb8(rgb);

        let normal = apply_orientation(dynamic, 1);
        assert_eq!(normal.width(), 10);
        assert_eq!(normal.height(), 20);
    }
}
