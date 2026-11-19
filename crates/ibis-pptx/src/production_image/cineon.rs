// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! SDR preview of Kodak Cineon 4.5 printing-density RGB scans.

use image::{Rgb, RgbImage};

const HEADER_SIZE: usize = 1024;
const MAGIC: u32 = 0x802A5FD7;

pub(super) fn decode(bytes: &[u8]) -> Result<RgbImage, &'static str> {
    let header = bytes.get(..HEADER_SIZE).ok_or("Incomplete Cineon header")?;
    let read_word = match header[..4].try_into().unwrap() {
        magic if u32::from_be_bytes(magic) == MAGIC => u32::from_be_bytes,
        magic if u32::from_le_bytes(magic) == MAGIC => u32::from_le_bytes,
        _ => return Err("Invalid Cineon signature"),
    };
    let field = |at| read_word(header[at..at + 4].try_into().unwrap());
    let orientation = header[192];
    if orientation > 3 {
        return Err("Unsupported Cineon scan orientation");
    }
    if header[193] != 3
        || header[680] != 0
        || !matches!(header[681], 5 | 0x85)
        || header[682..684] != [0, 0]
    {
        return Err("Cineon preview requires unsigned, positive, pixel-interleaved RGB");
    }
    let width = field(200);
    let height = field(204);
    if width == 0
        || height == 0
        || width > 8192
        || height > 8192
        || u64::from(width) * u64::from(height) > super::MAX_PIXELS
    {
        return Err("Cineon image dimensions exceed the preview limits");
    }
    let descriptors = [196, 224, 252];
    // ImageMagick's writer leaves all three designators zero for RGB files.
    let designators = descriptors.map(|at| [header[at], header[at + 1]]);
    if !matches!(
        designators,
        [[0, 1], [0, 2], [0, 3]] | [[0, 0], [0, 0], [0, 0]]
    ) {
        return Err("Unsupported Cineon channel designators");
    }
    for descriptor in descriptors {
        if header[descriptor + 2] != 10
            || field(descriptor + 4) != width
            || field(descriptor + 8) != height
        {
            return Err("Cineon preview requires matching 10-bit printing-density RGB channels");
        }
    }
    let offset = field(4) as usize;
    let header_end = [8, 12, 16]
        .into_iter()
        .try_fold(0usize, |total, at| total.checked_add(field(at) as usize))
        .ok_or("Invalid Cineon header lengths")?;
    if offset < header_end.max(HEADER_SIZE) {
        return Err("Invalid Cineon image offset");
    }
    let stride = (width as usize * 4)
        .checked_add(field(684) as usize)
        .ok_or("Invalid Cineon row padding")?;
    let end = stride
        .checked_mul(height as usize)
        .and_then(|size| offset.checked_add(size))
        .ok_or("Invalid Cineon image size")?;
    let pixels = bytes
        .get(offset..end)
        .ok_or("Truncated Cineon pixel data")?;

    // Kodak's default black and white references, mapped through linear exposure to sRGB.
    let exposure = |code: f32| 10.0_f32.powf((code - 685.0) * (0.002 / 0.6));
    let black = exposure(95.0);
    let lookup: [u8; 1024] = std::array::from_fn(|code| {
        super::encode_srgb(((exposure(code as f32) - black) / (1.0 - black)).clamp(0.0, 1.0))
    });
    Ok(RgbImage::from_fn(width, height, |x, y| {
        let x = if orientation & 2 == 0 {
            x
        } else {
            width - 1 - x
        };
        let y = if orientation & 1 == 0 {
            y
        } else {
            height - 1 - y
        };
        let start = y as usize * stride + x as usize * 4;
        let packed = read_word(pixels[start..start + 4].try_into().unwrap());
        Rgb([22, 12, 2].map(|bit| lookup[((packed >> bit) & 1023) as usize]))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(little_endian: bool, orientation: u8) -> Vec<u8> {
        let mut bytes = vec![0; 1048];
        let mut set_word = |offset, value: u32| {
            let word = if little_endian {
                value.to_le_bytes()
            } else {
                value.to_be_bytes()
            };
            bytes[offset..offset + 4].copy_from_slice(&word);
        };
        for (offset, value) in [
            (0, MAGIC),
            (4, 1024),
            (8, 1024),
            (20, 1048),
            (684, 4),
            (1024, (685 << 22) | (95 << 12) | (95 << 2)),
            (1028, 685 << 12),
            (1036, 685 << 2),
            (1040, (685 << 22) | (685 << 12) | (685 << 2)),
        ] {
            set_word(offset, value);
        }
        for descriptor in [196, 224, 252] {
            set_word(descriptor + 4, 2);
            set_word(descriptor + 8, 2);
        }
        bytes[192] = orientation;
        bytes[193] = 3;
        bytes[681] = 5;
        for (index, descriptor) in [196, 224, 252].into_iter().enumerate() {
            bytes[descriptor + 1] = index as u8 + 1;
            bytes[descriptor + 2] = 10;
        }
        bytes
    }

    #[test]
    fn previews_packed_rgb_in_both_byte_orders() {
        for little_endian in [false, true] {
            let bytes = fixture(little_endian, 3);
            let rgb = decode(&bytes).unwrap();
            assert_eq!(
                rgb.into_raw(),
                [255, 255, 255, 0, 0, 255, 0, 255, 0, 255, 0, 0]
            );
            assert!(super::super::open(Some("CIN"), &bytes).unwrap().is_some());
        }
    }

    #[test]
    fn accepts_rgb_with_omitted_channel_designators() {
        let mut bytes = fixture(false, 0);
        let expected = decode(&bytes).unwrap();
        for at in [197, 225, 253] {
            bytes[at] = 0;
        }
        assert_eq!(decode(&bytes).unwrap(), expected);
    }

    #[test]
    fn rejects_truncated_pixels_and_unsupported_layouts() {
        let mut bytes = fixture(false, 0);
        assert!(decode(&bytes[..1043]).is_err());
        bytes[680] = 2;
        assert!(decode(&bytes).is_err());
    }
}
