// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use std::collections::HashMap;

use ibis_doc_model::{EMU_PER_POINT, Fill, LineEnd, Presentation, ShapeKind};
use ibis_pdf::PdfRect;
use ibis_vsdx::write::{Drawing, ShapeKind as DiagramShape, Side};

pub(crate) fn is_visio_path(path: &std::path::Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("vsdx"))
}

pub(crate) fn from_presentation(presentation: &Presentation) -> Result<Vec<u8>, String> {
    let unsupported = || crate::i18n::text("vsdx.draft-unsupported").to_string();
    let [slide] = presentation.slides.as_slice() else {
        return Err(unsupported());
    };
    if !slide.groups.is_empty() || !slide.backdrop.is_empty() {
        return Err(unsupported());
    }
    let points = |emu: i64| emu as f64 / EMU_PER_POINT as f64;
    let size = slide.size.unwrap_or(presentation.slide_size);
    let mut drawing =
        Drawing::new(points(size.width), points(size.height)).map_err(|e| e.to_string())?;
    let mut ids = HashMap::new();
    for shape in &slide.shapes {
        if slide.hidden_shapes.contains(&shape.id) {
            return Err(unsupported());
        }
        if matches!(shape.kind, ShapeKind::Line(_)) {
            continue;
        }
        let ShapeKind::Auto(auto) = &shape.kind else {
            return Err(unsupported());
        };
        if shape.rotation != 0 || shape.flip_h || shape.flip_v {
            return Err(unsupported());
        }
        let (kind, sites): (_, &[Option<Side>]) = match auto.geometry.preset_name() {
            Some("rect" | "flowChartProcess") => (DiagramShape::Rectangle, &CARDINAL),
            Some("diamond" | "flowChartDecision") => (DiagramShape::Diamond, &CARDINAL),
            Some("ellipse") => (DiagramShape::Ellipse, &ELLIPSE),
            _ => return Err(unsupported()),
        };
        let Fill::Solid(fill) = auto.fill else {
            return Err(unsupported());
        };
        if fill.alpha != 255 {
            return Err(unsupported());
        }
        let frame = shape.frame;
        let rect = PdfRect {
            x0: points(frame.x) as f32,
            y0: points(frame.y) as f32,
            x1: (points(frame.x) + points(frame.width)) as f32,
            y1: (points(frame.y) + points(frame.height)) as f32,
        };
        let label = auto
            .text
            .as_ref()
            .map(|body| body.text())
            .unwrap_or_default();
        let id = drawing
            .add_shape(
                kind,
                rect,
                label,
                [fill.rgb.r(), fill.rgb.g(), fill.rgb.b()],
            )
            .map_err(|e| e.to_string())?;
        ids.insert(shape.id.0, (id, sites));
    }
    for shape in &slide.shapes {
        if let ShapeKind::Line(stroke) = &shape.kind {
            let arrow = |end| match end {
                LineEnd::None => Ok(false),
                LineEnd::Triangle(_) => Ok(true),
                _ => Err(unsupported()),
            };
            let glue = slide.shape_glues.get(&shape.id).ok_or_else(unsupported)?;
            let end = |value: Option<(u64, u32)>| {
                let (shape, index) = value?;
                let (id, sites) = ids.get(&shape)?;
                Some((*id, sites.get(index as usize).copied().flatten()?))
            };
            drawing
                .connect_with_arrows(
                    end(glue.start).ok_or_else(unsupported)?,
                    end(glue.end).ok_or_else(unsupported)?,
                    (arrow(stroke.head)?, arrow(stroke.tail)?),
                )
                .map_err(|e| e.to_string())?;
        }
    }
    drawing.to_vsdx().map_err(|e| e.to_string())
}

const CARDINAL: [Option<Side>; 4] = [
    Some(Side::Top),
    Some(Side::Left),
    Some(Side::Bottom),
    Some(Side::Right),
];
const ELLIPSE: [Option<Side>; 8] = [
    Some(Side::Top),
    None,
    Some(Side::Left),
    None,
    Some(Side::Bottom),
    None,
    Some(Side::Right),
    None,
];
