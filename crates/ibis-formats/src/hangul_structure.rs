// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use ibis_doc_model::{
    AbstractList, Block, EmuRect, EmuSize, FloatingAnchor, FlowAlign, FlowDocument, FlowParagraph,
    FlowRun, FlowRunKind, ListInstance, ListLevel, ListSuffix, MediaId, Numbering, PartKind,
    Picture, PictureLook, SectionSetup, Shape, ShapeId, ShapeKind,
};

pub(super) fn add_list(numbering: &mut Numbering, levels: Vec<ListLevel>) -> u32 {
    let id = numbering.nums.len() as u32 + 1;
    numbering.abstracts.insert(
        id,
        AbstractList {
            levels,
            num_style_link: None,
        },
    );
    numbering.nums.insert(
        id,
        ListInstance {
            abstract_id: id,
            ..Default::default()
        },
    );
    id
}

pub(super) fn list_text(pattern: &str, level: u32) -> String {
    let mut output = String::new();
    let mut chars = pattern.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '^' {
            match chars.peek().copied() {
                Some(n @ '1'..='9') => {
                    chars.next();
                    output.push('%');
                    output.push(n);
                    continue;
                }
                Some(n @ ('n' | 'N')) => {
                    chars.next();
                    for depth in 1..=level.min(9) {
                        if depth != 1 {
                            output.push('.');
                        }
                        output.push('%');
                        output.push(char::from_digit(depth, 10).unwrap());
                    }
                    if n == 'N' {
                        output.push('.');
                    }
                    continue;
                }
                _ => {}
            }
        }
        output.push(ch);
    }
    output
}

pub(super) fn bullet_level(ch: char) -> ListLevel {
    let (text, symbol) = match ch {
        '\u{f06c}' => ('●', Some(("Wingdings".into(), ch))),
        '\u{f06e}' => ('■', Some(("Wingdings".into(), ch))),
        _ => (ch, None),
    };
    ListLevel {
        start: 1,
        format: "bullet".into(),
        text: text.to_string(),
        suffix: ListSuffix::Space,
        symbol,
        ..Default::default()
    }
}

pub(super) fn number_format(code: u32) -> &'static str {
    match code {
        1 => "decimalEnclosedCircle",
        2 => "upperRoman",
        3 => "lowerRoman",
        4 => "upperLetter",
        5 => "lowerLetter",
        8 => "ganada",
        10 => "chosung",
        12 => "koreanCounting",
        13 => "ideographDigital",
        _ => "decimal",
    }
}

pub(super) fn page_number(id: u32, align: FlowAlign, prefix: &str, suffix: &str) -> Block {
    let mut p = FlowParagraph {
        id,
        ..Default::default()
    };
    p.direct.align = Some(align);
    if !prefix.is_empty() {
        p.runs.push(FlowRun::new(
            FlowRunKind::Text(prefix.into()),
            Default::default(),
        ));
    }
    p.runs
        .push(FlowRun::new(FlowRunKind::PageNumber, Default::default()));
    if !suffix.is_empty() {
        p.runs.push(FlowRun::new(
            FlowRunKind::Text(suffix.into()),
            Default::default(),
        ));
    }
    p.resolve(&Default::default());
    Block::Paragraph(p)
}

pub(super) fn set_last_section(doc: &mut FlowDocument, section: SectionSetup) {
    doc.page = section.page;
    doc.restart_numbering = section.restart_numbering;
    doc.page_number_format = section.page_number_format;
    for (i, part) in section.parts.into_iter().enumerate() {
        doc.linked[i] = part.is_none();
        let target = match PartKind::ALL[i] {
            PartKind::Header => &mut doc.header,
            PartKind::Footer => &mut doc.footer,
            PartKind::HeaderFirst => &mut doc.header_first,
            PartKind::FooterFirst => &mut doc.footer_first,
            PartKind::HeaderEven => &mut doc.header_even,
            PartKind::FooterEven => &mut doc.footer_even,
        };
        *target = part.unwrap_or_default();
    }
}

pub(super) fn imported_picture(
    media: MediaId,
    size: EmuSize,
    look: PictureLook,
    floating: Option<(ShapeId, FloatingAnchor)>,
) -> FlowRunKind {
    let Some((id, floating)) = floating else {
        return FlowRunKind::Picture { media, size, look };
    };
    let mut picture = Picture::new(media);
    picture.crop = look.crop;
    picture.alt_text = look.alt_text;
    let mut shape = Shape::new(
        id,
        EmuRect::new(
            floating.offset_x,
            floating.offset_y,
            size.width,
            size.height,
        ),
        ShapeKind::Picture(picture),
    );
    shape.rotation = look.rotation;
    shape.flip_h = look.flip_h;
    shape.flip_v = look.flip_v;
    shape.floating = Some(floating);
    FlowRunKind::Shape(Box::new(shape))
}

pub(crate) fn group_anchor() -> FloatingAnchor {
    FloatingAnchor {
        relative_h: "column".into(),
        relative_v: "paragraph".into(),
        wrap: "none".into(),
        allow_overlap: true,
        layout_in_cell: true,
        relative_height: FloatingAnchor::DEFAULT_RELATIVE_HEIGHT,
        ..Default::default()
    }
}

pub(crate) fn offset_anchor(parent: &FloatingAnchor, x: i64, y: i64) -> FloatingAnchor {
    let mut anchor = parent.clone();
    anchor.offset_x = anchor.offset_x.saturating_add(x);
    anchor.offset_y = anchor.offset_y.saturating_add(y);
    anchor
}

pub(crate) fn shape_group(
    children: Vec<ibis_doc_model::GroupMember>,
) -> ibis_doc_model::ShapeGroup {
    ibis_doc_model::ShapeGroup {
        children,
        ..ibis_doc_model::ShapeGroup::from_shape_ids("", "", &[])
    }
}

// The first matrix supplies the translation in group coordinates. The
// remaining matrices transform the original (unscaled) leaf geometry.
pub(super) fn place_group_leaf(
    shape: &mut Shape,
    anchor: &FloatingAnchor,
    original: EmuSize,
    matrices: impl IntoIterator<Item = [f64; 6]>,
    warp: impl FnOnce(&Picture, EmuSize, EmuRect, [f64; 6]) -> Option<MediaId>,
) -> Option<()> {
    let mut matrices = matrices.into_iter();
    let translation = matrices.next()?;
    let [mut a, mut b, mut x, mut c, mut d, mut y] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    for [aa, bb, xx, cc, dd, yy] in matrices {
        (a, b, x, c, d, y) = (
            aa * a + bb * c,
            aa * b + bb * d,
            aa * x + bb * y + xx,
            cc * a + dd * c,
            cc * b + dd * d,
            cc * x + dd * y + yy,
        );
    }
    let sx = a.hypot(c);
    let sy = b.hypot(d);
    if !(1e-6..=1000.0).contains(&sx) || !(1e-6..=1000.0).contains(&sy) {
        return None;
    }
    let determinant = a * d - b * c;
    let width = original.width as f64;
    let height = original.height as f64;
    let tx = (x + translation[2]) * 127.0;
    let ty = (y + translation[5]) * 127.0;
    // DrawingML's rotation and scale cannot express nonorthogonal axes.
    let matrix = [a, b, tx, c, d, ty];
    if (a * b + c * d).abs() > sx * sy * 1e-5
        && let Some(frame) = projected_frame([0.0, 0.0, width, height], matrix)
    {
        let baked = match &mut shape.kind {
            ShapeKind::Auto(auto) => {
                if let Some(geometry) = shear_outline(&auto.geometry, original, frame, matrix) {
                    auto.geometry = geometry;
                    if let Some(text) = &mut auto.text {
                        let i = &mut text.insets;
                        if let Some(area) = projected_frame(
                            [
                                i.left as f64,
                                i.top as f64,
                                width - i.right as f64,
                                height - i.bottom as f64,
                            ],
                            matrix,
                        ) {
                            let line_width =
                                ((width - i.left as f64 - i.right as f64) * sx).round() as i64;
                            i.left = (area.x - frame.x).max(0);
                            i.top = (area.y - frame.y).max(0);
                            i.right = (frame.right() - area.right()).max(0);
                            if text.wrap && line_width > 0 {
                                i.right = i.right.max(frame.width - i.left - line_width);
                            }
                            i.bottom = (frame.bottom() - area.bottom()).max(0);
                        }
                    }
                    true
                } else {
                    false
                }
            }
            ShapeKind::Picture(pic) => warp(pic, original, frame, matrix).is_some_and(|media| {
                pic.media = media;
                pic.crop = Default::default();
                true
            }),
            _ => false,
        };
        if baked {
            shape.frame = EmuRect::new(0, 0, frame.width, frame.height);
            shape.rotation = 0;
            shape.flip_h = false;
            shape.flip_v = false;
            shape.floating = Some(offset_anchor(anchor, frame.x, frame.y));
            return Some(());
        }
    }
    let cx = (a * width + b * height) / 2.0 + (x + translation[2]) * 127.0;
    let cy = (c * width + d * height) / 2.0 + (y + translation[5]) * 127.0;
    let bounded = |n: f64| n.round().clamp(-127_000_000.0, 127_000_000.0) as i64;
    shape.frame.width = bounded(width * sx).max(1);
    shape.frame.height = bounded(height * (determinant / sx).abs()).max(1);
    shape.rotation = ((c.atan2(a).to_degrees() * 60_000.0).round() as i32).rem_euclid(21_600_000);
    shape.flip_h = false;
    shape.flip_v = determinant < 0.0;
    shape.floating = Some(offset_anchor(
        anchor,
        bounded(cx - shape.frame.width as f64 / 2.0),
        bounded(cy - shape.frame.height as f64 / 2.0),
    ));
    Some(())
}

fn projected_frame([x0, y0, x1, y1]: [f64; 4], [a, b, tx, c, d, ty]: [f64; 6]) -> Option<EmuRect> {
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let project = |x: f64, y: f64| (a * x + b * y + tx, c * x + d * y + ty);
    let corners = [
        project(x0, y0),
        project(x1, y0),
        project(x0, y1),
        project(x1, y1),
    ];
    let extrema = |axis: fn((f64, f64)) -> f64| {
        let mut values = corners.into_iter().map(axis);
        let first = values.next().unwrap();
        values.fold((first, first), |(lo, hi), v| (lo.min(v), hi.max(v)))
    };
    let (left, right) = extrema(|p| p.0);
    let (top, bottom) = extrema(|p| p.1);
    if [left, right, top, bottom]
        .into_iter()
        .any(|v| !v.is_finite() || v.abs() > 127_000_000.0)
    {
        return None;
    }
    let left = left.floor();
    let top = top.floor();
    Some(EmuRect::new(
        left as i64,
        top as i64,
        ((right.ceil() - left) as i64).max(1),
        ((bottom.ceil() - top) as i64).max(1),
    ))
}

pub(super) fn warp_group_picture(
    presentation: &mut ibis_doc_model::Presentation,
    remaining: &mut usize,
    pic: &Picture,
    original: EmuSize,
    frame: EmuRect,
    [a, b, tx, c, d, ty]: [f64; 6],
) -> Option<MediaId> {
    use image::{ImageEncoder, ImageReader, Rgba, RgbaImage};
    let determinant = a * d - b * c;
    if *remaining < 128 || determinant.abs() < 1e-9 || pic.crop != Default::default() {
        return None;
    }
    let media = presentation.media(pic.media)?;
    let format = match media.format {
        ibis_doc_model::MediaFormat::Png => image::ImageFormat::Png,
        ibis_doc_model::MediaFormat::Jpeg => image::ImageFormat::Jpeg,
        ibis_doc_model::MediaFormat::Bmp => image::ImageFormat::Bmp,
        _ => return None,
    };
    let reader = || ImageReader::with_format(std::io::Cursor::new(media.data.as_ref()), format);
    let (input_w, input_h) = reader().into_dimensions().ok()?;
    let input_cost: usize = (u64::from(input_w) * u64::from(input_h) * 4)
        .try_into()
        .ok()?;
    if input_w > 4096 || input_h > 4096 || input_cost > *remaining / 2 {
        return None;
    }
    *remaining -= input_cost;
    let mut reader = reader();
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let source = reader.decode().ok()?.to_rgba8();
    let (sw, sh) = (f64::from(source.width()), f64::from(source.height()));
    let (fw, fh) = (frame.width as f64, frame.height as f64);
    let (ow, oh) = (original.width as f64, original.height as f64);
    let density = (sw / (ow * a.hypot(c))).max(sh / (oh * b.hypot(d)));
    let budget = ((*remaining / 4).min(4 * 1024 * 1024)) as f64;
    let density = density
        .min(4096.0 / fw.max(fh))
        .min((budget / (fw * fh)).sqrt());
    let (w, h) = (
        (fw * density).floor().max(1.0) as u32,
        (fh * density).floor().max(1.0) as u32,
    );
    let output_cost = (w as usize * h as usize * 4).min(*remaining);
    *remaining -= output_cost;
    let mut output = RgbaImage::new(w, h);
    for (px, py, out) in output.enumerate_pixels_mut() {
        let x = frame.x as f64 + (f64::from(px) + 0.5) * fw / f64::from(w) - tx;
        let y = frame.y as f64 + (f64::from(py) + 0.5) * fh / f64::from(h) - ty;
        let u = (d * x - b * y) / (determinant * ow);
        let v = (a * y - c * x) / (determinant * oh);
        if (0.0..1.0).contains(&u) && (0.0..1.0).contains(&v) {
            // Interpolate in premultiplied alpha to keep transparent edges clean.
            let sx = (u * sw - 0.5).clamp(0.0, sw - 1.0);
            let sy = (v * sh - 0.5).clamp(0.0, sh - 1.0);
            let mut color = [0.0; 4];
            for (ix, wx) in [(sx.floor(), 1.0 - sx.fract()), (sx.ceil(), sx.fract())] {
                for (iy, wy) in [(sy.floor(), 1.0 - sy.fract()), (sy.ceil(), sy.fract())] {
                    let rgba = source.get_pixel(ix as u32, iy as u32).0;
                    let weight = f64::from(rgba[3]) * wx * wy;
                    for i in 0..3 {
                        color[i] += f64::from(rgba[i]) * weight;
                    }
                    color[3] += weight;
                }
            }
            if color[3] > 0.0 {
                *out = Rgba([
                    (color[0] / color[3]).round() as u8,
                    (color[1] / color[3]).round() as u8,
                    (color[2] / color[3]).round() as u8,
                    color[3].round() as u8,
                ]);
            }
        }
    }
    let mut encoded = Vec::new();
    image::codecs::png::PngEncoder::new(&mut encoded)
        .write_image(output.as_raw(), w, h, image::ExtendedColorType::Rgba8)
        .ok()?;
    if encoded.len() > *remaining {
        return None;
    }
    *remaining -= encoded.len();
    Some(presentation.add_media(ibis_doc_model::MediaFormat::Png, encoded))
}

/// Bake a sheared vector into editable DrawingML paths.
fn shear_outline(
    geometry: &ibis_doc_model::Geometry,
    size: EmuSize,
    frame: EmuRect,
    [a, b, tx, c, d, ty]: [f64; 6],
) -> Option<ibis_doc_model::Geometry> {
    use ibis_doc_model::{FormulaGeometry, Geometry, GeometryDef, GeometryPath, PathOp};
    let w = size.width as f64;
    let h = size.height as f64;
    let left = frame.x as f64;
    let top = frame.y as f64;
    let project = |x: f64, y: f64| (a * x + b * y + tx, c * x + d * y + ty);
    let point = |x: f64, y: f64| {
        let (x, y) = project(x, y);
        [
            (x - left).round().to_string(),
            (y - top).round().to_string(),
        ]
    };
    let path = |ops| GeometryPath {
        width: frame.width,
        height: frame.height,
        fill: Default::default(),
        stroke: true,
        ops,
    };
    let paths = match geometry {
        Geometry::Rect => {
            let [x, y] = point(0.0, 0.0);
            let mut ops = vec![PathOp::MoveTo(x, y)];
            for (x, y) in [(w, 0.0), (w, h), (0.0, h)] {
                let [x, y] = point(x, y);
                ops.push(PathOp::LineTo(x, y));
            }
            ops.push(PathOp::Close);
            vec![path(ops)]
        }
        Geometry::Ellipse => {
            let (rx, ry) = (w / 2.0, h / 2.0);
            let (kx, ky) = (rx * 0.5522847498307936, ry * 0.5522847498307936);
            let [x, y] = point(w, ry);
            let mut ops = vec![PathOp::MoveTo(x, y)];
            for (p, q, end) in [
                ((w, ry + ky), (rx + kx, h), (rx, h)),
                ((rx - kx, h), (0.0, ry + ky), (0.0, ry)),
                ((0.0, ry - ky), (rx - kx, 0.0), (rx, 0.0)),
                ((rx + kx, 0.0), (w, ry - ky), (w, ry)),
            ] {
                let [x1, y1] = point(p.0, p.1);
                let [x2, y2] = point(q.0, q.1);
                let [x3, y3] = point(end.0, end.1);
                ops.push(PathOp::CubicTo([x1, y1, x2, y2, x3, y3]));
            }
            ops.push(PathOp::Close);
            vec![path(ops)]
        }
        Geometry::Formula(formula) if formula.preset.is_none() => {
            let mut paths = Vec::new();
            for source in &formula.def.paths {
                let sx = w / source.width.max(1) as f64;
                let sy = h / source.height.max(1) as f64;
                let convert = |x: &str, y: &str| {
                    Some(point(
                        x.parse::<i64>().ok()? as f64 * sx,
                        y.parse::<i64>().ok()? as f64 * sy,
                    ))
                };
                let mut ops = Vec::new();
                for op in &source.ops {
                    ops.push(match op {
                        PathOp::MoveTo(x, y) => {
                            let [x, y] = convert(x, y)?;
                            PathOp::MoveTo(x, y)
                        }
                        PathOp::LineTo(x, y) => {
                            let [x, y] = convert(x, y)?;
                            PathOp::LineTo(x, y)
                        }
                        PathOp::CubicTo([x1, y1, x2, y2, x3, y3]) => {
                            let [x1, y1] = convert(x1, y1)?;
                            let [x2, y2] = convert(x2, y2)?;
                            let [x3, y3] = convert(x3, y3)?;
                            PathOp::CubicTo([x1, y1, x2, y2, x3, y3])
                        }
                        PathOp::Close => PathOp::Close,
                        _ => return None,
                    });
                }
                paths.push(GeometryPath {
                    fill: source.fill,
                    stroke: source.stroke,
                    ..path(ops)
                });
            }
            paths
        }
        _ => return None,
    };
    Some(Geometry::Formula(FormulaGeometry {
        preset: None,
        adjust: Vec::new(),
        def: std::sync::Arc::new(GeometryDef {
            paths,
            ..Default::default()
        }),
    }))
}

pub(super) fn curve_geometry(
    points: &[(i64, i64)],
    curved: &[bool],
    size: EmuSize,
) -> Option<ibis_doc_model::Geometry> {
    use ibis_doc_model::{FormulaGeometry, Geometry, GeometryDef, GeometryPath, PathFill, PathOp};
    let &(x, y) = points.first()?;
    let closed = points.len() > 2 && points.first() == points.last();
    let segments = points.len() - 1;
    let is_curve = |i| curved.get(i).copied().unwrap_or(false);
    let mut ops = vec![PathOp::MoveTo(x.to_string(), y.to_string())];
    for i in 0..segments {
        let p = points[i];
        let q = points[i + 1];
        if !is_curve(i) || p == q {
            ops.push(PathOp::LineTo(q.0.to_string(), q.1.to_string()));
            continue;
        }
        let before = i.checked_sub(1).or_else(|| closed.then_some(segments - 1));
        let after = if i + 1 == segments && closed {
            0
        } else {
            i + 1
        };
        let previous = before.filter(|&j| is_curve(j)).map(|j| points[j]);
        let next = (after < segments && is_curve(after)).then(|| points[after + 1]);
        let c1 = curve_handle(p, q, previous);
        let c2 = curve_handle(q, p, next);
        ops.push(PathOp::CubicTo([
            c1.0.to_string(),
            c1.1.to_string(),
            c2.0.to_string(),
            c2.1.to_string(),
            q.0.to_string(),
            q.1.to_string(),
        ]));
    }
    if closed {
        ops.push(PathOp::Close);
    }
    Some(Geometry::Formula(FormulaGeometry {
        preset: None,
        adjust: Vec::new(),
        def: std::sync::Arc::new(GeometryDef {
            paths: vec![GeometryPath {
                width: size.width.max(1),
                height: size.height.max(1),
                fill: if closed {
                    PathFill::Normal
                } else {
                    PathFill::None
                },
                stroke: true,
                ops,
            }],
            ..Default::default()
        }),
    }))
}

// Centripetal Catmull–Rom: shorter chords influence the tangent more.
// Use a one-sided tangent at endpoints and at line/curve joins.
fn curve_handle(p: (i64, i64), q: (i64, i64), previous: Option<(i64, i64)>) -> (i64, i64) {
    let delta = ((q.0 - p.0) as f64, (q.1 - p.1) as f64);
    let step = delta.0.hypot(delta.1).sqrt();
    let tangent = previous.filter(|&r| r != p).map_or(delta, |r| {
        let back = ((p.0 - r.0) as f64, (p.1 - r.1) as f64);
        let prior = back.0.hypot(back.1).sqrt();
        let blend = |a, b| (a * step * step / prior + b * prior) / (prior + step);
        (blend(back.0, delta.0), blend(back.1, delta.1))
    });
    (
        (p.0 as f64 + tangent.0 / 3.0).round() as i64,
        (p.1 as f64 + tangent.1 / 3.0).round() as i64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ibis_doc_model::{AutoShape, Fill, Geometry, PathOp};

    #[test]
    fn hwp_shear_keeps_transformed_outline_and_anchor() {
        let size = EmuSize::new(25400, 12700);
        let anchor = offset_anchor(&group_anchor(), 1270, 2540);
        let mut shape = Shape::new(
            ShapeId(1),
            EmuRect::new(0, 0, size.width, size.height),
            ShapeKind::Auto(AutoShape::new(Geometry::Rect, Fill::None, None)),
        );
        place_group_leaf(
            &mut shape,
            &anchor,
            size,
            [[1., 0., 7., 0., 1., 11.], [1., -0.5, 0., 0.25, 1., 0.]],
            |_, _, _, _| None,
        )
        .unwrap();
        assert_eq!(shape.frame, EmuRect::new(0, 0, 31750, 19050));
        assert_eq!(shape.rotation, 0);
        let placed = shape.floating.unwrap();
        assert_eq!((placed.offset_x, placed.offset_y), (-4191, 3937));
        let ShapeKind::Auto(auto) = shape.kind else {
            panic!("auto shape")
        };
        let Geometry::Formula(geometry) = auto.geometry else {
            panic!("custom path")
        };
        assert_eq!(
            geometry.def.paths[0].ops,
            [
                PathOp::MoveTo("6350".into(), "0".into()),
                PathOp::LineTo("31750".into(), "6350".into()),
                PathOp::LineTo("25400".into(), "19050".into()),
                PathOp::LineTo("0".into(), "12700".into()),
                PathOp::Close,
            ]
        );
    }

    #[test]
    fn hwpx_curve_joins_are_smooth_at_the_seam_and_preserve_straight_segments() {
        let points = [(0, 0), (900, 0), (1000, 1200), (0, 0)];
        let Some(Geometry::Formula(closed)) =
            curve_geometry(&points, &[true; 3], EmuSize::new(1000, 1200))
        else {
            panic!("closed curve")
        };
        let ops = &closed.def.paths[0].ops;
        let (PathOp::CubicTo(first), PathOp::CubicTo(last)) = (&ops[1], &ops[3]) else {
            panic!("cubic segments")
        };
        let x1 = first[0].parse::<i64>().unwrap();
        let y1 = first[1].parse::<i64>().unwrap();
        let x2 = -last[2].parse::<i64>().unwrap();
        let y2 = -last[3].parse::<i64>().unwrap();
        assert!((x1 * y2 - x2 * y1).abs() < 500);
        assert!(x1 * x2 + y1 * y2 > 0);
        assert_eq!(ops.last(), Some(&PathOp::Close));

        let Some(Geometry::Formula(mixed)) =
            curve_geometry(&points[..3], &[false, true], EmuSize::new(1000, 1200))
        else {
            panic!("mixed path")
        };
        assert_eq!(
            mixed.def.paths[0].ops[1],
            PathOp::LineTo("900".into(), "0".into())
        );
        let PathOp::CubicTo(segment) = &mixed.def.paths[0].ops[2] else {
            panic!("cubic segment")
        };
        assert_eq!(&segment[..2], ["933", "400"]);
    }
}

pub(super) fn box_run_style(
    format: Option<&ibis_doc_model::RunFormat>,
) -> ibis_doc_model::RunStyle {
    let Some(f) = format else {
        return Default::default();
    };
    ibis_doc_model::RunStyle {
        font_size: f.size,
        lang: Some("ko-KR".into()),
        font: f.east_asian_font.clone().or_else(|| f.font.clone()),
        fallback_font: f.font.clone(),
        bold: f.bold,
        italic: f.italic,
        underline: f.underline.unwrap_or(false),
        color: f.color,
        char_scale: f.scale,
        spacing: (f.spacing.unwrap_or(0) / 127).clamp(-16_000, 16_000) as i32,
        ..Default::default()
    }
}

pub(super) fn box_paragraph(
    format: Option<&ibis_doc_model::ParaFormat>,
    pitch: Option<ibis_doc_model::LineHeight>,
) -> ibis_doc_model::Paragraph {
    use ibis_doc_model::{LineHeight, LineSpacing, TextAlign};
    let height = pitch.or_else(|| format.and_then(|p| p.line));
    let hundredths =
        |value: Option<i64>| (value.unwrap_or(0).max(0) / 127).min(i64::from(u32::MAX)) as u32;
    ibis_doc_model::Paragraph {
        margin_left: format.and_then(|p| p.left).unwrap_or(0),
        indent: format.and_then(|p| p.first_line).unwrap_or(0),
        space_before: hundredths(format.and_then(|p| p.before)),
        space_after: hundredths(format.and_then(|p| p.after)),
        align: format.and_then(|p| p.align).map(|a| match a {
            FlowAlign::Center => TextAlign::Center,
            FlowAlign::Right => TextAlign::Right,
            FlowAlign::Justify => TextAlign::Justify,
            FlowAlign::Distribute => TextAlign::Distributed,
            _ => TextAlign::Left,
        }),
        line_spacing: match height {
            Some(LineHeight::Exact(h)) => Some(LineSpacing::Points((h / 127).max(1) as u32)),
            Some(LineHeight::Auto(f)) => {
                Some(LineSpacing::Percent(f.saturating_mul(100_000) / 240))
            }
            _ => None,
        },
        ..Default::default()
    }
}
