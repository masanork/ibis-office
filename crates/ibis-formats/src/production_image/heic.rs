// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use core_foundation::{
    base::{CFType, CFTypeRef, TCFType},
    boolean::CFBoolean,
    data::{CFData, CFDataRef},
    dictionary::{CFDictionary, CFDictionaryRef},
    number::CFNumber,
    string::{CFString, CFStringRef},
};
use core_graphics::{
    base::kCGImageAlphaPremultipliedLast,
    color_space::{CGColorSpace, kCGColorSpaceSRGB},
    context::CGContext,
    geometry::{CGPoint, CGRect, CGSize},
    image::CGImage,
};
use foreign_types::ForeignType;
use image::RgbaImage;
use std::{ffi::c_void, ptr::null};

#[link(name = "ImageIO", kind = "framework")]
unsafe extern "C" {
    static kCGImagePropertyPixelWidth: CFStringRef;
    static kCGImagePropertyPixelHeight: CFStringRef;
    static kCGImagePropertyDepth: CFStringRef;
    static kCGImageSourceCreateThumbnailFromImageAlways: CFStringRef;
    static kCGImageSourceCreateThumbnailWithTransform: CFStringRef;
    static kCGImageSourceThumbnailMaxPixelSize: CFStringRef;
    fn CGImageSourceCreateWithData(data: CFDataRef, options: CFDictionaryRef) -> CFTypeRef;
    fn CGImageSourceGetType(source: CFTypeRef) -> CFStringRef;
    fn CGImageSourceGetPrimaryImageIndex(source: CFTypeRef) -> usize;
    fn CGImageSourceCopyPropertiesAtIndex(
        source: CFTypeRef,
        index: usize,
        options: CFDictionaryRef,
    ) -> CFDictionaryRef;
    fn CGImageSourceCreateThumbnailAtIndex(
        source: CFTypeRef,
        index: usize,
        options: CFDictionaryRef,
    ) -> *mut <CGImage as ForeignType>::CType;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        color_space: *mut <CGColorSpace as ForeignType>::CType,
        bitmap_info: u32,
    ) -> *mut <CGContext as ForeignType>::CType;
}

const MAX_SIDE: i64 = 8192;

pub(super) fn decode(bytes: &[u8]) -> Result<RgbaImage, String> {
    super::validate_heic_still(bytes)?;

    // The source and properties are retained Core Foundation objects. The
    // dictionary keys are process-lifetime Image I/O constants.
    let data = CFData::from_buffer(bytes);
    let (source, index, options) = unsafe {
        let source = CGImageSourceCreateWithData(data.as_concrete_TypeRef(), null());
        if source.is_null() {
            return Err("Cannot read HEIC image source".into());
        }
        let source = CFType::wrap_under_create_rule(source);
        let source_type = CGImageSourceGetType(source.as_CFTypeRef());
        if source_type.is_null()
            || CFString::wrap_under_get_rule(source_type) != CFString::new("public.heic")
        {
            return Err("Image I/O did not identify an HEIC still image".into());
        }
        let index = CGImageSourceGetPrimaryImageIndex(source.as_CFTypeRef());
        let properties = CGImageSourceCopyPropertiesAtIndex(source.as_CFTypeRef(), index, null());
        if properties.is_null() {
            return Err("Cannot read HEIC primary image properties".into());
        }
        let properties = CFDictionary::<CFString, CFType>::wrap_under_create_rule(properties);
        let number = |key| {
            properties
                .find(&CFString::wrap_under_get_rule(key))
                .and_then(|value| value.downcast::<CFNumber>())
                .and_then(|value| value.to_i64())
        };
        let width = number(kCGImagePropertyPixelWidth).unwrap_or(0);
        let height = number(kCGImagePropertyPixelHeight).unwrap_or(0);
        if !bounded(width, height) {
            return Err("HEIC dimensions exceed the preview limits".into());
        }
        if number(kCGImagePropertyDepth) != Some(8) {
            return Err("HEIC preview requires an 8-bit primary image".into());
        }
        let yes = CFBoolean::true_value().as_CFType();
        let options = CFDictionary::from_CFType_pairs(&[
            (
                CFString::wrap_under_get_rule(kCGImageSourceCreateThumbnailFromImageAlways),
                yes.clone(),
            ),
            (
                CFString::wrap_under_get_rule(kCGImageSourceCreateThumbnailWithTransform),
                yes,
            ),
            (
                CFString::wrap_under_get_rule(kCGImageSourceThumbnailMaxPixelSize),
                CFNumber::from(width.max(height)).as_CFType(),
            ),
        ]);
        (source, index, options)
    };
    let image = unsafe {
        let image = CGImageSourceCreateThumbnailAtIndex(
            source.as_CFTypeRef(),
            index,
            options.as_concrete_TypeRef(),
        );
        if image.is_null() {
            return Err("Cannot decode the HEIC primary image".into());
        }
        CGImage::from_ptr(image)
    };
    let (width, height) = (image.width(), image.height());
    if !bounded(width as i64, height as i64) {
        return Err("Decoded HEIC image exceeds the preview limits".into());
    }
    let color_space = CGColorSpace::create_with_name(unsafe { kCGColorSpaceSRGB })
        .ok_or("Cannot create the HEIC preview color space")?;
    let len = width * height * 4;
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(len)
        .map_err(|_| "Cannot allocate the HEIC preview")?;
    pixels.resize(len, 0);
    {
        // Core Graphics borrows the buffer until the context is dropped.
        let bitmap = unsafe {
            let ptr = CGBitmapContextCreate(
                pixels.as_mut_ptr().cast(),
                width,
                height,
                8,
                width * 4,
                color_space.as_ptr(),
                kCGImageAlphaPremultipliedLast,
            );
            if ptr.is_null() {
                return Err("Cannot create the HEIC preview bitmap".into());
            }
            CGContext::from_ptr(ptr)
        };
        bitmap.draw_image(
            CGRect::new(
                &CGPoint::new(0.0, 0.0),
                &CGSize::new(width as f64, height as f64),
            ),
            &image,
        );
    }
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        if (1..255).contains(&alpha) {
            for channel in &mut pixel[..3] {
                *channel = ((u16::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
    RgbaImage::from_raw(width as u32, height as u32, pixels)
        .ok_or_else(|| "Cannot construct the HEIC preview image".into())
}

fn bounded(width: i64, height: i64) -> bool {
    (1..=MAX_SIDE).contains(&width)
        && (1..=MAX_SIDE).contains(&height)
        && (width * height) as u64 <= super::MAX_PIXELS
}
