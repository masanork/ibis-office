// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Pictures inside Windows Enhanced Metafiles (EMF).
//!
//! Office often saves pasted pictures as an EMF that does nothing but draw
//! one bitmap, either with EMF+ (GDI+ records in EMF comments) or with a
//! GDI `EMR_STRETCHDIBITS`. [`extract_image`] finds that bitmap and returns
//! it as an ordinary image file. Vector drawing is not interpreted: a
//! metafile without a bitmap yields `None`.

mod charset;
mod emf;
pub mod gdi;
mod png;
mod svg;
mod wmf;

pub use emf::{Picture, emf_picture, emf_to_svg};
pub use svg::{FontMetricsFn, set_font_metrics};
pub use wmf::wmf_to_svg;

/// The encoding of an extracted image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Gif,
    Bmp,
    Tiff,
}

const EMR_HEADER: u32 = 1;
const EMR_EOF: u32 = 14;
const EMR_COMMENT: u32 = 70;
const EMR_STRETCHDIBITS: u32 = 81;

const EMFPLUS_OBJECT: u16 = 0x4008;
const OBJECT_IMAGE: u16 = 5;
const IMAGE_BITMAP: u32 = 1;
const BITMAP_PIXELS: u32 = 0;
const BITMAP_COMPRESSED: u32 = 1;

const PIXEL_ARGB32: u32 = 0x0026_200a;
const PIXEL_PARGB32: u32 = 0x000e_200b;
const PIXEL_RGB32: u32 = 0x0002_2009;
const PIXEL_RGB24: u32 = 0x0002_1808;

fn u16_at(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

fn i32_at(d: &[u8], at: usize) -> Option<i32> {
    Some(i32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?))
}

/// The largest bitmap the metafile draws, as an image file.
pub fn extract_image(emf: &[u8]) -> Option<(ImageFormat, Vec<u8>)> {
    if u32_at(emf, 0)? != EMR_HEADER || emf.get(40..44)? != b" EMF" {
        return None;
    }
    let mut emf_plus = Vec::new();
    let mut gdi_bitmaps = Vec::new();
    let mut rest = emf;
    while let Some((kind, record)) = gdi::take_emf_record(&mut rest) {
        let size = record.len();
        match kind {
            // EMF+ records travel in comments tagged "EMF+".
            EMR_COMMENT if record.get(12..16) == Some(b"EMF+") => {
                let len = u32_at(record, 8)? as usize;
                emf_plus.extend_from_slice(record.get(16..(12 + len).min(size))?);
            }
            EMR_STRETCHDIBITS => gdi_bitmaps.extend(stretch_dibits(record)),
            EMR_EOF => break,
            _ => {}
        }
    }
    let mut candidates = emf_plus_bitmaps(&emf_plus);
    candidates.extend(gdi_bitmaps);
    candidates.into_iter().max_by_key(|(area, _)| *area).map(|(_, image)| image)
}

pub(crate) type Candidate = (u64, (ImageFormat, Vec<u8>));

/// Bitmaps defined as EMF+ image objects, with their pixel area.
fn emf_plus_bitmaps(data: &[u8]) -> Vec<Candidate> {
    let mut out = Vec::new();
    // Objects too big for one record are split, the first part announcing
    // the total size; collect the parts per object id.
    let mut pending: Option<(u16, usize, Vec<u8>)> = None;
    let mut at = 0;
    while let (Some(kind), Some(flags), Some(size), Some(data_size)) =
        (u16_at(data, at), u16_at(data, at + 2), u32_at(data, at + 4), u32_at(data, at + 8))
    {
        let (size, data_size) = (size as usize, data_size as usize);
        if size < 12 || at + size > data.len() {
            break;
        }
        if kind == EMFPLUS_OBJECT && (flags >> 8) & 0x7f == OBJECT_IMAGE {
            let body = &data[at + 12..at + 12 + data_size.min(size - 12)];
            let id = flags & 0xff;
            if flags & 0x8000 != 0 {
                let Some(total) = u32_at(body, 0) else { break };
                let part = &body[4..];
                match &mut pending {
                    Some((pid, _, buf)) if *pid == id => buf.extend_from_slice(part),
                    _ => pending = Some((id, total as usize, part.to_vec())),
                }
                if let Some((_, total, buf)) = &pending
                    && buf.len() >= *total
                {
                    out.extend(emf_plus_image(buf));
                    pending = None;
                }
            } else {
                out.extend(emf_plus_image(body));
            }
        }
        at += size;
    }
    out
}

/// An `EmfPlusImage` object holding a bitmap.
pub(crate) fn emf_plus_image(body: &[u8]) -> Option<Candidate> {
    if u32_at(body, 4)? != IMAGE_BITMAP {
        return None;
    }
    let width = i32_at(body, 8)?;
    let height = i32_at(body, 12)?;
    let stride = i32_at(body, 16)?;
    let pixel_format = u32_at(body, 20)?;
    let bitmap_type = u32_at(body, 24)?;
    let pixels = body.get(28..)?;
    let area = width.unsigned_abs() as u64 * height.unsigned_abs() as u64;
    match bitmap_type {
        BITMAP_COMPRESSED => Some((area, (sniff(pixels)?, pixels.to_vec()))),
        BITMAP_PIXELS => {
            let rgba = to_rgba(pixels, width, height, stride, pixel_format, false)?;
            Some((area, (ImageFormat::Png, png::encode(width as u32, height as u32, &rgba))))
        }
        _ => None,
    }
}

/// The DIB of an `EMR_STRETCHDIBITS` record.
fn stretch_dibits(record: &[u8]) -> Option<Candidate> {
    let bmi_offset = u32_at(record, 48)? as usize;
    let bits_offset = u32_at(record, 56)? as usize;
    let bits_size = u32_at(record, 60)? as usize;
    let header = record.get(bmi_offset..)?;
    let width = i32_at(header, 4)?;
    let height = i32_at(header, 8)?;
    let bit_count = u16_at(header, 14)?;
    let compression = u32_at(header, 16)?;
    if compression != 0 {
        return None;
    }
    let format = match bit_count {
        32 => PIXEL_RGB32,
        24 => PIXEL_RGB24,
        _ => return None,
    };
    let stride = ((width.unsigned_abs() * bit_count as u32).div_ceil(32) * 4) as i32;
    let bits = record.get(bits_offset..bits_offset + bits_size)?;
    // DIB rows run bottom-up unless the height is negative.
    let rgba = to_rgba(bits, width, height.abs(), stride, format, height > 0)?;
    let area = width.unsigned_abs() as u64 * height.unsigned_abs() as u64;
    Some((area, (ImageFormat::Png, png::encode(width as u32, height.unsigned_abs(), &rgba))))
}

/// Convert BGR(A) rows to straight RGBA.
fn to_rgba(data: &[u8], width: i32, height: i32, stride: i32, format: u32, bottom_up: bool) -> Option<Vec<u8>> {
    if width <= 0 || height <= 0 || width > 32_768 || height > 32_768 {
        return None;
    }
    let (w, h) = (width as usize, height as usize);
    let bytes_per_pixel = match format {
        PIXEL_ARGB32 | PIXEL_PARGB32 | PIXEL_RGB32 => 4,
        PIXEL_RGB24 => 3,
        _ => return None,
    };
    let stride = stride.unsigned_abs() as usize;
    if stride < w * bytes_per_pixel || data.len() < stride * (h - 1) + w * bytes_per_pixel {
        return None;
    }
    let mut out = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let row = if bottom_up { h - 1 - y } else { y };
        let line = &data[row * stride..row * stride + w * bytes_per_pixel];
        for p in line.chunks_exact(bytes_per_pixel) {
            let (b, g, r) = (p[0], p[1], p[2]);
            let a = if bytes_per_pixel == 4 && format != PIXEL_RGB32 { p[3] } else { 255 };
            if format == PIXEL_PARGB32 && a > 0 && a < 255 {
                // Undo premultiplication.
                let un = |c: u8| ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8;
                out.extend_from_slice(&[un(r), un(g), un(b), a]);
            } else {
                out.extend_from_slice(&[r, g, b, a]);
            }
        }
    }
    Some(out)
}

/// The format of an image file, from its signature.
pub(crate) fn sniff(data: &[u8]) -> Option<ImageFormat> {
    Some(match data {
        [0x89, b'P', b'N', b'G', ..] => ImageFormat::Png,
        [0xff, 0xd8, ..] => ImageFormat::Jpeg,
        [b'G', b'I', b'F', ..] => ImageFormat::Gif,
        [b'B', b'M', ..] => ImageFormat::Bmp,
        [b'I', b'I', 42, 0, ..] | [b'M', b'M', 0, 42, ..] => ImageFormat::Tiff,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A metafile whose only content is one EMF+ image object.
    fn emf_with_plus_image(image: &[u8]) -> Vec<u8> {
        let mut plus = Vec::new();
        plus.extend(EMFPLUS_OBJECT.to_le_bytes());
        plus.extend((OBJECT_IMAGE << 8).to_le_bytes());
        plus.extend((12 + image.len() as u32).to_le_bytes());
        plus.extend((image.len() as u32).to_le_bytes());
        plus.extend(image);

        let mut emf = vec![0u8; 88];
        emf[0..4].copy_from_slice(&EMR_HEADER.to_le_bytes());
        emf[4..8].copy_from_slice(&88u32.to_le_bytes());
        emf[40..44].copy_from_slice(b" EMF");
        let comment_len = 16 + plus.len();
        emf.extend(EMR_COMMENT.to_le_bytes());
        emf.extend((comment_len as u32).to_le_bytes());
        emf.extend((plus.len() as u32 + 4).to_le_bytes());
        emf.extend(b"EMF+");
        emf.extend(&plus);
        emf.extend(EMR_EOF.to_le_bytes());
        emf.extend(20u32.to_le_bytes());
        emf.extend([0u8; 12]);
        emf
    }

    fn bitmap_object(width: i32, height: i32, format: u32, kind: u32, data: &[u8]) -> Vec<u8> {
        let mut o = Vec::new();
        o.extend(0xdbc0_1002u32.to_le_bytes()); // version
        o.extend(IMAGE_BITMAP.to_le_bytes());
        o.extend(width.to_le_bytes());
        o.extend(height.to_le_bytes());
        o.extend((width * 4).to_le_bytes());
        o.extend(format.to_le_bytes());
        o.extend(kind.to_le_bytes());
        o.extend(data);
        o
    }

    #[test]
    fn extracts_premultiplied_pixels_as_png() {
        // 2×1: opaque red, half-transparent premultiplied blue.
        let pixels = [0, 0, 255, 255, 64, 0, 0, 128];
        let emf = emf_with_plus_image(&bitmap_object(2, 1, PIXEL_PARGB32, BITMAP_PIXELS, &pixels));
        let (format, png) = extract_image(&emf).expect("an image");
        assert_eq!(format, ImageFormat::Png);
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(png::decode_rgba_for_test(&png), vec![255, 0, 0, 255, 0, 0, 128, 128]);
    }

    #[test]
    fn passes_compressed_images_through() {
        let jpeg = [0xff, 0xd8, 0xff, 0xe0, 1, 2, 3];
        let emf = emf_with_plus_image(&bitmap_object(1, 1, PIXEL_ARGB32, BITMAP_COMPRESSED, &jpeg));
        assert_eq!(extract_image(&emf), Some((ImageFormat::Jpeg, jpeg.to_vec())));
    }

    #[test]
    fn rejects_non_emf() {
        assert_eq!(extract_image(b"not a metafile"), None);
    }
}
