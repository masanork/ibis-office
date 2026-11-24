// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use super::{MAX_PAGE_PATH_COMMANDS, Matrix, PathCmd};

type Point = (f32, f32);

pub(super) fn straight_vertices(commands: &[PathCmd]) -> Option<Vec<Point>> {
    let (first, rest) = commands.split_first()?;
    let PathCmd::MoveTo(x, y) = *first else {
        return None;
    };
    let mut points = vec![(x, y)];
    for command in rest {
        match *command {
            PathCmd::LineTo(x, y) => points.push((x, y)),
            PathCmd::Close => {}
            _ => return None,
        }
    }
    Some(points)
}

fn mix(a: Point, b: Point, t: f32) -> Point {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

fn distance(a: Point, b: Point) -> f32 {
    (b.0 - a.0).hypot(b.1 - a.1)
}

pub(super) fn rounded_polygon(points: &[Point], radius: f32) -> Vec<PathCmd> {
    let mut out = Vec::new();
    for (i, &corner) in points.iter().enumerate() {
        let before = points[(i + points.len() - 1) % points.len()];
        let after = points[(i + 1) % points.len()];
        let (a, b) = (
            mix(corner, before, (radius / distance(corner, before)).min(0.5)),
            mix(corner, after, (radius / distance(corner, after)).min(0.5)),
        );
        out.push(if i == 0 {
            PathCmd::MoveTo(a.0, a.1)
        } else {
            PathCmd::LineTo(a.0, a.1)
        });
        let c1 = mix(a, corner, 0.5522848);
        let c2 = mix(b, corner, 0.5522848);
        out.push(PathCmd::CubicTo(c1.0, c1.1, c2.0, c2.1, b.0, b.1));
    }
    out.push(PathCmd::Close);
    out
}

pub(super) fn dashed_line(points: &[Point], weight: f32) -> Vec<PathCmd> {
    let unit = weight.max(0.5);
    let mut out = Vec::new();
    let mut drawing = true;
    let mut remaining = 3.0 * unit;
    for segment in points.windows(2) {
        let [a, b] = [segment[0], segment[1]];
        let length = distance(a, b);
        let mut offset = 0.0;
        while offset < length && out.len() < MAX_PAGE_PATH_COMMANDS {
            let step = remaining.min(length - offset);
            let end = offset + step;
            if end <= offset {
                break;
            }
            if drawing {
                let p = mix(a, b, offset / length);
                let q = mix(a, b, end / length);
                out.extend([PathCmd::MoveTo(p.0, p.1), PathCmd::LineTo(q.0, q.1)]);
            }
            if step == remaining {
                drawing = !drawing;
                remaining = (if drawing { 3.0 } else { 2.0 }) * unit;
            } else {
                remaining -= step;
            }
            offset = end;
        }
    }
    out
}

pub(super) fn triangle_arrow(tip: Point, adjacent: Point, weight: f32) -> Vec<PathCmd> {
    let length = distance(tip, adjacent);
    if length == 0.0 {
        return Vec::new();
    }
    let back = mix(tip, adjacent, ((weight * 6.0).max(3.0) / length).min(1.0));
    let half_width = (weight * 2.5).max(1.5) / length;
    let normal = (
        (adjacent.1 - tip.1) * half_width,
        (tip.0 - adjacent.0) * half_width,
    );
    vec![
        PathCmd::MoveTo(tip.0, tip.1),
        PathCmd::LineTo(back.0 + normal.0, back.1 + normal.1),
        PathCmd::LineTo(back.0 - normal.0, back.1 - normal.1),
        PathCmd::Close,
    ]
}

pub(super) fn elliptical_arc(
    matrix: Matrix,
    from: (f64, f64),
    through: (f64, f64),
    to: (f64, f64),
    angle: f64,
    ratio: f64,
) -> Option<Vec<PathCmd>> {
    if !(0.001..=1000.0).contains(&ratio) {
        return None;
    }
    // Map the ellipse to a circle, then find the circle through the three vertices.
    let (sin, cos) = angle.sin_cos();
    let circle = |(x, y): (f64, f64)| (cos * x + sin * y, (-sin * x + cos * y) * ratio);
    let a = circle(from);
    let b = circle(through);
    let c = circle(to);
    let (bx, by, cx, cy) = (b.0 - a.0, b.1 - a.1, c.0 - a.0, c.1 - a.1);
    let det = 2.0 * (bx * cy - by * cx);
    if det.abs() < 1e-12 {
        return None;
    }
    let (bb, cc) = (bx * bx + by * by, cx * cx + cy * cy);
    let center = (
        a.0 + (cy * bb - by * cc) / det,
        a.1 + (bx * cc - cx * bb) / det,
    );
    let radius = (a.0 - center.0).hypot(a.1 - center.1);
    if radius > 1_000_000.0 {
        return None;
    }
    let theta = |p: (f64, f64)| (p.1 - center.1).atan2(p.0 - center.0);
    let start = theta(a);
    let turn = std::f64::consts::TAU;
    let mut sweep = (theta(c) - start).rem_euclid(turn);
    if (theta(b) - start).rem_euclid(turn) > sweep {
        sweep -= turn;
    }
    let steps = (sweep.abs() / std::f64::consts::FRAC_PI_2 - 1e-10)
        .ceil()
        .max(1.0) as usize;
    let step = sweep / steps as f64;
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    let point = |t: f64, tangent: f64| {
        let (s, c) = t.sin_cos();
        let x = center.0 + radius * (c - tangent * s);
        let y = (center.1 + radius * (s + tangent * c)) / ratio;
        matrix.point(cos * x - sin * y, sin * x + cos * y)
    };
    Some(
        (0..steps)
            .map(|i| {
                let t = start + step * i as f64;
                let a = point(t, k);
                let b = point(t + step, -k);
                let end = if i + 1 == steps {
                    matrix.point(to.0, to.1)
                } else {
                    point(t + step, 0.0)
                };
                PathCmd::CubicTo(a.0, a.1, b.0, b.1, end.0, end.1)
            })
            .collect(),
    )
}
