// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use ibis_doc_model::EmuSize;
use image::{ImageEncoder, Rgba, RgbaImage};
use tiny_skia::Transform;

use super::{appearance::word, wpg_drawing::Drawing};

const MAX_PIXELS: usize = 16 * 1024 * 1024;

pub(super) fn split<'a>(data: &mut &'a [u8], size: usize) -> Option<&'a [u8]> {
    let (head, rest) = data.split_at_checked(size)?;
    *data = rest;
    Some(head)
}

fn byte(data: &mut &[u8]) -> Option<u8> {
    Some(split(data, 1)?[0])
}

fn number(data: &mut &[u8]) -> Option<usize> {
    let value = byte(data)?;
    if value != 0xff {
        return Some(usize::from(value));
    }
    let high = usize::from(word(split(data, 2)?, 0)?);
    if high & 0x8000 == 0 {
        return Some(high);
    }
    Some(((high & 0x7fff) << 16) | usize::from(word(split(data, 2)?, 0)?))
}

pub(super) fn decode(data: &[u8]) -> Option<(Vec<u8>, EmuSize)> {
    if !data.starts_with(b"\xffWPC") || data.get(9) != Some(&0x16) || data.get(12..14)? != [0, 0] {
        return None;
    }
    let offset = u32::from_le_bytes(data.get(4..8)?.try_into().ok()?) as usize;
    let version = *data.get(10)?;
    if offset < 16 || !matches!(version, 1 | 2) {
        return None;
    }
    let mut records = data.get(offset..)?;
    let mut palette = default_palette(version);
    let mut canvas = None;
    let mut bitmap_box = None;
    let mut result = None;
    let mut drawing = Drawing::default();
    let mut ended = false;
    let mut scopes: Vec<(usize, Transform, bool)> = Vec::new();
    while !records.is_empty() {
        if version == 2 {
            byte(&mut records)?; // record class
        }
        let kind = byte(&mut records)?;
        let children = if version == 2 {
            number(&mut records)?
        } else {
            0
        };
        while scopes
            .last()
            .is_some_and(|&(remaining, _, _)| remaining == 0)
        {
            if scopes.pop()?.2 {
                drawing.end_compound(canvas?)?;
            }
        }
        let parent = if let Some((remaining, transform, _)) = scopes.last_mut() {
            *remaining -= 1;
            *transform
        } else {
            Transform::identity()
        };
        let len = number(&mut records)?;
        let record = split(&mut records, len)?;
        let mut child_transform = parent;
        match (version, kind) {
            (1, 0x0f) => {}
            (2, 1) => canvas = Some(Canvas::read(record)?),
            (1, 0x10) | (2, 2) => {
                if !scopes.is_empty() || children != 0 {
                    return None;
                }
                ended = true;
                break;
            }
            (1, 0x0e) | (2, 0x0c) => {
                let first = usize::from(word(record, 0)?);
                let count = usize::from(word(record, 2)?);
                let size = if version == 1 { 3 } else { 4 };
                for (dst, rgb) in palette
                    .get_mut(first..first + count)?
                    .iter_mut()
                    .zip(record.get(4..4 + size * count)?.chunks_exact(size))
                {
                    *dst = Some(Rgba([
                        rgb[0],
                        rgb[1],
                        rgb[2],
                        if version == 1 { 255 } else { 255 - rgb[3] },
                    ]));
                }
            }
            (2, 0x1b) => {
                let (bounds, transform) = canvas?.bitmap_box(record)?;
                bitmap_box = Some((bounds, parent.pre_concat(transform)));
            }
            (2, 0x1a) => {
                if children == 0 {
                    return None;
                }
                child_transform = drawing.begin_compound(canvas?, record, parent)?;
            }
            (2, 0x20) => {
                if !record.is_empty() {
                    let (_, transform, _) = canvas?.object(record)?;
                    child_transform = parent.pre_concat(transform);
                }
            }
            (1, 0x0b | 0x14) => {
                if result.is_some() {
                    return None;
                }
                result = Some(bitmap_v1(record, kind == 0x14, &palette)?);
            }
            (2, 0x0e) => {
                let (bounds, transform) = bitmap_box.take()?;
                drawing.bitmap(canvas?, bitmap_v2(record, &palette)?, bounds, transform)?
            }
            (2, 0x15 | 0x17..=0x19 | 0x25 | 0x29 | 0x2b | 0x2c | 0x31) => {
                drawing.record(canvas?, kind, record, parent)?;
            }
            (1, 0x02..=0x0a | 0x11..=0x13 | 0x15..=0x1b)
            | (
                2,
                0x03..=0x06
                | 0x0d
                | 0x0f..=0x12
                | 0x16
                | 0x1c..=0x1f
                | 0x21..=0x22
                | 0x26
                | 0x2a
                | 0x2d..=0x30
                | 0x32
                | 0x35..=0x37
                | 0x3a..=0x3f,
            ) => return None,
            _ => {}
        }
        if children != 0 {
            if scopes.len() >= 64 || !child_transform.is_finite() {
                return None;
            }
            scopes.push((children, child_transform, kind == 0x1a));
        }
    }
    if !ended {
        return None;
    }
    let (pixels, size) = if version == 2 {
        drawing.finish(canvas?)?
    } else {
        result?
    };
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(
            pixels.as_raw(),
            pixels.width(),
            pixels.height(),
            image::ExtendedColorType::Rgba8,
        )
        .ok()?;
    Some((png, size))
}

// The two versions assign different colors to the low palette indices.
fn default_palette(version: u8) -> [Option<Rgba<u8>>; 256] {
    let colors: [u32; 16] = if version == 1 {
        [
            0x000000, 0x00007f, 0x007f00, 0x007f7f, 0x7f0000, 0x7f007f, 0x7f3f00, 0x7f7f7f,
            0xc0c0c0, 0x0000ff, 0x00ff00, 0x00ffff, 0xff0000, 0xff00ff, 0xffff00, 0xffffff,
        ]
    } else {
        [
            0x000000, 0xffffff, 0x7f7f7f, 0xbfbfbf, 0x00007f, 0x007f00, 0x007f7f, 0x7f0000,
            0x7f007f, 0x7f7f00, 0x0000ff, 0x00ff00, 0x00ffff, 0xff0000, 0xff00ff, 0xffff00,
        ]
    };
    let mut palette = [None; 256];
    for (entry, rgb) in palette.iter_mut().zip(colors) {
        let [_, r, g, b] = rgb.to_be_bytes();
        *entry = Some(Rgba([r, g, b, 255]));
    }
    if version == 2 {
        for (index, entry) in palette[16..32].iter_mut().enumerate() {
            let shade = index as u8 * 17;
            *entry = Some(Rgba([shade, shade, shade, 255]));
        }
    }
    palette
}

pub(super) type Bounds = [i64; 4];

#[derive(Clone, Copy)]
pub(super) struct Canvas {
    pub(super) bounds: Bounds,
    dpi_x: i64,
    dpi_y: i64,
    pub(super) coordinate_bytes: usize,
}

impl Canvas {
    fn read(record: &[u8]) -> Option<Self> {
        let coordinate_bytes = match record.get(4)? {
            0 => 2,
            1 => 4,
            _ => return None,
        };
        let bounds = bounds(record.get(5 + 4 * coordinate_bytes..)?, coordinate_bytes)?;
        let (x, y) = (word(record, 0)?, word(record, 2)?);
        let (x, y) = if x != 0 && y != 0 {
            (x, y)
        } else {
            (1200, 1200)
        };
        Some(Self {
            bounds,
            dpi_x: i64::from(x),
            dpi_y: i64::from(y),
            coordinate_bytes,
        })
    }

    pub(super) fn object(self, record: &[u8]) -> Option<(u16, Transform, &[u8])> {
        let flags = word(record, 0)?;
        if flags & 1 != 0 {
            return None; // Perspective taper requires a projective transform.
        }
        let mut data = record.get(2..)?;
        if flags & 0x80 != 0 {
            split(&mut data, 4)?;
        }
        if flags & 0x20 != 0 {
            let id = word(split(&mut data, 2)?, 0)?;
            if id & 0x8000 != 0 {
                split(&mut data, 2)?;
            }
        }
        if flags & 0x10 != 0 {
            // The angle is also stored for editors. Its sine/cosine terms
            // below already incorporate scaling and are authoritative.
            split(&mut data, 4)?;
        }
        fn signed(data: &mut &[u8]) -> Option<f32> {
            Some(i32::from_le_bytes(split(data, 4)?.try_into().ok()?) as f32)
        }
        let (sx, sy) = if flags & 0x18 != 0 {
            (signed(&mut data)? / 65536.0, signed(&mut data)? / 65536.0)
        } else {
            (1.0, 1.0)
        };
        let (kx, ky) = if flags & 0x14 != 0 {
            (signed(&mut data)? / 65536.0, signed(&mut data)? / 65536.0)
        } else {
            (0.0, 0.0)
        };
        let mut offset = || {
            let fraction = f32::from(word(split(&mut data, 2)?, 0)?) / 65536.0;
            Some(signed(&mut data)? + fraction)
        };
        let (x, y) = if flags & 2 != 0 {
            (offset()?, offset()?)
        } else {
            (0.0, 0.0)
        };
        Some((flags, Transform::from_row(sx, ky, kx, sy, x, y), data))
    }

    fn bitmap_box(self, record: &[u8]) -> Option<(Bounds, Transform)> {
        let (_, transform, data) = self.object(record)?;
        Some((bounds(data, self.coordinate_bytes)?, transform))
    }

    pub(super) fn size(self) -> Option<EmuSize> {
        let [left, bottom, right, top] = self.bounds;
        let dp = if self.coordinate_bytes == 4 { 65536 } else { 1 };
        let size = EmuSize::new(
            (right - left) * 914400 / (self.dpi_x * dp),
            (top - bottom) * 914400 / (self.dpi_y * dp),
        );
        (size.width > 0 && size.height > 0).then_some(size)
    }

    pub(super) fn pad(self, bitmap: RgbaImage, bounds: Bounds) -> Option<(RgbaImage, EmuSize)> {
        let [left, bottom, right, top] = self.bounds;
        let [x0, y0, x1, y1] = bounds;
        if x0 < left || x1 > right || y0 < bottom || y1 > top {
            return None;
        }
        let size = self.size()?;
        if self.bounds == bounds {
            return Some((bitmap, size));
        }
        let scale = |offset, pixels, extent| {
            u32::try_from((offset * i64::from(pixels) + extent / 2) / extent).ok()
        };
        let width = scale(right - left, bitmap.width(), x1 - x0)?;
        let height = scale(top - bottom, bitmap.height(), y1 - y0)?;
        limit(width as usize, height as usize)?;
        let x = scale(x0 - left, bitmap.width(), x1 - x0)?;
        let y = scale(top - y1, bitmap.height(), y1 - y0)?;
        let mut pixels = RgbaImage::new(width, height);
        image::imageops::replace(&mut pixels, &bitmap, i64::from(x), i64::from(y));
        Some((pixels, size))
    }
}

fn bounds(data: &[u8], size: usize) -> Option<Bounds> {
    let coordinate = |index| {
        let bytes = data.get(index * size..(index + 1) * size)?;
        Some(if size == 2 {
            i64::from(i16::from_le_bytes(bytes.try_into().ok()?))
        } else {
            i64::from(i32::from_le_bytes(bytes.try_into().ok()?))
        })
    };
    let [x0, y0, x1, y1] = [
        coordinate(0)?,
        coordinate(1)?,
        coordinate(2)?,
        coordinate(3)?,
    ];
    (x0 != x1 && y0 != y1).then_some([x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1)])
}

pub(super) fn limit(width: usize, height: usize) -> Option<usize> {
    let pixels = width.checked_mul(height)?;
    (pixels > 0 && pixels <= MAX_PIXELS).then_some(pixels)
}

fn bitmap_v1(
    record: &[u8],
    positioned: bool,
    palette: &[Option<Rgba<u8>>; 256],
) -> Option<(RgbaImage, EmuSize)> {
    // A Type 2 bitmap additionally carries rotation and a placement rectangle.
    // Support the unrotated Type 1 representation first, since it has no canvas offset.
    if positioned {
        return None;
    }
    let width = usize::from(word(record, 0)?);
    let height = usize::from(word(record, 2)?);
    let depth = usize::from(word(record, 4)?);
    limit(width, height)?;
    if !matches!(depth, 1 | 2 | 4 | 8) {
        return None;
    }
    let mut data = record.get(10..)?;
    let stride = (width * depth).div_ceil(8);
    let end = stride * height;
    let mut raw = Vec::new();
    while raw.len() < end {
        let op = byte(&mut data)?;
        match op {
            0 => repeat_row(&mut raw, stride, usize::from(byte(&mut data)?), end)?,
            0x80 => run(&mut raw, &[255], usize::from(byte(&mut data)?), end)?,
            0x81..=0xff => run(&mut raw, &[byte(&mut data)?], usize::from(op & 0x7f), end)?,
            _ => raw.extend_from_slice(split(&mut data, usize::from(op).min(end - raw.len()))?),
        }
    }
    let mut monochrome = [None; 256];
    monochrome[0] = Some(Rgba([0, 0, 0, 255]));
    monochrome[1] = Some(Rgba([255; 4]));
    let pixels = indexed(
        &raw,
        width,
        height,
        depth,
        if depth == 1 { &monochrome } else { palette },
    )?;
    let dpi = |at| word(record, at).map(|d| if d == 0 { 72 } else { i64::from(d) });
    Some((
        pixels,
        EmuSize::new(
            width as i64 * 914400 / dpi(6)?,
            height as i64 * 914400 / dpi(8)?,
        ),
    ))
}

fn bitmap_v2(record: &[u8], palette: &[Option<Rgba<u8>>; 256]) -> Option<RgbaImage> {
    let width = usize::from(word(record, 0)?);
    let height = usize::from(word(record, 2)?);
    limit(width, height)?;
    let format = *record.get(4)?;
    let depth = match format {
        1..=4 => 1 << (format - 1),
        8 => 24,
        9 => 32,
        12 => 8,
        _ => return None,
    };
    let stride = (width * depth).div_ceil(8);
    let total = stride * height;
    let data = record.get(6..)?;
    let expanded;
    let raw = match record[5] {
        0 => data.get(..total)?,
        1 => {
            expanded = expand_v2(data, stride, total)?;
            &expanded
        }
        _ => return None,
    };
    if format <= 4 {
        return indexed(raw, width, height, depth, palette);
    }
    let mut pixels = RgbaImage::new(width as u32, height as u32);
    for (pixel, sample) in pixels.pixels_mut().zip(raw.chunks_exact(depth / 8)) {
        *pixel = if format == 12 {
            Rgba([sample[0], sample[0], sample[0], 255])
        } else {
            Rgba([
                sample[0],
                sample[1],
                sample[2],
                if format == 8 { 255 } else { sample[3] },
            ])
        };
    }
    Some(pixels)
}

fn indexed(
    raw: &[u8],
    width: usize,
    height: usize,
    depth: usize,
    palette: &[Option<Rgba<u8>>; 256],
) -> Option<RgbaImage> {
    let mut pixels = RgbaImage::new(width as u32, height as u32);
    let stride = (width * depth).div_ceil(8);
    for (y, row) in raw.chunks_exact(stride).enumerate() {
        for x in 0..width {
            let bit = x * depth;
            let index = (usize::from(row[bit / 8]) >> (8 - depth - bit % 8)) & ((1 << depth) - 1);
            pixels.put_pixel(x as u32, y as u32, palette[index]?);
        }
    }
    Some(pixels)
}

fn run(raw: &mut Vec<u8>, sample: &[u8], count: usize, total: usize) -> Option<()> {
    if sample.len() * count > total - raw.len() {
        return None;
    }
    for _ in 0..count {
        raw.extend_from_slice(sample);
    }
    Some(())
}

fn repeat_row(raw: &mut Vec<u8>, stride: usize, count: usize, total: usize) -> Option<()> {
    let start = raw.len().checked_sub(stride)?;
    if start % stride != 0 || stride * count > total - raw.len() {
        return None;
    }
    for _ in 0..count {
        raw.extend_from_within(start..start + stride);
    }
    Some(())
}

fn expand_v2(mut data: &[u8], stride: usize, total: usize) -> Option<Vec<u8>> {
    let mut raw = Vec::new();
    let mut last = vec![0];
    let mut xor_end = None;
    while raw.len() < total {
        match byte(&mut data)? {
            0x7d => {
                let size = usize::from(byte(&mut data)?);
                if size == 0 || size > stride {
                    return None;
                }
                last = vec![0; size];
            }
            0x7e => {
                if raw.len() < stride || raw.len() % stride != 0 || xor_end.is_some() {
                    return None;
                }
                xor_end = Some(raw.len() + stride);
            }
            opcode @ (0x7f | 0xff) => {
                let sample = vec![if opcode == 0xff { 255 } else { 0 }; last.len()];
                run(&mut raw, &sample, usize::from(byte(&mut data)?) + 1, total)?;
            }
            0xfd => run(&mut raw, &last, usize::from(byte(&mut data)?) + 1, total)?,
            0xfe => repeat_row(&mut raw, stride, usize::from(byte(&mut data)?) + 1, total)?,
            op @ 0x80..=0xfc => {
                let sample = split(&mut data, last.len())?;
                last.copy_from_slice(sample);
                run(&mut raw, &last, usize::from(op & 0x7f) + 1, total)?;
            }
            op => {
                let count = (usize::from(op) + 1) * last.len();
                if count > total - raw.len() {
                    return None;
                }
                raw.extend_from_slice(split(&mut data, count)?);
            }
        }
        if let Some(end) = xor_end.filter(|&end| raw.len() >= end) {
            for i in end - stride..end {
                raw[i] ^= raw[i - stride];
            }
            xor_end = None;
        }
    }
    Some(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_monochrome_wpg1_with_a_repeated_row() {
        let mut wpg = b"\xffWPC\x10\0\0\0\x01\x16\x01\0\0\0\0\0".to_vec();
        wpg.extend(
            b"\x0f\x06\0\0\0\0\0\0\x0b\x0e\x08\0\x03\0\x01\0\x48\0\x48\0\x82\x80\0\x01\x10\0",
        );
        let (png, physical) = decode(&wpg).unwrap();
        assert_eq!(physical, EmuSize::new(101_600, 38_100));
        let image = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(image.dimensions(), (8, 3));
        for y in 0..3 {
            assert_eq!(image.get_pixel(0, y).0, [255; 4]);
            assert_eq!(image.get_pixel(1, y).0, [0, 0, 0, 255]);
        }
        wpg.truncate(wpg.len() - 3);
        assert!(decode(&wpg).is_none());
    }

    #[test]
    fn decodes_non_right_angle_wpg2_rotation_with_nonuniform_scale() {
        let mut object = 0x12u16.to_le_bytes().to_vec();
        let cos = 30f32.to_radians().cos();
        for value in [30.0, 1.5 * cos, 0.5 * cos, -0.25, 0.75] {
            object.extend(((value * 65536.0).round() as i32).to_le_bytes());
        }
        for (integer, fraction) in [(28i32, 32768u16), (-10, 0)] {
            object.extend(fraction.to_le_bytes());
            object.extend(integer.to_le_bytes());
        }
        let canvas = Canvas {
            bounds: [0, 0, 1200, 900],
            dpi_x: 1200,
            dpi_y: 1200,
            coordinate_bytes: 2,
        };
        let (_, transform, rest) = canvas.object(&object).unwrap();
        let mut x = tiny_skia::Point::from_xy(100.0, 0.0);
        let mut y = tiny_skia::Point::from_xy(0.0, 100.0);
        transform.map_point(&mut x);
        transform.map_point(&mut y);
        assert!((x.x - (28.5 + 150.0 * cos)).abs() < 0.002);
        assert_eq!(x.y, 65.0);
        assert_eq!(y.x, 3.5);
        assert!((y.y - (-10.0 + 50.0 * cos)).abs() < 0.002);
        assert!(rest.is_empty());
    }

    #[test]
    fn preserves_wpg2_direct_alpha() {
        let record = [1, 0, 1, 0, 9, 0, 8, 42, 119, 192];
        let pixels = bitmap_v2(&record, &[None; 256]).unwrap();
        assert_eq!(pixels.get_pixel(0, 0).0, [8, 42, 119, 192]);
    }

    #[test]
    fn expands_wpg2_samples_without_overrunning_the_raster() {
        let compressed = [0x7d, 3, 0x81, 17, 42, 87, 0xfe, 0];
        assert_eq!(
            expand_v2(&compressed, 6, 12).unwrap(),
            [17, 42, 87].repeat(4)
        );
        assert!(expand_v2(&compressed, 6, 11).is_none());
        assert!(expand_v2(&[0xfe, 0], 6, 12).is_none());
    }
}
