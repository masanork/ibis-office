// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Import standalone production images into a document for preview.

mod cineon;
mod dpx;
mod large_tiff;
mod tiff_pages;
#[cfg(target_os = "macos")]
mod heic;

use base64::Engine as _;
use image::{DynamicImage, ImageFormat, ImageReader};
use std::io::{BufRead, Cursor, Seek};

const MAX_PIXELS: u64 = 32 * 1024 * 1024;
const MAX_INPUT_BYTES: usize = 128 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DpxColor {
    LinearRec709,
    VideoRec709,
}

/// A supported DPX layout whose missing color tags require a user choice.
pub const DPX_NEEDS_COLOR: &str = "DPX preview requires an explicit Rec.709 color interpretation";

enum Format {
    Raster(ImageFormat),
    Jpeg2000,
    Photoshop,
    Tiff,
    OpenExr,
    Dpx,
    Cineon,
    Heic,
}

fn format(ext: &str) -> Option<Format> {
    match ext.to_ascii_lowercase().as_str() {
        "heic" | "heif" => Some(Format::Heic),
        "cin" => Some(Format::Cineon),
        "dpx" => Some(Format::Dpx),
        "exr" => Some(Format::OpenExr),
        "psd" => Some(Format::Photoshop),
        "jpg" | "jpeg" => Some(Format::Raster(ImageFormat::Jpeg)),
        "png" => Some(Format::Raster(ImageFormat::Png)),
        "tga" => Some(Format::Raster(ImageFormat::Tga)),
        "tif" | "tiff" => Some(Format::Tiff),
        "jp2" | "j2k" | "j2c" => Some(Format::Jpeg2000),
        _ => None,
    }
}

pub fn supports_extension(ext: &str) -> bool {
    format(ext).is_some()
}

pub fn open(extension: Option<&str>, bytes: &[u8]) -> Result<Option<crate::Opened>, String> {
    open_with_exposure(extension, bytes).map(|result| result.map(|(opened, _)| opened))
}

/// Open the SDR document and retain the linear samples when exposure can be adjusted.
pub fn open_with_exposure(
    extension: Option<&str>,
    bytes: &[u8],
) -> Result<Option<(crate::Opened, Option<ExrExposure>)>, String> {
    open_with_dpx_color(extension, bytes, None)
}

pub fn open_with_dpx_color(
    extension: Option<&str>,
    bytes: &[u8],
    dpx_color: Option<DpxColor>,
) -> Result<Option<(crate::Opened, Option<ExrExposure>)>, String> {
    let Some(format) = extension.and_then(format) else {
        return Ok(None);
    };
    if bytes.len() > MAX_INPUT_BYTES {
        return Err("Image exceeds the preview file size limit".into());
    }
    let (png, exposure) = match format {
        Format::Heic => {
            #[cfg(target_os = "macos")]
            {
                (
                    encode_png(DynamicImage::ImageRgba8(heic::decode(bytes)?))?,
                    None,
                )
            }
            #[cfg(any(target_os = "windows", target_os = "linux"))]
            {
                validate_heic_still(bytes)?;
                (ibis_heic_web::decode_heic_png(bytes)?, None)
            }
            #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
            {
                return Err("HEIC decoding is unavailable on this platform".into());
            }
        }
        Format::Cineon => (
            encode_png(DynamicImage::ImageRgb8(cineon::decode(bytes)?))?,
            None,
        ),
        Format::Dpx => (
            encode_png(DynamicImage::ImageRgb8(dpx::decode(bytes, dpx_color)?))?,
            None,
        ),
        Format::OpenExr => {
            let exposure = ExrExposure(decode_openexr(bytes)?);
            (exposure.preview_png(0)?, Some(exposure))
        }
        Format::Tiff => return tiff_pages::open(bytes).map(|opened| Some((opened, None))),
        Format::Raster(format) => (encode_png(decode_raster(bytes, format, 4)?)?, None),
        Format::Jpeg2000 => (encode_png(decode_jpeg2000(bytes)?)?, None),
        Format::Photoshop => (encode_png(decode_psd(bytes)?)?, None),
    };
    open_preview_png(&png).map(|opened| Some((opened, exposure)))
}

fn open_preview_png(png: &[u8]) -> Result<crate::Opened, String> {
    let data_url = base64::engine::general_purpose::STANDARD.encode(png);
    let markdown = format!("![Image preview](data:image/png;base64,{data_url})\n");
    crate::markdown::open_markdown(markdown.into_bytes()).map_err(|error| error.to_string())
}

#[cfg(target_family = "wasm")]
pub fn open_browser_heic_preview(png: &[u8]) -> Result<crate::Opened, String> {
    if png.len() > MAX_INPUT_BYTES {
        return Err("HEIC preview exceeds the preview file size limit".into());
    }
    open_preview_png(png)
}

pub fn validate_heic_still(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > MAX_INPUT_BYTES {
        return Err("Image exceeds the preview file size limit".into());
    }
    // Require the HEVC still-image brand. Image I/O can also open unrelated formats.
    let header = bytes.get(..16).ok_or("Missing HEIC file type box")?;
    let box_size = u32::from_be_bytes(header[..4].try_into().unwrap()) as usize;
    let file_type = bytes.get(0..box_size).filter(|box_bytes| {
        box_bytes.len() >= 16 && box_bytes.len() % 4 == 0 && &header[4..8] == b"ftyp"
    });
    if !file_type.is_some_and(|file_type| {
        &header[8..12] == b"heic"
            || (&header[8..12] == b"mif1"
                && file_type[16..]
                    .chunks_exact(4)
                    .any(|brand| brand == b"heic"))
    }) {
        return Err("HEIC preview requires an HEVC still image".into());
    }

    Ok(())
}

#[cfg(target_family = "wasm")]
pub fn is_heic_extension(extension: &str) -> bool {
    matches!(format(extension), Some(Format::Heic))
}

fn encode_png(image: DynamicImage) -> Result<Vec<u8>, String> {
    let mut png = Cursor::new(Vec::new());
    image
        .write_to(&mut png, ImageFormat::Png)
        .map_err(|error| error.to_string())?;
    Ok(png.into_inner())
}

/// Original scene-linear samples for the lifetime of the open preview.
pub struct ExrExposure(image::Rgba32FImage);

impl ExrExposure {
    pub fn preview_png(&self, ev: i8) -> Result<Vec<u8>, String> {
        encode_png(DynamicImage::ImageRgba8(self.preview_pixels(ev)))
    }

    fn preview_pixels(&self, ev: i8) -> image::RgbaImage {
        let linear = &self.0;
        let gain = 2.0_f32.powi(i32::from(ev));
        image::RgbaImage::from_fn(linear.width(), linear.height(), |x, y| {
            let [r, g, b, a] = linear.get_pixel(x, y).0;
            let alpha = if a.is_finite() {
                a.clamp(0.0, 1.0)
            } else {
                0.0
            };
            // EXR associates RGB with alpha. The display PNG stores straight RGB.
            let rgb = [r, g, b].map(|sample| {
                if alpha > 0.0 {
                    sdr_sample((sample / a) * gain)
                } else {
                    0
                }
            });
            image::Rgba([rgb[0], rgb[1], rgb[2], (alpha * 255.0).round() as u8])
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlphaPreview {
    Original,
    Checker,
    Black,
    Alpha,
}

/// Build a display-only PNG from the original pixels and the selected view.
pub fn display_preview_png(
    original: &[u8],
    exposure: Option<&ExrExposure>,
    ev: i8,
    mode: AlphaPreview,
) -> Result<Vec<u8>, String> {
    let mut image = match exposure {
        Some(exposure) => exposure.preview_pixels(ev),
        None => decode_raster(original, ImageFormat::Png, 4)?.into_rgba8(),
    };
    if mode == AlphaPreview::Original {
        return encode_png(DynamicImage::ImageRgba8(image));
    }
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let [r, g, b, alpha] = pixel.0;
        let backdrop = match mode {
            AlphaPreview::Original => unreachable!(),
            AlphaPreview::Alpha => {
                pixel.0 = [alpha, alpha, alpha, 255];
                continue;
            }
            AlphaPreview::Black => 0,
            AlphaPreview::Checker if ((x / 8) + (y / 8)) % 2 == 0 => 224,
            AlphaPreview::Checker => 160,
        };
        let alpha = u16::from(alpha);
        let blend =
            |channel| ((u16::from(channel) * alpha + backdrop * (255 - alpha) + 127) / 255) as u8;
        pixel.0 = [blend(r), blend(g), blend(b), 255];
    }
    encode_png(DynamicImage::ImageRgba8(image))
}

fn decode_raster(
    bytes: &[u8],
    format: ImageFormat,
    bytes_per_pixel: u64,
) -> Result<DynamicImage, String> {
    decode_raster_with_limits(bytes, format, bytes_per_pixel).map_err(|error| error.to_string())
}

fn decode_raster_with_limits(
    bytes: &[u8],
    format: ImageFormat,
    bytes_per_pixel: u64,
) -> Result<DynamicImage, image::ImageError> {
    decode_raster_reader(Cursor::new(bytes), format, bytes_per_pixel)
}

fn decode_raster_reader(
    reader: impl BufRead + Seek,
    format: ImageFormat,
    bytes_per_pixel: u64,
) -> Result<DynamicImage, image::ImageError> {
    let mut reader = ImageReader::with_format(reader, format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(MAX_PIXELS * bytes_per_pixel);
    reader.limits(limits);
    reader.decode()
}

fn decode_openexr(bytes: &[u8]) -> Result<image::Rgba32FImage, String> {
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

    Ok(decode_raster(bytes, ImageFormat::OpenExr, 16)?.into_rgba32f())
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

/// Compress highlights before encoding exposure-adjusted linear values as sRGB.
fn sdr_sample(sample: f32) -> u8 {
    let mapped = if sample == f32::INFINITY {
        1.0
    } else {
        let sample = sample.max(0.0);
        sample / (1.0 + sample)
    };
    encode_srgb(mapped)
}

fn encode_srgb(linear: f32) -> u8 {
    let encoded = if linear <= 0.0031308 {
        12.92 * linear
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
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
        j2k_rs::decode::decode_srgb_preview(bytes, MAX_PIXELS).map_err(|error| error.to_string())?;
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
    fn previews_alpha_without_changing_the_original_png() {
        let source = image::RgbaImage::from_fn(9, 1, |x, _| {
            if x == 0 {
                image::Rgba([200, 100, 50, 128])
            } else {
                image::Rgba([0, 0, 0, 0])
            }
        });
        let png = encode_png(DynamicImage::ImageRgba8(source.clone())).unwrap();
        let display = |mode| {
            let bytes = display_preview_png(&png, None, 0, mode).unwrap();
            image::load_from_memory_with_format(&bytes, ImageFormat::Png)
                .unwrap()
                .into_rgba8()
        };
        assert_eq!(
            display(AlphaPreview::Black).get_pixel(0, 0).0,
            [100, 50, 25, 255]
        );
        assert_eq!(
            display(AlphaPreview::Alpha).get_pixel(0, 0).0,
            [128, 128, 128, 255]
        );
        let checker = display(AlphaPreview::Checker);
        assert_eq!(checker.get_pixel(1, 0).0, [224, 224, 224, 255]);
        assert_eq!(checker.get_pixel(8, 0).0, [160, 160, 160, 255]);
        assert_eq!(image::load_from_memory(&png).unwrap().into_rgba8(), source);
    }

    #[test]
    fn opens_openexr_with_sdr_highlights_and_straight_alpha() {
        let pixels = image::Rgba32FImage::from_pixel(1, 1, image::Rgba([0.125, 0.5, 2.0, 0.5]));
        let mut exr = Cursor::new(Vec::new());
        DynamicImage::ImageRgba32F(pixels)
            .write_to(&mut exr, ImageFormat::OpenExr)
            .unwrap();
        let (opened, exposure) = open_with_exposure(Some("EXR"), exr.get_ref())
            .unwrap()
            .unwrap();
        let exposure = exposure.unwrap();
        let pixel_at = |ev| {
            image::load_from_memory(&exposure.preview_png(ev).unwrap())
                .unwrap()
                .to_rgba8()
                .get_pixel(0, 0)
                .0
        };
        assert_eq!(preview_pixel(opened), [124, 188, 231, 128]);
        assert_eq!(pixel_at(1), [156, 213, 242, 128]);
        assert_eq!(pixel_at(-1), [94, 156, 213, 128]);
        assert_eq!(pixel_at(0), [124, 188, 231, 128]);
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

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn opens_heic_primary_image() {
        // ffmpeg testsrc=size=128x64:rate=1, converted to HEIC with sips.
        let sample = include_bytes!("../tests/fixtures/preview-chart.heic");
        let opened = open(Some("HEIF"), sample).unwrap().unwrap();
        let media = opened.presentation.media.last().unwrap();
        let decoded = image::load_from_memory_with_format(&media.data, ImageFormat::Png)
            .unwrap()
            .to_rgba8();
        assert_eq!(decoded.dimensions(), (128, 64));
        for channel in 0..3 {
            let count = decoded
                .pixels()
                .filter(|pixel| {
                    pixel[3] == 255
                        && (0..3).all(|i| {
                            if i == channel {
                                pixel[i] > 170
                            } else {
                                pixel[i] < 90
                            }
                        })
                })
                .count();
            assert!(count > 50, "expected saturated color in channel {channel}");
        }
        assert!(open(Some("heic"), &sample[..36]).is_err());
        assert!(open(Some("heic"), b"not an image").is_err());
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
