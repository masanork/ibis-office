// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::collections::HashMap;

use image::RgbaImage;
use tiny_skia::{
    Color, ColorU8, FillRule, GradientStop, IntSize, LinearGradient, Paint, Path, PathBuilder,
    Pixmap, PixmapPaint, Point, Rect, SpreadMode, Stroke, StrokeDash, Transform,
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
    pen_patterns: HashMap<u16, Vec<f32>>,
    compound: Option<Compound>,
}

struct Compound {
    path: PathBuilder,
    transform: Transform,
    inverse: Transform,
    flags: u16,
}

struct Style {
    fill: Fill,
    gradient_angle: Option<f32>,
    pen: [u8; 4],
    pen_width: f32,
    pen_pattern: u16,
}

enum Fill {
    Solid(Color),
    Linear {
        angle: f32,
        start: Color,
        end: Color,
    },
}

impl Fill {
    fn paint(&self, path: &Path) -> Option<Paint<'static>> {
        let mut paint = Paint::default();
        match *self {
            Self::Solid(color) => paint.set_color(color),
            Self::Linear { angle, start, end } => {
                let bounds = path.bounds();
                let (dx, dy) = angle.sin_cos();
                let cx = (bounds.left() + bounds.right()) * 0.5;
                let cy = (bounds.top() + bounds.bottom()) * 0.5;
                let extent = (dx.abs() * bounds.width() + dy.abs() * bounds.height()) * 0.5;
                paint.shader = LinearGradient::new(
                    Point::from_xy(cx - dx * extent, cy - dy * extent),
                    Point::from_xy(cx + dx * extent, cy + dy * extent),
                    vec![GradientStop::new(0.0, start), GradientStop::new(1.0, end)],
                    SpreadMode::Pad,
                    Transform::identity(),
                )?;
            }
        }
        Some(paint)
    }
}

impl Default for Style {
    fn default() -> Self {
        Self {
            fill: Fill::Solid(Color::BLACK),
            gradient_angle: None,
            pen: [0, 0, 0, 255],
            pen_width: 0.0,
            pen_pattern: 0,
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
        if self.compound.is_some() {
            return None;
        }
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
            0x2f => {
                // Basic gradients span the full shape. Other reference points
                // and gradient flags require different stop placement.
                self.style.gradient_angle =
                    match (word(record, 4)?, word(record, 6)?, word(record, 8)?) {
                        (u16::MAX, u16::MAX, 0) => {
                            let degrees =
                                f32::from(word(record, 2)?) + f32::from(word(record, 0)?) / 65536.0;
                            Some(degrees.to_radians())
                        }
                        _ => None,
                    };
            }
            0x31 => {
                let rgba = |bytes| {
                    let [r, g, b, a] = color(bytes)?;
                    Some(Color::from_rgba8(r, g, b, a))
                };
                self.style.fill = match *record.first()? {
                    0 => Fill::Solid(rgba(record.get(1..)?)?),
                    1 if word(record, 1)? == 2 => {
                        word(record, 11)?; // The two-color form includes a position word.
                        Fill::Linear {
                            angle: self.style.gradient_angle?,
                            // WPG 2 stores the ending color first.
                            start: rgba(record.get(7..)?)?,
                            end: rgba(record.get(3..)?)?,
                        }
                    }
                    _ => return None,
                };
            }
            0x08 => {
                let id = word(record, 0)?;
                let pairs = usize::from(word(record, 2)?);
                let bytes = canvas.coordinate_bytes;
                let values = record.get(4..4 + pairs * 2 * bytes)?;
                let lengths = values
                    .chunks_exact(bytes)
                    .map(|value| {
                        if let Ok(double) = <[u8; 4]>::try_from(value) {
                            u32::from_le_bytes(double) as f32 / 65536.0
                        } else {
                            f32::from(word(value, 0).unwrap())
                        }
                    })
                    .collect();
                self.pen_patterns.insert(id, lengths);
            }
            0x29 => self.style.pen_pattern = word(record, 0)?,
            0x15 | 0x17..=0x19 => {
                let (flags, local, mut shape) = canvas.object(record)?;
                let map = parent.pre_concat(local);
                let close = self
                    .compound
                    .as_ref()
                    .map_or(0, |compound| compound.flags & 0x4000);
                let path = path(canvas, kind, flags | close, &mut shape)?;
                if let Some(compound) = &mut self.compound {
                    let contour = path.transform(compound.inverse.pre_concat(map))?;
                    compound.path.push_path(&contour);
                } else {
                    self.paint(canvas, &path, map, flags)?;
                }
            }
            _ => return None,
        }
        Some(())
    }

    pub(super) fn begin_compound(
        &mut self,
        canvas: Canvas,
        record: &[u8],
        parent: Transform,
    ) -> Option<Transform> {
        if self.compound.is_some() {
            return None;
        }
        let (flags, local, _) = canvas.object(record)?;
        let transform = parent.pre_concat(local);
        self.compound = Some(Compound {
            path: PathBuilder::new(),
            transform,
            inverse: transform.invert()?,
            flags,
        });
        Some(transform)
    }

    pub(super) fn end_compound(&mut self, canvas: Canvas) -> Option<()> {
        let compound = self.compound.take()?;
        self.paint(
            canvas,
            &compound.path.finish()?,
            compound.transform,
            compound.flags,
        )
    }

    fn paint(
        &mut self,
        canvas: Canvas,
        path: &Path,
        transform: Transform,
        flags: u16,
    ) -> Option<()> {
        self.surface(canvas)?;
        let surface = self.blend.as_mut()?;
        let map = Self::transform(surface, canvas).pre_concat(transform);
        if !map.is_finite() {
            return None;
        }
        if flags & 0x2000 != 0 {
            let paint = self.style.fill.paint(path)?;
            let rule = if flags & 0x1000 != 0 {
                FillRule::Winding
            } else {
                FillRule::EvenOdd
            };
            surface.fill_path(path, &paint, rule, map, None);
        }
        if flags & 0x8000 != 0 {
            let mut paint = Paint::default();
            let [r, g, b, a] = self.style.pen;
            paint.set_color_rgba8(r, g, b, a);
            let stroke = Stroke {
                width: self.style.pen_width,
                ..Stroke::default()
            };
            if self.style.pen_pattern == 0 {
                surface.stroke_path(path, &paint, &stroke, map, None);
            } else {
                // Match the WPG 2 pen-pattern scale used by libwpg. Pattern lengths
                // are relative to the pen width and follow the object's transform.
                let scale = self.style.pen_width * (72.0 * 3.6 / 218.0);
                let intervals = self.pen_patterns.get(&self.style.pen_pattern)?;
                let dash =
                    StrokeDash::new(intervals.iter().map(|length| length * scale).collect(), 0.0)?;
                let dashed = path.dash(&dash, 1.0)?;
                surface.stroke_path(&dashed, &paint, &stroke, map, None);
            }
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
        0x17 => {
            let count = usize::from(word(split(data, 2)?, 0)?);
            if count < 2 || count > data.len() / (6 * canvas.coordinate_bytes) {
                return None;
            }
            let mut point = || Some((coordinate(canvas, data)?, coordinate(canvas, data)?));
            point()?; // Incoming control point of the initial vertex.
            let (x, y) = point()?;
            let mut control = point()?;
            path.move_to(x, y);
            for _ in 1..count {
                let (ix, iy) = point()?;
                let (x, y) = point()?;
                path.cubic_to(control.0, control.1, ix, iy, x, y);
                control = point()?;
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
            if rx <= 0.0 || ry <= 0.0 {
                return None;
            }
            if start == end {
                path.push_oval(Rect::from_ltrb(x - rx, y - ry, x + rx, y + ry)?);
            } else {
                ellipse_arc(&mut path, (x, y), (rx, ry), start, end)?;
                if flags & 0x4000 != 0 {
                    path.close();
                }
            }
        }
        _ => return None,
    }
    path.finish()
}

fn ellipse_arc(
    path: &mut PathBuilder,
    center: (f32, f32),
    radii: (f32, f32),
    start: (f32, f32),
    end: (f32, f32),
) -> Option<()> {
    if start == (0.0, 0.0) || end == (0.0, 0.0) {
        return None;
    }
    let (rx, ry) = (f64::from(radii.0), f64::from(radii.1));
    let angle = |(x, y): (f32, f32)| (f64::from(y) / ry).atan2(f64::from(x) / rx);
    let mut from = angle(start);
    // WPG uses a y-up coordinate system. A clockwise arc decreases its angle.
    let sweep = -(from - angle(end)).rem_euclid(std::f64::consts::TAU);
    let count = (sweep.abs() / std::f64::consts::FRAC_PI_2).ceil().max(1.0) as usize;
    let step = sweep / count as f64;
    let control = 4.0 / 3.0 * (step / 4.0).tan();
    let position = |x: f64, y: f64| (center.0 + (rx * x) as f32, center.1 + (ry * y) as f32);
    let (sin, cos) = from.sin_cos();
    let (x, y) = position(cos, sin);
    path.move_to(x, y);
    for _ in 0..count {
        let (sin, cos) = from.sin_cos();
        let (s, c) = (from + step).sin_cos();
        let a = position(cos - control * sin, sin + control * cos);
        let b = position(c + control * s, s - control * c);
        let end = position(c, s);
        path.cubic_to(a.0, a.1, b.0, b.1, end.0, end.1);
        from += step;
    }
    Some(())
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
