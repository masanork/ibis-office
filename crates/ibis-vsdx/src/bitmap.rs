// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::io::Cursor;

use ibis_formats::zip::Archive;
use ibis_ooxml::Relationship;
use ibis_pdf::{DisplayCmd, PdfRect};
use image::{ImageFormat, ImageReader, Limits, RgbaImage};
use roxmltree::Node;
use tiny_skia::{ColorU8, FilterQuality, Pixmap, PixmapPaint, Transform};

use super::{MAX_PART, Matrix, rel_id, relationships};

const IMAGE_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
const MAX_PAGE_ENCODED: usize = 16 * 1024 * 1024;
const MAX_PAGE_PIXELS: usize = 4_194_304;

pub(super) struct Bitmaps<'a, 'i> {
    archive: &'a Archive<'i>,
    page: &'a str,
    rels: Vec<Relationship>,
    remaining_pixels: usize,
    remaining_encoded: usize,
}

impl<'a, 'i> Bitmaps<'a, 'i> {
    pub(super) fn new(archive: &'a Archive<'i>, page: &'a str) -> Self {
        let rels = if archive.contains(&ibis_ooxml::package::relationships_part(page)) {
            relationships(archive, page).unwrap_or_default()
        } else {
            Vec::new()
        };
        Self {
            archive,
            page,
            rels,
            remaining_pixels: MAX_PAGE_PIXELS,
            remaining_encoded: MAX_PAGE_ENCODED,
        }
    }

    pub(super) fn draw(
        &mut self,
        foreign: Node<'_, '_>,
        transform: Matrix,
        (x, y, width, height): (f64, f64, f64, f64),
    ) -> Option<DisplayCmd> {
        let [a, b, c, d, _, _] = transform.0;
        if width <= 0.0
            || height <= 0.0
            || self.remaining_pixels == 0
            || self.remaining_encoded == 0
        {
            return None;
        }
        if foreign.attribute("ForeignType") != Some("Bitmap") {
            return None;
        }
        let id = rel_id(foreign)?;
        let target = self
            .rels
            .iter()
            .find(|rel| rel.id == id && rel.kind == IMAGE_REL)?
            .target_part(self.page)?;
        let bytes = self
            .archive
            .read_bounded(&target, MAX_PART.min(self.remaining_encoded))
            .ok()??;
        self.remaining_encoded -= bytes.len();
        let format = match image::guess_format(&bytes).ok()? {
            format @ (ImageFormat::Png | ImageFormat::Jpeg) => format,
            _ => return None,
        };
        let reader = || ImageReader::with_format(Cursor::new(&bytes), format);
        let (pixels_x, pixels_y) = reader().into_dimensions().ok()?;
        let count = usize::try_from(u64::from(pixels_x) * u64::from(pixels_y)).ok()?;
        if count == 0 || count > self.remaining_pixels {
            return None;
        }
        // Account for attempted decoding and for each copy retained in the page scene.
        self.remaining_pixels -= count;
        let mut decoder = reader();
        let mut limits = Limits::default();
        limits.max_image_width = Some(4096);
        limits.max_image_height = Some(4096);
        limits.max_alloc = Some((MAX_PAGE_PIXELS * 8) as u64);
        decoder.limits(limits);
        let mut image = decoder.decode().ok()?.into_rgba8();
        if b.abs() > 1e-7 || c.abs() > 1e-7 {
            let (output, bounds) = transform_bitmap(
                image,
                transform,
                (x, y, width, height),
                &mut self.remaining_pixels,
            )?;
            return Some(DisplayCmd::DrawImage {
                bounds,
                width: output.width(),
                height: output.height(),
                rgba: output.into_raw(),
            });
        }
        if a < 0.0 {
            image::imageops::flip_horizontal_in_place(&mut image);
        }
        if d > 0.0 {
            image::imageops::flip_vertical_in_place(&mut image);
        }
        let (left, bottom) = transform.point(x, y);
        let (right, top) = transform.point(x + width, y + height);
        Some(DisplayCmd::DrawImage {
            bounds: PdfRect {
                x0: left.min(right),
                y0: top.min(bottom),
                x1: left.max(right),
                y1: top.max(bottom),
            },
            width: pixels_x,
            height: pixels_y,
            rgba: image.into_raw(),
        })
    }
}

fn transform_bitmap(
    image: RgbaImage,
    transform: Matrix,
    (x, y, width, height): (f64, f64, f64, f64),
    remaining_pixels: &mut usize,
) -> Option<(RgbaImage, PdfRect)> {
    let [a, b, c, d, tx, ty] = transform.0;
    let (input_w, input_h) = image.dimensions();
    // Source pixels advance right and down from the image's upper-left corner.
    let (ux, uy) = (a * width, b * width);
    let (vx, vy) = (-c * height, -d * height);
    let (origin_x, origin_y) = (a * x + c * (y + height) + tx, b * x + d * (y + height) + ty);
    let (min_x, min_y) = (ux.min(0.0) + vx.min(0.0), uy.min(0.0) + vy.min(0.0));
    let (span_x, span_y) = (ux.abs() + vx.abs(), uy.abs() + vy.abs());
    let density_u = f64::from(input_w) / ux.hypot(uy);
    let density_v = f64::from(input_h) / vx.hypot(vy);
    if !density_u.is_finite() || !density_v.is_finite() || span_x <= 0.0 || span_y <= 0.0 {
        return None;
    }
    let raster_size = |u: f64, v: f64| {
        ((u.abs() * density_u + v.abs() * density_v - 1e-7)
            .ceil()
            .max(1.0)) as u32
    };
    let (output_w, output_h) = (raster_size(ux, vx), raster_size(uy, vy));
    let count = usize::try_from(u64::from(output_w) * u64::from(output_h)).ok()?;
    if count > *remaining_pixels || output_w > 4096 || output_h > 4096 {
        return None;
    }
    *remaining_pixels -= count;
    let mut source = Pixmap::new(input_w, input_h)?;
    for (input, output) in image.pixels().zip(source.pixels_mut()) {
        let [r, g, b, a] = input.0;
        *output = ColorU8::from_rgba(r, g, b, a).premultiply();
    }
    let mut output = Pixmap::new(output_w, output_h)?;
    let (sx, sy) = (f64::from(output_w) / span_x, f64::from(output_h) / span_y);
    output.draw_pixmap(
        0,
        0,
        source.as_ref(),
        &PixmapPaint {
            quality: FilterQuality::Bilinear,
            ..Default::default()
        },
        Transform::from_row(
            (ux * sx / f64::from(input_w)) as f32,
            (uy * sy / f64::from(input_w)) as f32,
            (vx * sx / f64::from(input_h)) as f32,
            (vy * sy / f64::from(input_h)) as f32,
            (-min_x * sx) as f32,
            (-min_y * sy) as f32,
        ),
        None,
    );
    let mut rgba = RgbaImage::new(output_w, output_h);
    for (input, pixel) in output.pixels().iter().zip(rgba.pixels_mut()) {
        let color = input.demultiply();
        pixel.0 = [color.red(), color.green(), color.blue(), color.alpha()];
    }
    let bounds = PdfRect {
        x0: (origin_x + min_x) as f32,
        y0: (origin_y + min_y) as f32,
        x1: (origin_x + min_x + span_x) as f32,
        y1: (origin_y + min_y + span_y) as f32,
    };
    Some((rgba, bounds))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn right_angle_rotation_keeps_orientation_and_alpha() {
        let source = RgbaImage::from_fn(2, 1, |x, _| {
            if x == 0 {
                Rgba([255, 0, 0, 255])
            } else {
                Rgba([0, 128, 0, 128])
            }
        });
        let mut budget = 2;
        let (output, bounds) = transform_bitmap(
            source,
            Matrix([0.0, -72.0, -72.0, 0.0, 400.0, 300.0]),
            (0.0, 0.0, 2.0, 1.0),
            &mut budget,
        )
        .unwrap();
        assert_eq!(output.dimensions(), (1, 2));
        assert_eq!(
            (bounds.x0, bounds.y0, bounds.x1, bounds.y1),
            (328.0, 156.0, 400.0, 300.0)
        );
        assert_eq!(output.get_pixel(0, 0)[3], 128);
        assert!(output.get_pixel(0, 0)[1] >= 127);
        assert_eq!(output.get_pixel(0, 1).0, [255, 0, 0, 255]);
        assert_eq!(budget, 0);
    }

    #[test]
    fn rejects_a_transformed_image_that_exceeds_the_remaining_budget() {
        let source = RgbaImage::new(16, 16);
        let mut budget = 16;
        let angle = 45.0_f64.to_radians();
        let (sine, cosine) = angle.sin_cos();
        assert!(
            transform_bitmap(
                source,
                Matrix([cosine, sine, -sine, cosine, 0.0, 0.0]),
                (0.0, 0.0, 1.0, 1.0),
                &mut budget,
            )
            .is_none()
        );
        assert_eq!(budget, 16);
    }
}
