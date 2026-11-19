// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Shared HEIC preview decoder for the browser worker and desktop.

use zune_heic::{
    HeifDecoder,
    zune_core::{bytestream::ZCursor, colorspace::ColorSpace, options::DecoderOptions},
};

const MAX_SIDE: usize = 8192;
const MAX_PIXELS: usize = 32 * 1024 * 1024;
const MAX_BYTES: usize = 128 * 1024 * 1024;

#[cfg_attr(target_family = "wasm", wasm_bindgen::prelude::wasm_bindgen)]
pub fn decode_heic_png(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() > MAX_BYTES {
        return Err("HEIC file exceeds the preview limits".into());
    }
    let options = DecoderOptions::default()
        .set_max_width(MAX_SIDE)
        .set_max_height(MAX_SIDE)
        .set_hevc_max_mdat_size(MAX_BYTES);
    let mut decoder = HeifDecoder::new_with_options(ZCursor::new(bytes), options);
    decoder
        .decode_headers()
        .map_err(|error| error.to_string())?;
    let width = decoder.width().ok_or("Missing HEIC width")?;
    let height = decoder.height().ok_or("Missing HEIC height")?;
    if !(1..=MAX_SIDE).contains(&width)
        || !(1..=MAX_SIDE).contains(&height)
        || width * height > MAX_PIXELS
    {
        return Err("HEIC image exceeds the preview dimensions".into());
    }
    let (color, channels) = match decoder.colorspace() {
        Some(ColorSpace::RGB) => (png::ColorType::Rgb, 3),
        Some(ColorSpace::RGBA) => (png::ColorType::Rgba, 4),
        Some(ColorSpace::Luma) => (png::ColorType::Grayscale, 1),
        Some(ColorSpace::LumaA) => (png::ColorType::GrayscaleAlpha, 2),
        _ => return Err("Unsupported HEIC color layout".into()),
    };
    let mut pixels = decoder.decode().map_err(|error| error.to_string())?;
    if pixels.len() != width * height * channels {
        return Err("Invalid HEIC pixel data".into());
    }
    if decoder.is_alpha_premultiplied() && matches!(channels, 2 | 4) {
        for pixel in pixels.chunks_exact_mut(channels) {
            let alpha = u16::from(pixel[channels - 1]);
            if alpha > 0 && alpha < 255 {
                for channel in &mut pixel[..channels - 1] {
                    *channel = ((u16::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
                }
            }
        }
    }
    let mut info = png::Info::with_size(width as u32, height as u32);
    info.color_type = color;
    info.icc_profile = decoder.icc_data().map(|profile| profile.as_slice().into());
    let mut output = Vec::new();
    let encoder = png::Encoder::with_info(&mut output, info).map_err(|error| error.to_string())?;
    let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
    writer
        .write_image_data(&pixels)
        .map_err(|error| error.to_string())?;
    writer.finish().map_err(|error| error.to_string())?;
    Ok(output)
}
