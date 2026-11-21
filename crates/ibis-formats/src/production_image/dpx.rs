// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! SDR preview of a single unsigned, filled 10-bit RGB element (SMPTE ST 268).

use image::{Rgb, RgbImage};

const GENERIC_HEADER_SIZE: usize = 1664;
const MAX_CODE: u32 = 1023;

use super::{DPX_NEEDS_COLOR, DpxColor};

pub(super) fn decode(
    bytes: &[u8],
    selected_color: Option<DpxColor>,
) -> Result<RgbImage, &'static str> {
    let header = bytes
        .get(..GENERIC_HEADER_SIZE)
        .ok_or("Incomplete DPX header")?;
    let big_endian = match &header[..4] {
        b"SDPX" => true,
        b"XPDS" => false,
        _ => return Err("Invalid DPX signature"),
    };
    let word = |data: [u8; 4]| {
        if big_endian {
            u32::from_be_bytes(data)
        } else {
            u32::from_le_bytes(data)
        }
    };
    let u32_at = |offset| word(header[offset..offset + 4].try_into().unwrap());
    let u16_at = |offset| {
        let data = header[offset..offset + 2].try_into().unwrap();
        if big_endian {
            u16::from_be_bytes(data)
        } else {
            u16::from_le_bytes(data)
        }
    };

    let orientation = u16_at(768);
    if orientation > 3 {
        return Err("Unsupported DPX scan orientation");
    }
    if u32_at(660) != u32::MAX {
        return Err("Encrypted DPX images cannot be previewed");
    }
    if u16_at(770) != 1
        || u32_at(780) != 0
        || header[800] != 50
        || header[803] != 10
        || u16_at(806) != 0
    {
        return Err("DPX preview requires one uncompressed unsigned 10-bit RGB element");
    }
    let shift = match u16_at(804) {
        1 => 2, // Method A places the two padding bits at the low end.
        2 => 0, // Method B places them at the high end.
        _ => return Err("Unsupported DPX packing"),
    };

    let width = u32_at(772);
    let height = u32_at(776);
    if width == 0
        || height == 0
        || width > 8192
        || height > 8192
        || u64::from(width) * u64::from(height) > super::MAX_PIXELS
    {
        return Err("DPX image dimensions exceed the preview limits");
    }
    let offset = u32_at(808) as usize;
    let header_end = [24, 28, 32]
        .into_iter()
        .try_fold(0usize, |sum, field| sum.checked_add(u32_at(field) as usize))
        .ok_or("Invalid DPX header lengths")?;
    if offset < header_end.max(GENERIC_HEADER_SIZE)
        || offset < u32_at(4) as usize
        || offset % 4 != 0
    {
        return Err("Invalid DPX image offset");
    }
    let pixels_per_row = width as usize * 4;
    let stride = pixels_per_row
        .checked_add(u32_at(812) as usize)
        .ok_or("Invalid DPX row padding")?;
    let end = stride
        .checked_mul(height as usize - 1)
        .and_then(|last_row| offset.checked_add(last_row))
        .and_then(|last_row| last_row.checked_add(pixels_per_row))
        .ok_or("Invalid DPX image size")?;
    let pixels = bytes.get(offset..end).ok_or("Truncated DPX pixel data")?;

    let (tagged_color, can_choose) = match (header[801], header[802]) {
        (2, 6) => (Some(DpxColor::LinearRec709), false),
        (6, 6) => (Some(DpxColor::VideoRec709), false),
        (0 | 255, 0 | 255 | 6) | (2 | 6, 0 | 255) => (None, true),
        _ => return Err("DPX preview requires linear or video Rec.709 color"),
    };
    let range = match (u32_at(784), u32_at(792)) {
        (u32::MAX, u32::MAX) => None,
        (low, high) if low < high && high <= MAX_CODE => Some((low, high)),
        _ => return Err("DPX preview requires a valid reference code range"),
    };
    let color = tagged_color.or(selected_color).ok_or(DPX_NEEDS_COLOR)?;
    let (low, high) = match (range, color) {
        (Some(range), _) => range,
        (None, DpxColor::LinearRec709) => (0, MAX_CODE),
        (None, DpxColor::VideoRec709) if can_choose => (64, 940),
        _ => return Err("DPX preview requires a valid reference code range"),
    };

    let lookup: [u8; 1024] = std::array::from_fn(|code| {
        let signal = ((code as f32 - low as f32) / (high - low) as f32).clamp(0.0, 1.0);
        let linear = if color == DpxColor::LinearRec709 {
            signal
        } else if signal < 0.081 {
            signal / 4.5
        } else {
            ((signal + 0.099) / 1.099).powf(1.0 / 0.45)
        };
        super::encode_srgb(linear)
    });
    Ok(RgbImage::from_fn(width, height, |x, y| {
        let x = if orientation & 1 == 0 {
            x
        } else {
            width - 1 - x
        };
        let y = if orientation & 2 == 0 {
            y
        } else {
            height - 1 - y
        };
        let start = y as usize * stride + x as usize * 4;
        let rgb = word(pixels[start..start + 4].try_into().unwrap()) >> shift;
        Rgb([20, 10, 0].map(|bit| lookup[((rgb >> bit) & MAX_CODE) as usize]))
    }))
}

#[cfg(test)]
mod tests {
    fn fixture(little_endian: bool, method: u16) -> Vec<u8> {
        let mut bytes = vec![0; 1664];
        let u32_bytes = |value: u32| {
            if little_endian {
                value.to_le_bytes()
            } else {
                value.to_be_bytes()
            }
        };
        let u16_bytes = |value: u16| {
            if little_endian {
                value.to_le_bytes()
            } else {
                value.to_be_bytes()
            }
        };
        bytes[..4].copy_from_slice(if little_endian { b"XPDS" } else { b"SDPX" });
        bytes[8..12].copy_from_slice(b"V2.0");
        for (offset, value) in [
            (4, 1664),
            (16, 1688),
            (24, 1664),
            (660, u32::MAX),
            (772, 2),
            (776, 2),
            (792, 1023),
            (808, 1664),
            (812, 4),
        ] {
            bytes[offset..offset + 4].copy_from_slice(&u32_bytes(value));
        }
        for (offset, value) in [(768, 3), (770, 1), (804, method)] {
            bytes[offset..offset + 2].copy_from_slice(&u16_bytes(value));
        }
        bytes[800..804].copy_from_slice(&[50, 2, 6, 10]);
        let shift = if method == 1 { 2 } else { 0 };
        for word in [
            1023 << 20,
            1023 << 10,
            0,
            1023,
            (1023 << 20) | (512 << 10),
            0,
        ] {
            bytes.extend(u32_bytes(word << shift));
        }
        bytes
    }

    #[test]
    fn opens_dpx_with_both_byte_orders_and_filling_methods() {
        for (little_endian, method) in [(false, 1), (true, 2)] {
            let bytes = fixture(little_endian, method);
            let opened = super::super::open(Some("DPX"), &bytes).unwrap().unwrap();
            let png = image::load_from_memory(&opened.presentation.media.last().unwrap().data)
                .unwrap()
                .to_rgb8();
            assert_eq!(png.dimensions(), (2, 2));
            assert_eq!(
                png.into_raw(),
                [255, 188, 0, 0, 0, 255, 0, 255, 0, 255, 0, 0]
            );
        }
    }

    #[test]
    fn normalizes_video_code_range_and_converts_rec709_to_srgb() {
        let mut bytes = fixture(false, 1);
        bytes[768..770].copy_from_slice(&0u16.to_be_bytes());
        bytes[801] = 6;
        bytes[784..788].copy_from_slice(&64u32.to_be_bytes());
        bytes[792..796].copy_from_slice(&940u32.to_be_bytes());
        let word = (940u32 << 22) | (502 << 12) | (64 << 2);
        bytes[1664..1668].copy_from_slice(&word.to_be_bytes());
        assert_eq!(
            super::decode(&bytes, None).unwrap().get_pixel(0, 0).0,
            [255, 139, 0]
        );
    }

    #[test]
    fn chooses_color_for_missing_tags_without_overriding_valid_tags() {
        use super::{DPX_NEEDS_COLOR, DpxColor};

        let mut bytes = fixture(false, 1);
        let tagged = super::decode(&bytes, Some(DpxColor::VideoRec709)).unwrap();
        assert_eq!(tagged, super::decode(&bytes, None).unwrap());
        bytes[801..803].fill(0);
        bytes[784..788].fill(255);
        bytes[792..796].fill(255);
        assert_eq!(super::decode(&bytes, None).unwrap_err(), DPX_NEEDS_COLOR);
        let linear = super::decode(&bytes, Some(DpxColor::LinearRec709)).unwrap();
        let video = super::decode(&bytes, Some(DpxColor::VideoRec709)).unwrap();
        assert_eq!(linear, tagged);
        assert_eq!(video.get_pixel(0, 0).0, [255, 142, 0]);
        bytes.truncate(bytes.len() - 5);
        assert!(
            super::decode(&bytes, None)
                .unwrap_err()
                .contains("Truncated")
        );
    }

    #[test]
    fn exported_docx_contains_the_selected_preview_pixels() {
        let mut bytes = fixture(false, 1);
        bytes[801..803].fill(0);
        for color in [super::DpxColor::LinearRec709, super::DpxColor::VideoRec709] {
            let (opened, _) = super::super::open_with_dpx_color(Some("dpx"), &bytes, Some(color))
                .unwrap()
                .unwrap();
            let docx = opened
                .source
                .save(&opened.presentation, Default::default())
                .unwrap();
            let reopened = crate::open(docx).unwrap();
            let media = reopened.presentation.media.last().unwrap();
            let exported = image::load_from_memory(&media.data).unwrap().to_rgb8();
            assert_eq!(exported, super::decode(&bytes, Some(color)).unwrap());
        }
    }

    #[test]
    fn rejects_unimplemented_color_and_truncated_rows() {
        let mut bytes = fixture(false, 1);
        bytes[801] = 1;
        assert!(super::decode(&bytes, None).unwrap_err().contains("Rec.709"));
        bytes[801] = 2;
        bytes.truncate(bytes.len() - 5);
        assert!(
            super::decode(&bytes, None)
                .unwrap_err()
                .contains("Truncated")
        );
    }
}
