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
    Photoshop,
    OpenExr,
}

fn format(ext: &str) -> Option<Format> {
    match ext.to_ascii_lowercase().as_str() {
        "exr" => Some(Format::OpenExr),
        "psd" => Some(Format::Photoshop),
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
        Format::Raster(format) => decode_raster(bytes, format, 4)?,
        Format::OpenExr => decode_openexr(bytes)?,
        Format::Jpeg2000 => decode_jpeg2000(bytes)?,
        Format::Photoshop => decode_psd(bytes)?,
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

fn decode_raster(
    bytes: &[u8],
    format: ImageFormat,
    bytes_per_pixel: u64,
) -> Result<DynamicImage, String> {
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(MAX_PIXELS * bytes_per_pixel);
    reader.limits(limits);
    reader.decode().map_err(|error| error.to_string())
}

fn decode_openexr(bytes: &[u8]) -> Result<DynamicImage, String> {
    let metadata = exr::meta::MetaData::read_from_buffered(Cursor::new(bytes), false)
        .map_err(|error| error.to_string())?;
    let [header] = metadata.headers.as_slice() else {
        return Err("OpenEXR preview requires a single part".into());
    };
    if header.deep {
        return Err("Deep OpenEXR images cannot be previewed".into());
    }
    if !is_rec709_exr(header) {
        return Err("OpenEXR preview supports linear Rec.709 color only".into());
    }
    for size in [
        header.shared_attributes.display_window.size,
        header.layer_size,
    ] {
        if size.0 > 8192 || size.1 > 8192 || size.0 as u64 * size.1 as u64 > MAX_PIXELS {
            return Err("OpenEXR image or display window exceeds the preview limits".into());
        }
    }

    let linear = decode_raster(bytes, ImageFormat::OpenExr, 16)?.into_rgba32f();
    let preview = image::RgbaImage::from_fn(linear.width(), linear.height(), |x, y| {
        let [r, g, b, a] = linear.get_pixel(x, y).0;
        let alpha = if a.is_finite() {
            a.clamp(0.0, 1.0)
        } else {
            0.0
        };
        // EXR convention associates RGB with alpha. PNG stores straight RGB.
        let rgb = [r, g, b].map(|sample| {
            if alpha > 0.0 {
                sdr_sample(sample / a)
            } else {
                0
            }
        });
        image::Rgba([rgb[0], rgb[1], rgb[2], (alpha * 255.0).round() as u8])
    });
    Ok(DynamicImage::ImageRgba8(preview))
}

fn is_rec709_exr(header: &exr::meta::header::Header) -> bool {
    use exr::meta::attribute::AttributeValue;
    let attributes = &header.shared_attributes;
    let known_color = attributes
        .other
        .iter()
        .chain(&header.own_attributes.other)
        .all(|(name, value)| {
            if name == "colorInteropID" {
                return matches!(value, AttributeValue::Text(id) if id == "lin_rec709_scene");
            }
            name != "acesImageContainerFlag"
        });
    // Files without color metadata are previewed using the Rec.709 assumption.
    known_color
        && attributes.chromaticities.as_ref().is_none_or(|c| {
            [
                (c.red, [0.64, 0.33]),
                (c.green, [0.30, 0.60]),
                (c.blue, [0.15, 0.06]),
                (c.white, [0.3127, 0.3290]),
            ]
            .into_iter()
            .all(|(point, [x, y])| (point.0 - x).abs() <= 0.0001 && (point.1 - y).abs() <= 0.0001)
        })
}

/// Compress highlights at 0 EV before encoding the linear values as sRGB.
fn sdr_sample(sample: f32) -> u8 {
    let mapped = if sample == f32::INFINITY {
        1.0
    } else {
        let sample = sample.max(0.0);
        sample / (1.0 + sample)
    };
    let encoded = if mapped <= 0.0031308 {
        12.92 * mapped
    } else {
        1.055 * mapped.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0).round() as u8
}

fn decode_psd(bytes: &[u8]) -> Result<DynamicImage, String> {
    let header = bytes.get(..26).ok_or("Incomplete PSD header")?;
    if &header[..6] != b"8BPS\0\x01" || header[22..26] != [0, 8, 0, 3] {
        return Err("PSD preview requires a version 1, 8-bit RGB image".into());
    }
    let height = u32::from_be_bytes(header[14..18].try_into().unwrap());
    let width = u32::from_be_bytes(header[18..22].try_into().unwrap());
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err("PSD dimensions exceed the preview limits".into());
    }
    let missing_composite =
        "PSD has no saved composite image. Enable Maximize Compatibility when saving.";
    let psd = ag_psd::read_psd(
        bytes,
        &ag_psd::psd::ReadOptions {
            skip_layer_image_data: Some(true),
            skip_thumbnail: Some(true),
            skip_linked_files_data: Some(true),
            use_image_data: Some(true),
            total_memory_limit: Some((MAX_PIXELS * 4) as usize),
            ..Default::default()
        },
    )
    .map_err(|error| format!("Cannot read the saved PSD composite: {error}"))?;
    if psd
        .image_resources
        .and_then(|resources| resources.version_info)
        .is_some_and(|version| !version.has_real_merged_data)
    {
        return Err(missing_composite.into());
    }
    let pixels = psd.image_data.ok_or(missing_composite)?;
    image::RgbaImage::from_raw(pixels.width, pixels.height, pixels.data)
        .map(DynamicImage::ImageRgba8)
        .ok_or_else(|| "Invalid PSD composite pixel buffer".into())
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
    fn opens_openexr_with_sdr_highlights_and_straight_alpha() {
        let pixels = image::Rgba32FImage::from_pixel(1, 1, image::Rgba([0.125, 0.5, 2.0, 0.5]));
        let mut exr = Cursor::new(Vec::new());
        DynamicImage::ImageRgba32F(pixels)
            .write_to(&mut exr, ImageFormat::OpenExr)
            .unwrap();
        let opened = open(Some("EXR"), exr.get_ref()).unwrap().unwrap();
        assert_eq!(preview_pixel(opened), [124, 188, 231, 128]);
        assert_eq!(sdr_sample(f32::NAN), 0);
        assert_eq!(sdr_sample(-1.0), 0);
        assert_eq!(sdr_sample(f32::INFINITY), 255);

        use exr::meta::attribute::AttributeValue;
        let mut metadata =
            exr::meta::MetaData::read_from_buffered(Cursor::new(exr.get_ref()), false).unwrap();
        let header = &mut metadata.headers[0];
        for (id, supported) in [("lin_rec709_scene", true), ("lin_ap1_scene", false)] {
            header
                .own_attributes
                .other
                .insert("colorInteropID".into(), AttributeValue::Text(id.into()));
            assert_eq!(is_rec709_exr(header), supported);
        }
    }

    #[test]
    fn opens_saved_psd_composite() {
        use ag_psd::psd::{ColorMode, ImageResources, PixelData, Psd, VersionInfo, WriteOptions};
        let pixels = [25, 100, 225, 255];
        let mut psd = Psd {
            width: 1.0,
            height: 1.0,
            color_mode: Some(ColorMode::Rgb),
            bits_per_channel: Some(8.0),
            image_data: Some(PixelData {
                width: 1,
                height: 1,
                data: pixels.to_vec(),
            }),
            ..Default::default()
        };
        let mut bytes = ag_psd::write_psd(&psd, &WriteOptions::default());
        assert_eq!(
            preview_pixel(open(Some("psd"), &bytes).unwrap().unwrap()),
            pixels
        );
        bytes.truncate(26);
        assert!(open(Some("psd"), &bytes).is_err());
        psd.image_resources = Some(ImageResources {
            version_info: Some(VersionInfo::default()),
            ..Default::default()
        });
        let without_composite = ag_psd::write_psd(&psd, &WriteOptions::default());
        assert!(matches!(open(Some("psd"), &without_composite),
            Err(error) if error.contains("Maximize Compatibility")));
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
