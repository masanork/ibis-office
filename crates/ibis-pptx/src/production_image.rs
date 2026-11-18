// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Import standalone production images into a document for preview.

use base64::Engine as _;
use image::{DynamicImage, ImageFormat, ImageReader};
use std::io::Cursor;

const MAX_PIXELS: u64 = 32 * 1024 * 1024;
const MAX_INPUT_BYTES: usize = 128 * 1024 * 1024;

enum Format {
    Raster(ImageFormat),
    Jpeg2000,
}

fn format(ext: &str) -> Option<Format> {
    match ext.to_ascii_lowercase().as_str() {
        "tga" => Some(Format::Raster(ImageFormat::Tga)),
        "tif" | "tiff" => Some(Format::Raster(ImageFormat::Tiff)),
        "jp2" | "j2k" | "j2c" => Some(Format::Jpeg2000),
        _ => None,
    }
}

pub fn supports_extension(ext: &str) -> bool {
    format(ext).is_some()
}

pub fn open(extension: Option<&str>, bytes: &[u8]) -> Result<Option<crate::Opened>, String> {
    let Some(format) = extension.and_then(format) else {
        return Ok(None);
    };
    if bytes.len() > MAX_INPUT_BYTES {
        return Err("Image exceeds the preview file size limit".into());
    }
    let image = match format {
        Format::Raster(format) => {
            let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(8192);
            limits.max_image_height = Some(8192);
            limits.max_alloc = Some(MAX_PIXELS * 4);
            reader.limits(limits);
            reader.decode().map_err(|error| error.to_string())?
        }
        Format::Jpeg2000 => decode_jpeg2000(bytes)?,
    };
    let mut png = Cursor::new(Vec::new());
    image
        .write_to(&mut png, ImageFormat::Png)
        .map_err(|error| error.to_string())?;
    let data_url = base64::engine::general_purpose::STANDARD.encode(png.into_inner());
    let markdown = format!("![Image preview](data:image/png;base64,{data_url})\n");
    crate::markdown::open_markdown(markdown.into_bytes())
        .map(Some)
        .map_err(|error| error.to_string())
}

fn decode_jpeg2000(bytes: &[u8]) -> Result<DynamicImage, String> {
    use j2k_rs::ColorSpace;
    let decoded =
        j2k_rs::decode_with_pixel_limit(bytes, MAX_PIXELS).map_err(|error| error.to_string())?;
    let (width, height, data) = (decoded.width, decoded.height, decoded.data);
    match (decoded.color_space, decoded.channels) {
        (ColorSpace::Gray, 1) => {
            image::GrayImage::from_raw(width, height, data).map(DynamicImage::ImageLuma8)
        }
        (ColorSpace::Gray, 2) => {
            image::GrayAlphaImage::from_raw(width, height, data).map(DynamicImage::ImageLumaA8)
        }
        (ColorSpace::Rgb, 3) => {
            image::RgbImage::from_raw(width, height, data).map(DynamicImage::ImageRgb8)
        }
        (ColorSpace::Rgb, 4) => {
            image::RgbaImage::from_raw(width, height, data).map(DynamicImage::ImageRgba8)
        }
        _ => return Err("Unsupported JPEG 2000 preview color space".into()),
    }
    .ok_or_else(|| "Invalid JPEG 2000 pixel buffer".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_jp2_and_j2k_as_documents() {
        let pixels = [37, 81, 205];
        let options = j2k_rs::EncodeOptions::default();
        for (extension, bytes) in [
            (
                "JP2",
                j2k_rs::encode_jp2(&pixels, 1, 1, j2k_rs::PixelFormat::Rgb8, &options).unwrap(),
            ),
            (
                "j2k",
                j2k_rs::encode_j2k(&pixels, 1, 1, j2k_rs::PixelFormat::Rgb8, &options).unwrap(),
            ),
        ] {
            let opened = open(Some(extension), &bytes).unwrap().unwrap();
            assert_eq!(preview_pixel(opened), [37, 81, 205, 255]);
            assert_eq!(
                decode_jpeg2000(&bytes).unwrap().to_rgb8().into_raw(),
                pixels
            );
            assert!(j2k_rs::decode_with_pixel_limit(&bytes, 0).is_err());
        }
    }

    fn preview_pixel(opened: crate::Opened) -> [u8; 4] {
        let media = opened.presentation.media.last().unwrap();
        image::load_from_memory_with_format(&media.data, ImageFormat::Png)
            .unwrap()
            .to_rgba8()
            .get_pixel(0, 0)
            .0
    }

    #[test]
    fn opens_first_tiff_image() {
        let pixels = image::RgbaImage::from_pixel(1, 1, image::Rgba([7, 81, 203, 127]));
        let mut tiff = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(pixels)
            .write_to(&mut tiff, ImageFormat::Tiff)
            .unwrap();
        let opened = open(Some("tiff"), tiff.get_ref()).unwrap().unwrap();
        assert_eq!(preview_pixel(opened), [7, 81, 203, 127]);
    }

    #[test]
    fn opens_rle_tga_and_rejects_truncated_pixels() {
        let mut bytes = vec![0; 18];
        bytes[2] = 10;
        bytes[12] = 1;
        bytes[14] = 1;
        bytes[16] = 32;
        bytes[17] = 40;
        bytes.extend([128, 30, 20, 10, 128]);
        let opened = open(Some("tga"), &bytes).unwrap().unwrap();
        assert_eq!(preview_pixel(opened), [10, 20, 30, 128]);
        bytes.pop();
        assert!(open(Some("tga"), &bytes).is_err());
    }
}
