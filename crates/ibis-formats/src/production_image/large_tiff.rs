// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Bounded preview of oversized, uncompressed strip-based TIFF images.

use image::{DynamicImage, GrayImage, RgbImage};
use std::io::Cursor;
use tiff::{
    ColorType,
    decoder::{ChunkType, Decoder},
    tags::Tag,
};

const MAX_SOURCE_EDGE: u32 = 32_768;
const MAX_PREVIEW_EDGE: u32 = 2_048;
const UNSUPPORTED: &str = "Large TIFF preview requires uncompressed 8-bit grayscale or RGB strips";
const BAD_STRIPS: &str = "Invalid or truncated TIFF strips";

pub(super) fn preview(bytes: &[u8]) -> Result<DynamicImage, String> {
    let mut decoder = Decoder::new(Cursor::new(bytes)).map_err(|error| error.to_string())?;
    let (width, height) = decoder.dimensions().map_err(|error| error.to_string())?;
    let longest = width.max(height);
    if width == 0 || height == 0 || longest > MAX_SOURCE_EDGE {
        return Err("TIFF image dimensions exceed the large-preview limit".into());
    }
    let mut tag = |tag, default| -> Result<u16, String> {
        decoder
            .find_tag_unsigned(tag)
            .map(|value| value.unwrap_or(default))
            .map_err(|error| error.to_string())
    };
    let compression = tag(Tag::Compression, 1)?;
    let planar = tag(Tag::PlanarConfiguration, 1)?;
    let predictor = tag(Tag::Predictor, 1)?;
    let sample_format = decoder
        .find_tag_unsigned_vec::<u16>(Tag::SampleFormat)
        .map_err(|error| error.to_string())?;
    if compression != 1
        || planar != 1
        || predictor != 1
        || sample_format.is_some_and(|formats| formats.iter().any(|&format| format != 1))
        || decoder.get_chunk_type() != ChunkType::Strip
    {
        return Err(UNSUPPORTED.into());
    }
    let photometric = decoder
        .get_tag_unsigned::<u16>(Tag::PhotometricInterpretation)
        .map_err(|error| error.to_string())?;
    let channels = match (
        decoder.colortype().map_err(|error| error.to_string())?,
        photometric,
    ) {
        (ColorType::Gray(8), 0 | 1) => 1,
        (ColorType::RGB(8), 2) => 3,
        _ => return Err(UNSUPPORTED.into()),
    };
    let rows = decoder
        .find_tag_unsigned::<u32>(Tag::RowsPerStrip)
        .map_err(|error| error.to_string())?
        .unwrap_or(height)
        .min(height);
    if rows == 0 {
        return Err(BAD_STRIPS.into());
    }

    let offsets = decoder
        .get_tag_u64_vec(Tag::StripOffsets)
        .map_err(|error| error.to_string())?;
    let lengths = decoder
        .get_tag_u64_vec(Tag::StripByteCounts)
        .map_err(|error| error.to_string())?;
    let strip_count = height.div_ceil(rows) as usize;
    if offsets.len() != strip_count || lengths.len() != strip_count {
        return Err(BAD_STRIPS.into());
    }
    let row_bytes = width as usize * channels;
    let mut strips = Vec::with_capacity(strip_count);
    for (index, (offset, length)) in offsets.into_iter().zip(lengths).enumerate() {
        let expected_rows = rows.min(height - index as u32 * rows);
        let required = expected_rows as usize * row_bytes;
        let offset = usize::try_from(offset).map_err(|_| BAD_STRIPS)?;
        let length = usize::try_from(length).map_err(|_| BAD_STRIPS)?;
        let end = offset.checked_add(length).ok_or(BAD_STRIPS)?;
        let strip = bytes
            .get(offset..end)
            .filter(|strip| strip.len() >= required)
            .ok_or(BAD_STRIPS)?;
        strips.push(strip);
    }

    let divisor = longest.max(MAX_PREVIEW_EDGE);
    let scaled = |length: u32| {
        ((u64::from(length) * u64::from(MAX_PREVIEW_EDGE) + u64::from(divisor) / 2)
            / u64::from(divisor))
        .max(1) as u32
    };
    let (preview_width, preview_height) = (scaled(width), scaled(height));
    // Sample pixel centers directly from the source strips. The full raster is never allocated.
    let source_index = |index: u32, source: u32, output: u32| {
        ((2 * u64::from(index) + 1) * u64::from(source) / (2 * u64::from(output))) as u32
    };
    let mut pixels =
        Vec::with_capacity(preview_width as usize * preview_height as usize * channels);
    for y in 0..preview_height {
        let y = source_index(y, height, preview_height);
        let strip = strips[(y / rows) as usize];
        let row = (y % rows) as usize * row_bytes;
        for x in 0..preview_width {
            let x = source_index(x, width, preview_width) as usize;
            let start = row + x * channels;
            pixels.extend_from_slice(&strip[start..start + channels]);
        }
    }
    if photometric == 0 {
        pixels.iter_mut().for_each(|value| *value = 255 - *value);
    }
    if channels == 1 {
        GrayImage::from_raw(preview_width, preview_height, pixels).map(DynamicImage::ImageLuma8)
    } else {
        RgbImage::from_raw(preview_width, preview_height, pixels).map(DynamicImage::ImageRgb8)
    }
    .ok_or_else(|| BAD_STRIPS.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tiff::encoder::{TiffEncoder, colortype};

    #[test]
    fn opens_large_stripped_rgb_and_preserves_the_preview_in_docx() {
        let mut tiff = Cursor::new(Vec::new());
        let mut encoder = TiffEncoder::new(&mut tiff).unwrap();
        let mut first = encoder.new_image::<colortype::RGB8>(8_200, 80).unwrap();
        first.rows_per_strip(9).unwrap();
        let pixels: Vec<_> = (0..80)
            .flat_map(|y| (0..8_200).flat_map(move |x| [(x % 251) as u8, y as u8, 113]))
            .collect();
        first.write_data(&pixels).unwrap();
        encoder.write_image::<colortype::Gray8>(1, 1, &[0]).unwrap();
        let tiff = tiff.into_inner();
        let opened = super::super::open(Some("tiff"), &tiff).unwrap().unwrap();
        let preview = image::load_from_memory(&opened.presentation.media.last().unwrap().data)
            .unwrap()
            .into_rgb8();
        assert_eq!(preview.dimensions(), (2_048, 20));
        assert_eq!(preview.get_pixel(0, 0).0, [2, 2, 113]);
        assert_eq!(
            preview.get_pixel(1_000, 10).0,
            [(4_005 % 251) as u8, 42, 113]
        );
        let docx = opened
            .source
            .save(&opened.presentation, Default::default())
            .unwrap();
        let reopened = crate::open(docx).unwrap();
        let media = reopened.presentation.media.last().unwrap();
        assert_eq!(
            image::load_from_memory(&media.data).unwrap().into_rgb8(),
            preview
        );
    }

    #[test]
    fn rejects_a_truncated_strip() {
        let mut file = Cursor::new(Vec::new());
        TiffEncoder::new(&mut file)
            .unwrap()
            .write_image::<colortype::Gray8>(8_200, 1, &vec![4; 8_200])
            .unwrap();
        let mut bytes = file.into_inner();
        let mut decoder = Decoder::new(Cursor::new(&bytes)).unwrap();
        let offset = decoder.get_tag_u64_vec(Tag::StripOffsets).unwrap()[0] as u32;
        let ifd = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        let entries = u16::from_le_bytes(bytes[ifd..ifd + 2].try_into().unwrap()) as usize;
        let entry = bytes[ifd + 2..]
            .chunks_exact_mut(12)
            .take(entries)
            .find(|entry| entry[..2] == 273u16.to_le_bytes())
            .unwrap();
        entry[8..12].copy_from_slice(&(offset + 1_000_000).to_le_bytes());
        assert_eq!(preview(&bytes).unwrap_err(), BAD_STRIPS);
    }
}
