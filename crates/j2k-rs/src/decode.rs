// SPDX-License-Identifier: MIT OR Apache-2.0
//! Pure-Rust JPEG 2000 decoder wrapper over [`hayro_jpeg2000`].

use hayro_jpeg2000::{ColorSpace as HayroColorSpace, DecodeSettings, Image};

use crate::{Error, Result};

/// Decoded color space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorSpace {
    Gray,
    Rgb,
    Cmyk,
    Other(u8),
}

/// Decoded 8-bit image buffer.
#[derive(Clone, Debug)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub channels: u8,
    pub color_space: ColorSpace,
    pub has_alpha: bool,
    pub data: Vec<u8>,
}

/// Decode a JPEG 2000 codestream (`.j2k`/`.j2c`) or container (`.jp2`) into 8-bit interleaved pixels.
pub fn decode(bytes: &[u8]) -> Result<DecodedImage> {
    decode_with_pixel_limit(bytes, u64::MAX)
}

/// Decode only if the image dimensions fit the caller’s pixel budget.
pub fn decode_with_pixel_limit(bytes: &[u8], max_pixels: u64) -> Result<DecodedImage> {
    decode_image(read_image(bytes, max_pixels)?)
}

fn read_image(bytes: &[u8], max_pixels: u64) -> Result<Image<'_>> {
    let image = Image::new(bytes, &DecodeSettings::default())
        .map_err(|e| Error::new(format!("JPEG 2000 header decode failed: {e:?}")))?;
    let width = image.width();
    let height = image.height();
    if u64::from(width) * u64::from(height) > max_pixels {
        return Err(Error::new("JPEG 2000 exceeds the preview pixel limit"));
    }
    Ok(image)
}

fn decode_image(image: Image<'_>) -> Result<DecodedImage> {
    let (width, height) = (image.width(), image.height());
    let has_alpha = image.has_alpha();
    let color_space = match image.color_space() {
        HayroColorSpace::Gray => ColorSpace::Gray,
        HayroColorSpace::RGB => ColorSpace::Rgb,
        HayroColorSpace::CMYK => ColorSpace::Cmyk,
        HayroColorSpace::Unknown { num_channels } => ColorSpace::Other(*num_channels),
        _ => ColorSpace::Other(image.color_space().num_channels()),
    };
    let channels = image.color_space().num_channels() + u8::from(has_alpha);
    let data = image
        .decode()
        .map_err(|e| Error::new(format!("JPEG 2000 decode failed: {e:?}")))?;

    Ok(DecodedImage {
        width,
        height,
        channels,
        color_space,
        has_alpha,
        data,
    })
}

/// Decode display pixels, converting an embedded RGB or CMYK ICC profile to sRGB.
#[cfg(feature = "color-managed-preview")]
pub fn decode_srgb_preview(bytes: &[u8], max_pixels: u64) -> Result<DecodedImage> {
    use moxcms::{ColorProfile, DataColorSpace, Layout, TransformOptions};

    let image = read_image(bytes, max_pixels)?;
    let HayroColorSpace::Icc {
        profile,
        num_channels,
    } = image.color_space()
    else {
        return decode_image(image);
    };
    let color_error = |error| Error::new(format!("JPEG 2000 ICC conversion failed: {error}"));
    let profile = ColorProfile::new_from_slice(profile).map_err(color_error)?;
    let (input, output) = match (profile.color_space, *num_channels, image.has_alpha()) {
        (DataColorSpace::Rgb, 3, false) => (Layout::Rgb, Layout::Rgb),
        (DataColorSpace::Rgb, 3, true) => (Layout::Rgba, Layout::Rgba),
        // moxcms represents a four-component CMYK source with the Rgba layout.
        (DataColorSpace::Cmyk, 4, false) => (Layout::Rgba, Layout::Rgb),
        (DataColorSpace::Cmyk, 4, true) => {
            return Err(Error::new(
                "JPEG 2000 CMYK previews with alpha are unsupported",
            ));
        }
        _ => {
            return Err(Error::new(
                "JPEG 2000 preview requires an RGB or CMYK ICC profile",
            ));
        }
    };
    let transform = profile
        .create_transform_8bit(
            input,
            &ColorProfile::new_srgb(),
            output,
            TransformOptions::default(),
        )
        .map_err(color_error)?;
    let mut decoded = decode_image(image)?;
    let channels = 3 + u8::from(decoded.has_alpha);
    let mut srgb =
        vec![0; decoded.data.len() / usize::from(decoded.channels) * usize::from(channels)];
    transform
        .transform(&decoded.data, &mut srgb)
        .map_err(color_error)?;
    decoded.data = srgb;
    decoded.channels = channels;
    decoded.color_space = ColorSpace::Rgb;
    Ok(decoded)
}

#[cfg(all(test, feature = "color-managed-preview"))]
mod tests {
    use super::*;
    use crate::{EncodeOptions, PixelFormat, encode_jp2};

    #[test]
    fn converts_embedded_rgb_profile_only_for_preview() {
        let samples = [200, 75, 50];
        let mut jp2 =
            encode_jp2(&samples, 1, 1, PixelFormat::Rgb8, &EncodeOptions::default()).unwrap();
        let profile = moxcms::ColorProfile::new_adobe_rgb().encode().unwrap();
        let header = jp2.windows(4).position(|bytes| bytes == b"jp2h").unwrap() - 4;
        let colr = jp2.windows(4).position(|bytes| bytes == b"colr").unwrap() - 4;
        let header_size = u32::from_be_bytes(jp2[header..header + 4].try_into().unwrap());
        let size = 11 + profile.len() as u32;
        let mut replacement = size.to_be_bytes().to_vec();
        replacement.extend_from_slice(b"colr\x02\x00\x00");
        replacement.extend(profile);
        jp2.splice(colr..colr + 15, replacement);
        jp2[header..header + 4].copy_from_slice(&(header_size + size - 15).to_be_bytes());

        let raw = decode(&jp2).unwrap();
        assert_eq!(raw.color_space, ColorSpace::Other(3));
        assert_eq!(raw.data, samples);
        let preview = decode_srgb_preview(&jp2, 1).unwrap();
        assert_eq!(preview.color_space, ColorSpace::Rgb);
        assert!(preview.data[0] > 215);
        assert!(preview.data[1] < 80);
        assert!(preview.data[2] < 55);
        assert!(decode_srgb_preview(&jp2, 0).is_err());
    }
}
