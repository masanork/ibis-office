// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use ibis_pdf::{DisplayCmd, PdfRect};
use tiny_skia::{
    Color, FillRule, GradientStop, LinearGradient, Paint, PathBuilder, Pixmap, Point, SpreadMode,
    Transform,
};

use super::{Matrix, PathCmd};

pub(super) const PAGE_PIXELS: usize = 4_194_304;

pub(super) struct Gradient {
    pub(super) stops: Vec<GradientStop>,
    pub(super) direction: (f32, f32),
}

pub(super) fn direction(angle: f64, transform: Option<Matrix>) -> (f32, f32) {
    let (sin, cos) = angle.sin_cos();
    if let Some(Matrix([a, b, c, d, _, _])) = transform {
        (
            (a * cos + c * sin) as f32 / 72.0,
            (b * cos + d * sin) as f32 / 72.0,
        )
    } else {
        (cos as f32, -sin as f32)
    }
}

pub(super) fn stop(position: f64, rgba: [u8; 4]) -> GradientStop {
    let [r, g, b, a] = rgba;
    GradientStop::new(position as f32, Color::from_rgba8(r, g, b, a))
}

impl Gradient {
    pub(super) fn raster(&self, commands: &[PathCmd], remaining: &mut usize) -> Option<DisplayCmd> {
        let mut builder = PathBuilder::new();
        for cmd in commands {
            match *cmd {
                PathCmd::MoveTo(x, y) => builder.move_to(x, y),
                PathCmd::LineTo(x, y) => builder.line_to(x, y),
                PathCmd::CubicTo(x1, y1, x2, y2, x, y) => builder.cubic_to(x1, y1, x2, y2, x, y),
                PathCmd::Close => builder.close(),
                _ => return None,
            }
        }
        let path = builder.finish()?;
        let bounds = path.bounds();
        let (w, h) = (bounds.width(), bounds.height());
        if w <= 0.0 || h <= 0.0 {
            return None;
        }
        let scale = 2.0f32.min(1024.0 / w.max(h));
        let (width, height) = ((w * scale).ceil() as u32, (h * scale).ceil() as u32);
        let count = (width as usize).checked_mul(height as usize)?;
        if count == 0 || count > *remaining {
            return None;
        }
        *remaining -= count;
        let (dx, dy) = self.direction;
        let (cx, cy) = (bounds.left() + w / 2.0, bounds.top() + h / 2.0);
        let extent = (w * dx.abs() + h * dy.abs()) / 2.0;
        let mut paint = Paint::default();
        paint.shader = LinearGradient::new(
            Point::from_xy(cx - dx * extent, cy - dy * extent),
            Point::from_xy(cx + dx * extent, cy + dy * extent),
            self.stops.clone(),
            SpreadMode::Pad,
            Transform::identity(),
        )?;
        let sx = width as f32 / w;
        let sy = height as f32 / h;
        let mut image = Pixmap::new(width, height)?;
        image.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::from_row(sx, 0.0, 0.0, sy, -bounds.left() * sx, -bounds.top() * sy),
            None,
        );
        let rgba = image
            .pixels()
            .iter()
            .flat_map(|color| {
                let c = color.demultiply();
                [c.red(), c.green(), c.blue(), c.alpha()]
            })
            .collect();
        Some(DisplayCmd::DrawImage {
            bounds: PdfRect {
                x0: bounds.left(),
                y0: bounds.top(),
                x1: bounds.right(),
                y1: bounds.bottom(),
            },
            width,
            height,
            rgba,
        })
    }
}
