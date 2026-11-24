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
    // Orthogonal axes can be written as a rotated DrawingML rectangle.
    // For a shear, use the perpendicular height of the transformed frame.
    let determinant = a * d - b * c;
    let width = original.width as f64;
    let height = original.height as f64;
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

pub(super) fn curve_geometry(
    points: &[(i64, i64)],
    curved: &[bool],
    size: EmuSize,
) -> Option<ibis_doc_model::Geometry> {
    use ibis_doc_model::{FormulaGeometry, Geometry, GeometryDef, GeometryPath, PathFill, PathOp};
    let &(x, y) = points.first()?;
    let mut ops = vec![PathOp::MoveTo(x.to_string(), y.to_string())];
    for (i, (&(x0, y0), &(x1, y1))) in points.iter().zip(&points[1..]).enumerate() {
        if curved.get(i).copied().unwrap_or(false) {
            let (px, py) = points[i.saturating_sub(1)];
            let (nx, ny) = points[(i + 2).min(points.len() - 1)];
            ops.push(PathOp::CubicTo([
                (x0 + (x1 - px) / 6).to_string(),
                (y0 + (y1 - py) / 6).to_string(),
                (x1 - (nx - x0) / 6).to_string(),
                (y1 - (ny - y0) / 6).to_string(),
                x1.to_string(),
                y1.to_string(),
            ]));
        } else {
            ops.push(PathOp::LineTo(x1.to_string(), y1.to_string()));
        }
    }
    let closed = points.len() > 2 && points.first() == points.last();
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
