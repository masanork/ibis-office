// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use image::RgbaImage;
use tiny_skia::{
    ColorU8, FillRule, IntSize, Paint, Path, PathBuilder, Pixmap, PixmapPaint, Rect, Stroke,
    Transform,
};

use super::{
    appearance::word,
    wpg::{Bounds, Canvas, split},
};

#[derive(Default)]
pub(super) struct Drawing {
    blend: Option<Pixmap>,
    first_bitmap: Option<(RgbaImage, Bounds)>,
    style: Style,
}

struct Style {
    fill: [u8; 4],
    pen: [u8; 4],
    pen_width: f32,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            fill: [0, 0, 0, 255],
            pen: [0, 0, 0, 255],
            pen_width: 0.0,
        }
    }
}

impl Drawing {
    pub(super) fn bitmap(
        &mut self,
        canvas: Canvas,
        bitmap: RgbaImage,
        bounds: Bounds,
        local: Transform,
    ) -> Option<()> {
        if self.blend.is_none() && self.first_bitmap.is_none() && local.is_identity() {
            self.first_bitmap = Some((bitmap, bounds));
            return Some(());
        }
        self.surface(canvas)?;
        Self::paint_bitmap(self.blend.as_mut()?, canvas, bitmap, bounds, local)
    }

    fn surface(&mut self, canvas: Canvas) -> Option<()> {
        if self.blend.is_none() {
            let size = canvas.size()?;
            // Keep a fixed physical sampling rate for vector content. Cap both
            // dimensions with the same scale to retain the canvas aspect ratio.
            let w = size.width as f64 / 3048.0;
            let h = size.height as f64 / 3048.0;
            let factor = (16_000_000.0 / (w * h))
                .sqrt()
                .min(1.0)
                .min(4096.0 / w)
                .min(4096.0 / h);
            let mut surface = Pixmap::new(
                (w * factor).round().max(1.0) as u32,
                (h * factor).round().max(1.0) as u32,
            )?;
            if let Some((bitmap, bounds)) = self.first_bitmap.take() {
                Self::paint_bitmap(&mut surface, canvas, bitmap, bounds, Transform::identity())?;
            }
            self.blend = Some(surface);
        }
        Some(())
    }

    fn transform(surface: &Pixmap, canvas: Canvas) -> Transform {
        let [left, bottom, right, top] = canvas.bounds;
        let sx = surface.width() as f32 / (right - left) as f32;
        let sy = surface.height() as f32 / (top - bottom) as f32;
        Transform::from_row(sx, 0.0, 0.0, -sy, -left as f32 * sx, top as f32 * sy)
    }

    fn paint_bitmap(
        surface: &mut Pixmap,
        canvas: Canvas,
        bitmap: RgbaImage,
        [left, bottom, right, top]: Bounds,
        local: Transform,
    ) -> Option<()> {
        let size = IntSize::from_wh(bitmap.width(), bitmap.height())?;
        let sx = (right - left) as f32 / bitmap.width() as f32;
        let sy = (bottom - top) as f32 / bitmap.height() as f32;
        let bitmap_space = Transform::from_row(sx, 0.0, 0.0, sy, left as f32, top as f32);
        let map = Self::transform(surface, canvas)
            .pre_concat(local)
            .pre_concat(bitmap_space);
        if !map.is_finite() {
            return None;
        }
        let mut rgba = bitmap.into_raw();
        for rgba in rgba.chunks_exact_mut(4) {
            let pixel = ColorU8::from_rgba(rgba[0], rgba[1], rgba[2], rgba[3]).premultiply();
            rgba.copy_from_slice(&[pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()]);
        }
        let pixmap = Pixmap::from_vec(rgba, size)?;
        surface.draw_pixmap(0, 0, pixmap.as_ref(), &PixmapPaint::default(), map, None);
        Some(())
    }

    pub(super) fn record(
        &mut self,
        canvas: Canvas,
        kind: u8,
        record: &[u8],
        parent: Transform,
    ) -> Option<()> {
        let color = |bytes: &[u8]| {
            let [r, g, b, transparency] = *bytes.first_chunk::<4>()?;
            Some([r, g, b, 255 - transparency])
        };
        match kind {
            0x25 => self.style.pen = color(record)?,
            0x2b => self.style.pen_width = f32::from(word(record, 0)?),
            0x2c => {
                self.style.pen_width = u32::from_le_bytes(record.get(..4)?.try_into().ok()?) as f32
            }
            0x31 if record.first() == Some(&0) => self.style.fill = color(record.get(1..)?)?,
            0x29 if word(record, 0)? == 0 => {}
            0x15 | 0x18 | 0x19 => {
                let (flags, local, mut shape) = canvas.object(record)?;
                let path = path(canvas, kind, flags, &mut shape)?;
                self.surface(canvas)?;
                let surface = self.blend.as_mut()?;
                let map = Self::transform(surface, canvas)
                    .pre_concat(parent)
                    .pre_concat(local);
                if !map.is_finite() {
                    return None;
                }
                if flags & 0x2000 != 0 {
                    let mut paint = Paint::default();
                    let [r, g, b, a] = self.style.fill;
                    paint.set_color_rgba8(r, g, b, a);
                    let rule = if flags & 0x1000 != 0 {
                        FillRule::Winding
                    } else {
                        FillRule::EvenOdd
                    };
                    surface.fill_path(&path, &paint, rule, map, None);
                }
                if flags & 0x8000 != 0 {
                    let mut paint = Paint::default();
                    let [r, g, b, a] = self.style.pen;
                    paint.set_color_rgba8(r, g, b, a);
                    let stroke = Stroke {
                        width: self.style.pen_width,
                        ..Stroke::default()
                    };
                    surface.stroke_path(&path, &paint, &stroke, map, None);
                }
            }
            _ => return None,
        }
        Some(())
    }

    pub(super) fn finish(self, canvas: Canvas) -> Option<(RgbaImage, ibis_doc_model::EmuSize)> {
        if let Some(surface) = self.blend {
            let mut rgba = Vec::with_capacity(surface.data().len());
            for pixel in surface.pixels() {
                let pixel = pixel.demultiply();
                rgba.extend_from_slice(&[pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()]);
            }
            return Some((
                RgbaImage::from_raw(surface.width(), surface.height(), rgba)?,
                canvas.size()?,
            ));
        }
        let (bitmap, bounds) = self.first_bitmap?;
        canvas.pad(bitmap, bounds)
    }
}

fn coordinate(canvas: Canvas, data: &mut &[u8]) -> Option<f32> {
    Some(if canvas.coordinate_bytes == 2 {
        i16::from_le_bytes(split(data, 2)?.try_into().ok()?) as f32
    } else {
        i32::from_le_bytes(split(data, 4)?.try_into().ok()?) as f32
    })
}

fn path(canvas: Canvas, kind: u8, flags: u16, data: &mut &[u8]) -> Option<Path> {
    let mut next = || coordinate(canvas, data);
    let mut path = PathBuilder::new();
    match kind {
        0x15 => {
            let count = usize::from(word(split(data, 2)?, 0)?);
            if count < 2 || count > data.len() / (2 * canvas.coordinate_bytes) {
                return None;
            }
            let mut next = || coordinate(canvas, data);
            path.move_to(next()?, next()?);
            for _ in 1..count {
                path.line_to(next()?, next()?);
            }
            if flags & 0x4000 != 0 {
                path.close();
            }
        }
        0x18 => {
            let (x0, y0, x1, y1) = (next()?, next()?, next()?, next()?);
            let bounds = Rect::from_ltrb(x0.min(x1), y0.min(y1), x0.max(x1), y0.max(y1))?;
            let (rx, ry) = (next()?, next()?);
            if rx == 0.0 || ry == 0.0 {
                return Some(PathBuilder::from_rect(bounds));
            }
            rounded_rectangle(&mut path, bounds, rx.abs(), ry.abs());
        }
        0x19 => {
            let (x, y, rx, ry) = (next()?, next()?, next()?, next()?);
            let (start, end) = ((next()?, next()?), (next()?, next()?));
            if start != end || rx <= 0.0 || ry <= 0.0 {
                return None; // A partial arc has different drawing semantics.
            }
            path.push_oval(Rect::from_ltrb(x - rx, y - ry, x + rx, y + ry)?);
        }
        _ => return None,
    }
    path.finish()
}

fn rounded_rectangle(path: &mut PathBuilder, rect: Rect, x: f32, y: f32) {
    let (left, top, right, bottom) = (rect.left(), rect.top(), rect.right(), rect.bottom());
    let (x, y) = (x.min(rect.width() / 2.0), y.min(rect.height() / 2.0));
    let (cx, cy) = (0.447_715_25 * x, 0.447_715_25 * y);
    path.move_to(left + x, top);
    path.line_to(right - x, top);
    path.cubic_to(right - cx, top, right, top + cy, right, top + y);
    path.line_to(right, bottom - y);
    path.cubic_to(right, bottom - cy, right - cx, bottom, right - x, bottom);
    path.line_to(left + x, bottom);
    path.cubic_to(left + cx, bottom, left, bottom - cy, left, bottom - y);
    path.line_to(left, top + y);
    path.cubic_to(left, top + cy, left + cx, top, left + x, top);
    path.close();
}
