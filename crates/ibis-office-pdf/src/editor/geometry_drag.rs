// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use super::{
    Context, Pixels, Point, SlideEditor, State, Window, cx_report_geometry, page_scale, page_tops,
    pdf_point, point, px,
};
use ibis_vsdx::geometry_edit::{Endpoint, GeometryEdit, GeometryTargets};
use std::sync::Arc;

const HANDLE_RADIUS: f32 = 8.0;

#[derive(Clone, Copy)]
enum Object {
    Shape(u32),
    Connector(u32, Endpoint),
}

pub(super) struct GeometryDrag {
    object: Object,
    start: (f32, f32),
    current: (f32, f32),
}

fn shape_at(shapes: impl Iterator<Item = (u32, ibis_pdf::PdfRect)>, x: f32, y: f32) -> Option<u32> {
    shapes
        .filter(|(_, r)| r.x0 <= x && x <= r.x1 && r.y0 <= y && y <= r.y1)
        .min_by(|(_, a), (_, b)| (a.width() * a.height()).total_cmp(&(b.width() * b.height())))
        .map(|(id, _)| id)
}

pub(super) fn targets(editor: &SlideEditor, page: usize) -> Arc<GeometryTargets> {
    let Some(viewer) = super::viewer_of(editor) else {
        return Arc::default();
    };
    let viewer = viewer.lock().expect("pdf viewer lock");
    let crate::ViewerSource::Vsdx(document) = &viewer.source else {
        return Arc::default();
    };
    Arc::new(document.geometry_targets(page).unwrap_or_default())
}

pub(super) fn press(
    editor: &mut SlideEditor,
    page: usize,
    x: f32,
    y: f32,
    cx: &mut Context<SlideEditor>,
) -> bool {
    let Some(mode) = editor
        .format_state::<State>()
        .and_then(|s| s.geometry_edit.as_ref())
    else {
        return false;
    };
    let targets = if mode.page == page {
        mode.targets.clone()
    } else {
        targets(editor, page)
    };
    let radius = HANDLE_RADIUS / page_scale(editor);
    let object = targets
        .endpoints
        .iter()
        .map(|&(id, end, (a, b))| ((x - a).hypot(y - b), id, end))
        .filter(|&(distance, _, _)| distance <= radius)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, id, end)| Object::Connector(id, end))
        .or_else(|| {
            shape_at(
                targets
                    .group_children
                    .iter()
                    .rev()
                    .filter(|child| child.contains(x, y))
                    .map(|child| (child.id, child.bounds))
                    .chain(targets.shapes.iter().rev().copied()),
                x,
                y,
            )
            .map(Object::Shape)
        });
    let mode = editor
        .format_state_mut::<State>()
        .unwrap()
        .geometry_edit
        .as_mut()
        .unwrap();
    mode.page = page;
    mode.targets = targets;
    mode.drag = object.map(|object| GeometryDrag {
        object,
        start: (x, y),
        current: (x, y),
    });
    cx.notify();
    true
}

pub(super) fn update(editor: &mut SlideEditor, at: Point<Pixels>, cx: &mut Context<SlideEditor>) {
    let Some((page, x, y)) = pdf_point(editor, at) else {
        return;
    };
    if let Some(mode) = editor
        .format_state_mut::<State>()
        .and_then(|s| s.geometry_edit.as_mut())
        && mode.page == page
        && let Some(drag) = mode.drag.as_mut()
    {
        drag.current = (x, y);
        cx.notify();
    }
}

pub(super) fn ghost(drag: &GeometryDrag, targets: &GeometryTargets) -> ibis_pdf::PdfRect {
    let (x, y) = drag.current;
    if let Object::Shape(id) = drag.object
        && let Some((_, rect)) = targets
            .shapes
            .iter()
            .copied()
            .chain(
                targets
                    .group_children
                    .iter()
                    .map(|child| (child.id, child.bounds)),
            )
            .find(|(shape, _)| *shape == id)
    {
        let (dx, dy) = (x - drag.start.0, y - drag.start.1);
        return ibis_pdf::PdfRect {
            x0: rect.x0 + dx,
            y0: rect.y0 + dy,
            x1: rect.x1 + dx,
            y1: rect.y1 + dy,
        };
    }
    ibis_pdf::PdfRect {
        x0: x - 4.0,
        y0: y - 4.0,
        x1: x + 4.0,
        y1: y + 4.0,
    }
}

pub(super) fn release(
    editor: &mut SlideEditor,
    at: Point<Pixels>,
    window: &mut Window,
    cx: &mut Context<SlideEditor>,
) -> bool {
    let position = pdf_point(editor, at);
    let tolerance = 3.0 / page_scale(editor);
    let Some(mode) = editor
        .format_state_mut::<State>()
        .and_then(|s| s.geometry_edit.as_mut())
    else {
        return false;
    };
    let Some(drag) = mode.drag.take() else {
        return true;
    };
    cx.notify();
    let Some((page, x, y)) = position else {
        return true;
    };
    let dx = x - drag.start.0;
    let dy = y - drag.start.1;
    if page != mode.page || dx.hypot(dy) < tolerance {
        return true;
    }
    let edit = match drag.object {
        Object::Shape(shape) => {
            let (dx, dy) = (f64::from(dx) / 72.0, -f64::from(dy) / 72.0);
            let (dx, dy) = mode
                .targets
                .group_children
                .iter()
                .find(|child| child.id == shape)
                .map_or((dx, dy), |child| child.local_delta(dx, dy));
            GeometryEdit::Move { shape, dx, dy }
        }
        Object::Connector(connector, endpoint) => {
            let Some(target) = shape_at(mode.targets.shapes.iter().rev().copied(), x, y) else {
                editor.report(
                    Err(ibis_office_shell::i18n::text("vsdx.geometry-drop-target").to_string()),
                    cx,
                );
                return true;
            };
            GeometryEdit::Reconnect {
                connector,
                endpoint,
                target,
            }
        }
    };
    let result = super::geometry_edit_bytes(editor, page, edit)
        .and_then(|bytes| super::stage_geometry_edit(editor, bytes, window, cx, false));
    cx_report_geometry(editor, result, cx);
    true
}

/// Map a page point to the corresponding viewer-window location.
pub fn screen_point(editor: &SlideEditor, page: usize, x: f32, y: f32) -> Option<(f32, f32)> {
    let state = editor.format_state::<State>()?;
    let (left, top, width) = state.ui.lock().ok()?.canvas?;
    let scale = page_scale(editor);
    let viewer = state.viewer.lock().ok()?;
    let page_width = viewer.pages.get(page)?.crop_box.width() * scale;
    let (page_top, _) = page_tops(&viewer, scale)[page];
    let start = point(
        px(left + ((width - page_width) * 0.5).max(super::PAGE_PAD_PX)),
        px(top + page_top),
    );
    Some((
        f32::from(start.x) + x * scale,
        f32::from(start.y) + y * scale,
    ))
}
