// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Bounded preview of oversized TIFF images.

use image::{DynamicImage, GrayImage, RgbImage};
use std::io::{Read, Seek};
use tiff::{
    ColorType,
    decoder::{ChunkType, Decoder, DecodingResult, Limits},
    tags::{CompressionMethod, Tag},
};

const MAX_SOURCE_EDGE: u32 = 32_768;
const MAX_PREVIEW_EDGE: u32 = 2_048;
const MAX_CHUNK_BYTES: usize = 8 * 1024 * 1024;
const MAX_RASTER_BYTES: u64 = 256 * 1024 * 1024;
const UNSUPPORTED: &str =
    "Large TIFF preview requires supported 8-bit grayscale or RGB strips or tiles";
const CHUNK_LIMIT: &str = "TIFF strip or tile exceeds the large-preview limit";
const BAD_CHUNKS: &str = "Invalid or truncated TIFF strips or tiles";

pub(super) fn preview(
    bytes: &[u8],
    mut decoder: Decoder<impl Read + Seek>,
) -> Result<DynamicImage, String> {
    let (width, height) = decoder.dimensions().map_err(|error| error.to_string())?;
    let longest = width.max(height);
    if width == 0 || height == 0 || longest > MAX_SOURCE_EDGE {
        return Err("TIFF image dimensions exceed the large-preview limit".into());
    }
    let tiled = decoder.get_chunk_type() == ChunkType::Tile;
    let mut tag = |tag, default| -> Result<u16, String> {
        decoder
            .find_tag_unsigned(tag)
            .map(|value| value.unwrap_or(default))
            .map_err(|error| error.to_string())
    };
    let compression = CompressionMethod::from_u16_exhaustive(tag(Tag::Compression, 1)?);
    let uncompressed = compression == CompressionMethod::None;
    let borrow_strips = uncompressed && !tiled;
    let planar = tag(Tag::PlanarConfiguration, 1)?;
    let predictor = tag(Tag::Predictor, 1)?;
    let sample_format = decoder
        .find_tag_unsigned_vec::<u16>(Tag::SampleFormat)
        .map_err(|error| error.to_string())?;
    if !matches!(
        compression,
        CompressionMethod::None
            | CompressionMethod::LZW
            | CompressionMethod::Deflate
            | CompressionMethod::OldDeflate
            | CompressionMethod::PackBits
    ) || planar != 1
        || !(predictor == 1 || !uncompressed && predictor == 2)
        || sample_format.is_some_and(|formats| formats.iter().any(|&format| format != 1))
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
    let (chunk_width, chunk_height, offsets_tag, lengths_tag) = if tiled {
        (
            decoder
                .get_tag_unsigned::<u32>(Tag::TileWidth)
                .map_err(|error| error.to_string())?,
            decoder
                .get_tag_unsigned::<u32>(Tag::TileLength)
                .map_err(|error| error.to_string())?,
            Tag::TileOffsets,
            Tag::TileByteCounts,
        )
    } else {
        let rows = decoder
            .find_tag_unsigned::<u32>(Tag::RowsPerStrip)
            .map_err(|error| error.to_string())?
            .unwrap_or(height)
            .min(height);
        (width, rows, Tag::StripOffsets, Tag::StripByteCounts)
    };
    if chunk_width == 0 || chunk_height == 0 || chunk_width.max(chunk_height) > MAX_SOURCE_EDGE {
        return Err(BAD_CHUNKS.into());
    }

    let columns = width.div_ceil(chunk_width);
    let rows = height.div_ceil(chunk_height);
    let chunk_count = (columns * rows) as usize;
    let nominal_bytes = u64::from(chunk_width) * u64::from(chunk_height) * channels as u64;
    if !borrow_strips {
        if nominal_bytes > MAX_CHUNK_BYTES as u64 {
            return Err(CHUNK_LIMIT.into());
        }
        let raster_bytes = if tiled {
            nominal_bytes * chunk_count as u64
        } else {
            u64::from(width) * u64::from(height) * channels as u64
        };
        if raster_bytes > MAX_RASTER_BYTES {
            return Err("TIFF image exceeds the large-preview raster limit".into());
        }
    }
    let offsets = decoder
        .get_tag_u64_vec(offsets_tag)
        .map_err(|error| error.to_string())?;
    let lengths = decoder
        .get_tag_u64_vec(lengths_tag)
        .map_err(|error| error.to_string())?;
    if offsets.len() != chunk_count || lengths.len() != chunk_count {
        return Err(BAD_CHUNKS.into());
    }
    if !borrow_strips
        && lengths
            .iter()
            .try_fold(0_u64, |total, &len| total.checked_add(len))
            .is_none_or(|total| total > super::MAX_INPUT_BYTES as u64)
    {
        return Err("TIFF chunks exceed the large-preview input limit".into());
    }
    let mut chunks = Vec::with_capacity(chunk_count);
    for (index, (offset, length)) in offsets.into_iter().zip(lengths).enumerate() {
        let origin_x = (index as u32 % columns) * chunk_width;
        let origin_y = (index as u32 / columns) * chunk_height;
        let data_width = chunk_width.min(width - origin_x);
        let data_height = chunk_height.min(height - origin_y);
        let row_bytes = data_width as usize * channels;
        let offset = usize::try_from(offset).map_err(|_| BAD_CHUNKS)?;
        let length = usize::try_from(length).map_err(|_| BAD_CHUNKS)?;
        if !borrow_strips && length > MAX_CHUNK_BYTES {
            return Err(CHUNK_LIMIT.into());
        }
        let required = if uncompressed {
            if tiled {
                nominal_bytes as usize
            } else {
                data_height as usize * row_bytes
            }
        } else {
            1
        };
        let end = offset.checked_add(length).ok_or(BAD_CHUNKS)?;
        let raw = bytes
            .get(offset..end)
            .filter(|chunk| chunk.len() >= required)
            .ok_or(BAD_CHUNKS)?;
        chunks.push((origin_x, origin_y, data_width, data_height, raw));
    }

    let mut limits = Limits::default();
    limits.decoding_buffer_size = MAX_CHUNK_BYTES;
    limits.intermediate_buffer_size = MAX_CHUNK_BYTES;
    let mut decoder = decoder.with_limits(limits);

    let divisor = longest.max(MAX_PREVIEW_EDGE);
    let scaled = |length: u32| {
        ((u64::from(length) * u64::from(MAX_PREVIEW_EDGE) + u64::from(divisor) / 2)
            / u64::from(divisor))
        .max(1) as u32
    };
    let (preview_width, preview_height) = (scaled(width), scaled(height));
    // Look up the output pixels covered by each chunk without assembling the source raster.
    let sample_centers = |source: u32, output: u32| {
        (0..output)
            .map(|index| {
                ((2 * u64::from(index) + 1) * u64::from(source) / (2 * u64::from(output))) as u32
            })
            .collect::<Vec<_>>()
    };
    let xs = sample_centers(width, preview_width);
    let ys = sample_centers(height, preview_height);
    let mut pixels = vec![0; preview_width as usize * preview_height as usize * channels];
    for (index, (left, top, data_width, data_height, raw)) in chunks.into_iter().enumerate() {
        let decoded;
        let data = if borrow_strips {
            raw
        } else {
            let DecodingResult::U8(data) = decoder
                .read_chunk(index as u32)
                .map_err(|error| error.to_string())?
            else {
                return Err(UNSUPPORTED.into());
            };
            decoded = data;
            &decoded
        };
        let x_start = xs.partition_point(|&x| x < left);
        let x_end = xs.partition_point(|&x| x < left + data_width);
        let y_start = ys.partition_point(|&y| y < top);
        let y_end = ys.partition_point(|&y| y < top + data_height);
        let row_bytes = data_width as usize * channels;
        for (output_y, &y) in ys.iter().enumerate().take(y_end).skip(y_start) {
            let row = (y - top) as usize * row_bytes;
            let row = data.get(row..row + row_bytes).ok_or(BAD_CHUNKS)?;
            for (output_x, &x) in xs.iter().enumerate().take(x_end).skip(x_start) {
                let source_x = (x - left) as usize * channels;
                let output = (output_y * preview_width as usize + output_x) * channels;
                pixels[output..output + channels]
                    .copy_from_slice(&row[source_x..source_x + channels]);
            }
        }
    }
    // The decoder already normalizes WhiteIsZero when it reads a chunk.
    if borrow_strips && photometric == 0 {
        pixels.iter_mut().for_each(|value| *value = 255 - *value);
    }
    if channels == 1 {
        GrayImage::from_raw(preview_width, preview_height, pixels).map(DynamicImage::ImageLuma8)
    } else {
        RgbImage::from_raw(preview_width, preview_height, pixels).map(DynamicImage::ImageRgb8)
    }
    .ok_or_else(|| BAD_CHUNKS.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn preview_test(bytes: &[u8]) -> Result<DynamicImage, String> {
        preview(bytes, Decoder::new(Cursor::new(bytes)).unwrap())
    }
    use tiff::encoder::{Compression, DeflateLevel, Predictor, TiffEncoder, colortype};

    #[test]
    fn opens_large_stripped_rgb_and_preserves_the_preview_in_docx() {
        for compression in [
            Compression::Uncompressed,
            Compression::Lzw,
            Compression::Deflate(DeflateLevel::Fast),
            Compression::Packbits,
        ] {
            check_rgb_preview(compression);
        }
    }

    fn check_rgb_preview(compression: Compression) {
        let mut tiff = Cursor::new(Vec::new());
        let mut encoder = TiffEncoder::new(&mut tiff)
            .unwrap()
            .with_compression(compression);
        if matches!(compression, Compression::Lzw | Compression::Deflate(_)) {
            encoder = encoder.with_predictor(Predictor::Horizontal);
        }
        let mut first = encoder.new_image::<colortype::RGB8>(8_200, 80).unwrap();
        first.rows_per_strip(9).unwrap();
        let pixels: Vec<_> = (0..80)
            .flat_map(|y| (0..8_200).flat_map(move |x| [(x % 251) as u8, y as u8, 113]))
            .collect();
        first.write_data(&pixels).unwrap();
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
    fn opens_padded_tiles_and_saves_the_same_docx_pixels() {
        let (width, height) = (8_201_u32, 35_u32);
        let (tile_width, tile_height) = (128_u32, 16_u32);
        for compressed in [false, true] {
            let mut file = Cursor::new(Vec::new());
            let mut encoder = TiffEncoder::new(&mut file).unwrap();
            let mut directory = encoder.image_directory().unwrap();
            let mut offsets = Vec::new();
            let mut lengths = Vec::new();
            for top in (0..height).step_by(tile_height as usize) {
                for left in (0..width).step_by(tile_width as usize) {
                    let pixels: Vec<_> = (top..top + tile_height)
                        .flat_map(|y| {
                            (left..left + tile_width).map(move |x| ((x + y * 13) % 251) as u8)
                        })
                        .collect();
                    let encoded = if compressed {
                        miniz_oxide::deflate::compress_to_vec_zlib(&pixels, 1)
                    } else {
                        pixels
                    };
                    offsets.push(directory.write_data(encoded.as_slice()).unwrap() as u32);
                    lengths.push(encoded.len() as u32);
                }
            }
            directory.write_tag(Tag::ImageWidth, width).unwrap();
            directory.write_tag(Tag::ImageLength, height).unwrap();
            directory.write_tag(Tag::TileWidth, tile_width).unwrap();
            directory.write_tag(Tag::TileLength, tile_height).unwrap();
            directory.write_tag(Tag::BitsPerSample, 8_u16).unwrap();
            directory.write_tag(Tag::SamplesPerPixel, 1_u16).unwrap();
            directory
                .write_tag(Tag::PhotometricInterpretation, 1_u16)
                .unwrap();
            directory
                .write_tag(Tag::Compression, if compressed { 8_u16 } else { 1_u16 })
                .unwrap();
            directory
                .write_tag(Tag::TileOffsets, offsets.as_slice())
                .unwrap();
            directory
                .write_tag(Tag::TileByteCounts, lengths.as_slice())
                .unwrap();
            directory.finish().unwrap();

            let opened = super::super::open(Some("tiff"), &file.into_inner())
                .unwrap()
                .unwrap();
            let preview = image::load_from_memory(&opened.presentation.media.last().unwrap().data)
                .unwrap()
                .into_luma8();
            assert_eq!(preview.dimensions(), (2_048, 9));
            for (x, y, pixel) in preview.enumerate_pixels() {
                let source_x = (2 * x + 1) * width / 4_096;
                let source_y = (2 * y + 1) * height / 18;
                assert_eq!(pixel.0, [((source_x + source_y * 13) % 251) as u8]);
            }
            let exported = opened
                .source
                .save(&opened.presentation, Default::default())
                .unwrap();
            let reopened = crate::open(exported).unwrap();
            let image = image::load_from_memory(&reopened.presentation.media.last().unwrap().data)
                .unwrap()
                .into_luma8();
            assert_eq!(image, preview);
        }
    }

    #[test]
    fn rejects_a_compressed_strip_over_the_decode_budget() {
        let width = 8_200;
        let height = (MAX_CHUNK_BYTES as u32 / width) + 1;
        let mut file = Cursor::new(Vec::new());
        let mut encoder = TiffEncoder::new(&mut file)
            .unwrap()
            .with_compression(Compression::Deflate(DeflateLevel::Fast));
        let mut image = encoder
            .new_image::<colortype::Gray8>(width, height)
            .unwrap();
        image.rows_per_strip(height).unwrap();
        image
            .write_data(&vec![0; (width * height) as usize])
            .unwrap();
        assert_eq!(preview_test(&file.into_inner()).unwrap_err(), CHUNK_LIMIT);
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
        assert_eq!(preview_test(&bytes).unwrap_err(), BAD_CHUNKS);
    }
}
