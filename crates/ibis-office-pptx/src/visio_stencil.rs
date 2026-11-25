// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! Visio draft stencil palette (left pane), empty-canvas guide overlay,
//! and cardinal connection-point highlights.
//!
//! Keeps placement and starter templates within the subset supported by
//! `ibis_office_shell::visio::from_presentation` (single page, `rect` /
//! `roundRect` / `diamond` / `parallelogram` / `ellipse` with opaque solid
//! fills, and straight connectors glued to cardinal sites at both ends).

use gpui_kit::component::{ActiveTheme as _, Icon, h_flex, v_flex};
use gpui_kit::*;
use ibis_doc_model::{
    Color, Command, EmuPoint, EmuRect, Fill, Glue, ResizeHandle, Rgb, Shape, ShapeId, ShapeKind, Stroke,
};

use crate::ext::*;
use crate::shell::editor::SlideEditor;
use crate::shell::i18n::text;
use crate::shell::icons::Material;
use crate::shell::palette::rgb;
use crate::shell::shape_items::{ArrowKind, LineKind, new_line, new_shape};
use crate::shell::shape_state::{Drag, Tool};

/// Snap radius (in EMU, ~5 mm) for cardinal Visio connection sites.
pub(crate) const VISIO_SNAP_RADIUS: i64 = 180_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StencilSide {
    Top,
    Left,
    Bottom,
    Right,
}

#[derive(Clone, Copy, Debug)]
struct StencilItem {
    id: &'static str,
    preset: &'static str,
    label_key: &'static str,
    default_text_key: &'static str,
    icon: &'static str,
    color: u32,
    width: i64,
    height: i64,
}

/// VSDX-compatible flowchart & basic stencils shown in the left pane.
const STENCILS: &[StencilItem] = &[
    StencilItem {
        id: "visio-stencil-process",
        preset: "rect",
        label_key: "vsdx.stencil.process",
        default_text_key: "vsdx.stencil.default.process",
        icon: "rectangle",
        color: 0x4472c4,
        width: 1_800_000,
        height: 900_000,
    },
    StencilItem {
        id: "visio-stencil-decision",
        preset: "diamond",
        label_key: "vsdx.stencil.decision",
        default_text_key: "vsdx.stencil.default.decision",
        icon: "diamond",
        color: 0xed7d31,
        width: 1_800_000,
        height: 1_100_000,
    },
    StencilItem {
        id: "visio-stencil-terminator",
        preset: "ellipse",
        label_key: "vsdx.stencil.terminator",
        default_text_key: "vsdx.stencil.default.start",
        icon: "circle",
        color: 0x70ad47,
        width: 1_650_000,
        height: 820_000,
    },
    StencilItem {
        id: "visio-stencil-rounded",
        preset: "roundRect",
        label_key: "vsdx.stencil.rounded",
        default_text_key: "vsdx.stencil.default.rounded",
        icon: "rounded_corner",
        color: 0x2e8b8b,
        width: 1_800_000,
        height: 900_000,
    },
    StencilItem {
        id: "visio-stencil-data",
        preset: "parallelogram",
        label_key: "vsdx.stencil.data",
        default_text_key: "vsdx.stencil.default.data",
        icon: "input",
        color: 0x7030a0,
        width: 1_850_000,
        height: 900_000,
    },
    StencilItem {
        id: "visio-stencil-box",
        preset: "rect",
        label_key: "vsdx.stencil.box",
        default_text_key: "vsdx.stencil.default.box",
        icon: "crop_square",
        color: 0x5b9bd5,
        width: 1_800_000,
        height: 900_000,
    },
];

const QUICK_COLORS: &[(&str, u32)] = &[
    ("visio-color-blue", 0x4472c4),
    ("visio-color-sky", 0x5b9bd5),
    ("visio-color-green", 0x70ad47),
    ("visio-color-orange", 0xed7d31),
    ("visio-color-gold", 0xbf9000),
    ("visio-color-slate", 0x595959),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StarterTemplate {
    BasicFlow,
    DecisionFlow,
}

fn darker_outline(rgb_hex: u32) -> Rgb {
    let r = (((rgb_hex >> 16) & 0xff) as f32 * 0.72) as u32;
    let g = (((rgb_hex >> 8) & 0xff) as f32 * 0.72) as u32;
    let b = ((rgb_hex & 0xff) as f32 * 0.72) as u32;
    Rgb((r << 16) | (g << 8) | b)
}

fn site_index(preset: &str, side: StencilSide) -> u32 {
    if preset == "ellipse" {
        match side {
            StencilSide::Top => 0,
            StencilSide::Left => 2,
            StencilSide::Bottom => 4,
            StencilSide::Right => 6,
        }
    } else {
        match side {
            StencilSide::Top => 0,
            StencilSide::Left => 1,
            StencilSide::Bottom => 2,
            StencilSide::Right => 3,
        }
    }
}

fn shape_preset(shape: &Shape) -> Option<&str> {
    match &shape.kind {
        ShapeKind::Auto(auto) => auto.geometry.preset_name(),
        _ => None,
    }
}

fn site_point(frame: EmuRect, side: StencilSide) -> EmuPoint {
    match side {
        StencilSide::Left => EmuPoint::new(frame.x, frame.y + frame.height / 2),
        StencilSide::Right => EmuPoint::new(frame.right(), frame.y + frame.height / 2),
        StencilSide::Top => EmuPoint::new(frame.x + frame.width / 2, frame.y),
        StencilSide::Bottom => EmuPoint::new(frame.x + frame.width / 2, frame.bottom()),
    }
}

/// The 4 VSDX-compatible cardinal connection sites `(site_index, point)` of `shape`.
pub(crate) fn cardinal_sites_of(shape: &Shape) -> Vec<(u32, EmuPoint)> {
    if shape.rotation != 0 || shape.flip_h || shape.flip_v {
        return Vec::new();
    }
    let Some(preset) = shape_preset(shape) else {
        return Vec::new();
    };
    if !matches!(
        preset,
        "rect"
            | "roundRect"
            | "flowChartProcess"
            | "flowChartAlternateProcess"
            | "flowChartTerminator"
            | "diamond"
            | "flowChartDecision"
            | "parallelogram"
            | "flowChartInputOutput"
            | "ellipse"
    ) {
        return Vec::new();
    }
    [StencilSide::Top, StencilSide::Left, StencilSide::Bottom, StencilSide::Right]
        .into_iter()
        .map(|side| (site_index(preset, side), site_point(shape.frame, side)))
        .collect()
}

/// Nearest VSDX-compatible cardinal connection site within `radius`, optionally
/// excluding a shape id (e.g., to prevent self-connecting a connector).
pub(crate) fn nearest_cardinal_site<'a>(
    shapes: impl IntoIterator<Item = &'a Shape>,
    x: i64,
    y: i64,
    radius: i64,
    exclude_shape: Option<u64>,
) -> Option<(u64, u32, EmuPoint)> {
    let max_d2 = (radius as i128).pow(2);
    let mut best: Option<(i128, (u64, u32, EmuPoint))> = None;
    for shape in shapes {
        if exclude_shape == Some(shape.id.0) {
            continue;
        }
        for (idx, pt) in cardinal_sites_of(shape) {
            let d2 = (pt.x as i128 - x as i128).pow(2) + (pt.y as i128 - y as i128).pow(2);
            if d2 <= max_d2 && best.as_ref().is_none_or(|b| d2 < b.0) {
                best = Some((d2, (shape.id.0, idx, pt)));
            }
        }
    }
    best.map(|b| b.1)
}

/// Update hovered connection site while a connector tool is armed in Visio draft mode.
pub(crate) fn update_connector_hover(editor: &mut SlideEditor, position: Point<Pixels>, cx: &mut Context<SlideEditor>) {
    let next = if editor.is_visio_draft()
        && matches!(editor.pptx_tool(), Tool::Line { .. })
        && editor.shape_edit.drag.is_none()
        && let Some(layout) = editor.slide_layout()
    {
        let p = layout.to_emu(position);
        nearest_cardinal_site(editor.slide().shown_shapes(), p.x, p.y, VISIO_SNAP_RADIUS, None)
            .map(|(id, site, _)| (id, site))
    } else {
        None
    };
    if let Some(st) = editor.pptx_mut()
        && st.visio_hover_site != next
    {
        st.visio_hover_site = next;
        cx.notify();
    }
}

pub(crate) fn clear_connector_hover(editor: &mut SlideEditor) -> bool {
    if let Some(st) = editor.pptx_mut()
        && st.visio_hover_site.is_some()
    {
        st.visio_hover_site = None;
        return true;
    }
    false
}

/// Render Visio cardinal connection points and highlight active snap targets
/// when a connector tool is armed or a connector is being drawn/reconnected.
pub(crate) fn render_connection_points(editor: &SlideEditor, scale: f32, cx: &mut Context<SlideEditor>) -> Option<AnyElement> {
    if !editor.is_visio_draft() {
        return None;
    }
    let slide = editor.slide();
    let drag = editor.shape_edit.drag.as_ref();
    let is_line_tool = matches!(editor.pptx_tool(), Tool::Line { .. });
    let is_line_place = matches!(drag, Some(Drag::Place { tool: Tool::Line { .. }, .. }));
    let is_line_resize = matches!(
        drag,
        Some(Drag::Resize { shape, .. })
            if slide.shape(*shape).is_some_and(|s| matches!(s.kind, ShapeKind::Line(_)))
    );
    if !is_line_tool && !is_line_place && !is_line_resize {
        return None;
    }

    let mut active_sites: Vec<(u64, u32)> = Vec::new();
    if let (Some(drag), Some(layout)) = (drag, editor.slide_layout()) {
        match drag {
            Drag::Place { tool: Tool::Line { .. }, start, current, .. } => {
                let a = layout.to_emu(*start);
                let b = layout.to_emu(*current);
                let start_snap = nearest_cardinal_site(slide.shown_shapes(), a.x, a.y, VISIO_SNAP_RADIUS, None);
                if let Some((id, site, _)) = start_snap {
                    active_sites.push((id, site));
                }
                let end_snap = nearest_cardinal_site(
                    slide.shown_shapes(),
                    b.x,
                    b.y,
                    VISIO_SNAP_RADIUS,
                    start_snap.map(|s| s.0),
                );
                if let Some((id, site, _)) = end_snap {
                    active_sites.push((id, site));
                }
            }
            Drag::Resize { shape, handle, current, .. } => {
                if let Some(line) = slide.shape(*shape)
                    && matches!(line.kind, ShapeKind::Line(_))
                {
                    let start_handle = match (line.flip_h, line.flip_v) {
                        (false, false) => ResizeHandle::TopLeft,
                        (true, false) => ResizeHandle::TopRight,
                        (false, true) => ResizeHandle::BottomLeft,
                        (true, true) => ResizeHandle::BottomRight,
                    };
                    let opposite = slide.shape_glues.get(shape).and_then(|g| {
                        if *handle == start_handle { g.end } else { g.start }
                    }).map(|(id, _)| id);
                    let p = layout.to_emu(*current);
                    if let Some((id, site, _)) = nearest_cardinal_site(slide.shown_shapes(), p.x, p.y, VISIO_SNAP_RADIUS, opposite) {
                        active_sites.push((id, site));
                    }
                }
            }
            _ => {}
        }
    } else if let Some(hovered) = editor.pptx().and_then(|s| s.visio_hover_site) {
        active_sites.push(hovered);
    }

    let theme = cx.theme();
    let mut dots: Vec<AnyElement> = Vec::new();
    for shape in slide.shown_shapes() {
        for (idx, pt) in cardinal_sites_of(shape) {
            let active = active_sites.contains(&(shape.id.0, idx));
            let cx_px = pt.x as f32 * scale;
            let cy_px = pt.y as f32 * scale;
            if active {
                // Classic Visio green snap ring + center dot.
                let size = 16.0_f32;
                dots.push(
                    div()
                        .absolute()
                        .left(px(cx_px - size / 2.0))
                        .top(px(cy_px - size / 2.0))
                        .w(px(size))
                        .h(px(size))
                        .rounded_full()
                        .border_2()
                        .border_color(rgb(0x16a34a))
                        .bg(rgba(0x16a34a33))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(div().w(px(7.0)).h(px(7.0)).rounded_full().bg(rgb(0x16a34a)))
                        .into_any_element(),
                );
            } else {
                let size = 8.0_f32;
                dots.push(
                    div()
                        .absolute()
                        .left(px(cx_px - size / 2.0))
                        .top(px(cy_px - size / 2.0))
                        .w(px(size))
                        .h(px(size))
                        .rounded_full()
                        .border_1()
                        .border_color(theme.primary)
                        .bg(rgb(0xffffff))
                        .into_any_element(),
                );
            }
        }
    }
    if dots.is_empty() {
        return None;
    }
    Some(div().absolute().inset_0().children(dots).into_any_element())
}

fn make_diagram_shape(id: ShapeId, preset: &str, frame: EmuRect, fill_hex: u32, label: &str) -> Shape {
    let mut shape = new_shape(id, preset, frame);
    if let ShapeKind::Auto(auto) = &mut shape.kind {
        auto.fill = Fill::Solid(Color::opaque(Rgb(fill_hex)));
        auto.stroke = Some(Stroke::solid(Color::opaque(darker_outline(fill_hex)), 12_700));
        if let Some(body) = auto.text.as_mut() {
            body.set_text(label);
        }
    }
    shape
}

fn make_glued_arrow(
    id: ShapeId,
    from_shape: &Shape,
    from_side: StencilSide,
    to_shape: &Shape,
    to_side: StencilSide,
) -> (Shape, Glue) {
    let a = site_point(from_shape.frame, from_side);
    let b = site_point(to_shape.frame, to_side);
    let line = new_line(id, EmuRect::from_corners(a, b), a.x > b.x, a.y > b.y, true);
    let from_preset = shape_preset(from_shape).unwrap_or("rect");
    let to_preset = shape_preset(to_shape).unwrap_or("rect");
    let glue = Glue {
        start: Some((from_shape.id.0, site_index(from_preset, from_side))),
        end: Some((to_shape.id.0, site_index(to_preset, to_side))),
    };
    (line, glue)
}

fn next_shape_id(editor: &SlideEditor) -> u64 {
    editor.slide().shapes.iter().map(|s| s.id.0).max().unwrap_or(0) + 1
}

/// Place a stencil shape onto the Visio draft canvas.
///
/// When `visio_auto_connect` is enabled and a single auto-shape is selected,
/// places the new shape beside it (or below if near the right edge) and glues
/// a straight arrow connector between them as a single undo step.
pub(crate) fn place_stencil(
    editor: &mut SlideEditor,
    stencil_id: &str,
    window: &mut Window,
    cx: &mut Context<SlideEditor>,
) {
    let Some(item) = STENCILS.iter().find(|s| s.id == stencil_id) else {
        return;
    };
    editor.commit_edit(window, cx);
    editor.set_pptx_tool(Tool::Select, window, cx);

    let slide_id = editor.current_slide;
    let slide_size = editor.presentation().size_of(editor.slide());
    let auto_connect = editor.pptx().map(|s| s.visio_auto_connect).unwrap_or(true);
    let custom_fill = editor.pptx().and_then(|s| s.visio_fill_override);
    let fill_hex = custom_fill.unwrap_or(item.color);

    let margin_x = (slide_size.width / 12).max(450_000);
    let margin_y = (slide_size.height / 10).max(450_000);
    let gap_x = 680_000;
    let gap_y = 600_000;

    let anchor = {
        let selected = editor.selected_shapes();
        if selected.len() == 1 && matches!(selected[0].kind, ShapeKind::Auto(_)) {
            Some(selected[0].clone())
        } else {
            None
        }
    };

    let (frame, connect_from): (EmuRect, Option<(Shape, StencilSide, StencilSide)>) =
        if let Some(prev) = anchor {
            let right_x = prev.frame.right() + gap_x;
            let aligned_y = prev.frame.y + (prev.frame.height - item.height) / 2;
            if right_x + item.width <= slide_size.width - margin_x / 2 {
                let f = EmuRect::new(
                    right_x,
                    aligned_y.clamp(margin_y / 2, (slide_size.height - item.height - margin_y / 2).max(0)),
                    item.width,
                    item.height,
                );
                (f, auto_connect.then_some((prev, StencilSide::Right, StencilSide::Left)))
            } else {
                let below_x = prev.frame.x + (prev.frame.width - item.width) / 2;
                let below_y = prev.frame.bottom() + gap_y;
                let f = EmuRect::new(
                    below_x.clamp(margin_x / 2, (slide_size.width - item.width - margin_x / 2).max(0)),
                    below_y.clamp(margin_y / 2, (slide_size.height - item.height - margin_y / 2).max(0)),
                    item.width,
                    item.height,
                );
                (f, auto_connect.then_some((prev, StencilSide::Bottom, StencilSide::Top)))
            }
        } else {
            let auto_count = editor
                .slide()
                .shapes
                .iter()
                .filter(|s| matches!(s.kind, ShapeKind::Auto(_)))
                .count() as i64;
            let cols = ((slide_size.width - margin_x * 2 + gap_x) / (item.width + gap_x)).max(1);
            let col = auto_count % cols;
            let row = auto_count / cols;
            let x = (margin_x + col * (item.width + gap_x))
                .min((slide_size.width - item.width - margin_x / 2).max(0));
            let y = (margin_y + row * (item.height + gap_y))
                .min((slide_size.height - item.height - margin_y / 2).max(0));
            (EmuRect::new(x, y, item.width, item.height), None)
        };

    let mut next_id = next_shape_id(editor);
    let shape_id = ShapeId(next_id);
    next_id += 1;
    let label = text(item.default_text_key);
    let shape = make_diagram_shape(shape_id, item.preset, frame, fill_hex, &label);
    let base_index = editor.slide().shapes.len();

    let mut commands = Vec::new();
    let mut shapes_to_insert = vec![(base_index, shape.clone())];
    if let Some((prev, from_side, to_side)) = connect_from {
        let conn_id = ShapeId(next_id);
        let (line, glue) = make_glued_arrow(conn_id, &prev, from_side, &shape, to_side);
        shapes_to_insert.push((base_index + 1, line));
        commands.push(Command::InsertShape {
            slide: slide_id,
            shapes: shapes_to_insert,
        });
        commands.push(Command::SetShapeGlue {
            slide: slide_id,
            shape: conn_id,
            glue: Some(glue),
        });
    } else {
        commands.push(Command::InsertShape {
            slide: slide_id,
            shapes: shapes_to_insert,
        });
    }

    editor.run(commands, cx);
    editor.set_selection(vec![shape_id]);
    cx.notify();
}

/// Insert a 1-click starter flowchart template (fully VSDX-exportable).
pub(crate) fn insert_starter_template(
    editor: &mut SlideEditor,
    template: StarterTemplate,
    window: &mut Window,
    cx: &mut Context<SlideEditor>,
) {
    editor.commit_edit(window, cx);
    editor.set_pptx_tool(Tool::Select, window, cx);

    let slide_id = editor.current_slide;
    let size = editor.presentation().size_of(editor.slide());
    let mut next_id = next_shape_id(editor);
    let mut alloc = || {
        let id = ShapeId(next_id);
        next_id += 1;
        id
    };

    let base_index = editor.slide().shapes.len();
    let mut shapes: Vec<Shape> = Vec::new();
    let mut glues: Vec<(ShapeId, Glue)> = Vec::new();

    match template {
        StarterTemplate::BasicFlow => {
            let w = (size.width / 6).clamp(1_300_000, 1_850_000);
            let h = (size.height / 7).clamp(720_000, 960_000);
            let gap = ((size.width - w * 4) / 5).clamp(320_000, 750_000);
            let total_w = w * 4 + gap * 3;
            let start_x = ((size.width - total_w) / 2).max(200_000);
            let y = ((size.height - h) / 2).max(200_000);

            let s1 = make_diagram_shape(alloc(), "ellipse", EmuRect::new(start_x, y, w, h), 0x70ad47, &text("vsdx.stencil.default.start"));
            let s2 = make_diagram_shape(alloc(), "rect", EmuRect::new(start_x + (w + gap), y, w, h), 0x4472c4, &text("vsdx.template.step1"));
            let s3 = make_diagram_shape(alloc(), "rect", EmuRect::new(start_x + (w + gap) * 2, y, w, h), 0x4472c4, &text("vsdx.template.step2"));
            let s4 = make_diagram_shape(alloc(), "ellipse", EmuRect::new(start_x + (w + gap) * 3, y, w, h), 0x595959, &text("vsdx.stencil.default.end"));

            let pairs = [(&s1, &s2), (&s2, &s3), (&s3, &s4)];
            let mut conns = Vec::new();
            for (from, to) in pairs {
                let cid = alloc();
                let (line, glue) = make_glued_arrow(cid, from, StencilSide::Right, to, StencilSide::Left);
                conns.push(line);
                glues.push((cid, glue));
            }
            shapes.extend([s1, s2, s3, s4]);
            shapes.extend(conns);
        }
        StarterTemplate::DecisionFlow => {
            let w = (size.width / 6).clamp(1_300_000, 1_800_000);
            let h = (size.height / 7).clamp(740_000, 960_000);
            let dh = (h as f64 * 1.25) as i64;
            let gap_x = ((size.width - w * 3) / 4).clamp(400_000, 850_000);
            let total_w = w * 3 + gap_x * 2;
            let start_x = ((size.width - total_w) / 2).max(200_000);
            let top_y = (size.height / 4).max(400_000);
            let bottom_y = (top_y + dh + 650_000).min((size.height - h - 300_000).max(top_y + h));

            let s_start = make_diagram_shape(alloc(), "ellipse", EmuRect::new(start_x, top_y + (dh - h) / 2, w, h), 0x70ad47, &text("vsdx.stencil.default.start"));
            let s_check = make_diagram_shape(alloc(), "diamond", EmuRect::new(start_x + w + gap_x, top_y, w, dh), 0xed7d31, &text("vsdx.stencil.default.decision"));
            let s_yes = make_diagram_shape(alloc(), "rect", EmuRect::new(start_x + (w + gap_x) * 2, top_y + (dh - h) / 2, w, h), 0x4472c4, &text("vsdx.template.approve"));
            let s_no = make_diagram_shape(alloc(), "rect", EmuRect::new(start_x + w + gap_x, bottom_y, w, h), 0x5b9bd5, &text("vsdx.template.revise"));

            let links = [
                (&s_start, StencilSide::Right, &s_check, StencilSide::Left),
                (&s_check, StencilSide::Right, &s_yes, StencilSide::Left),
                (&s_check, StencilSide::Bottom, &s_no, StencilSide::Top),
            ];
            let mut conns = Vec::new();
            for (from, fs, to, ts) in links {
                let cid = alloc();
                let (line, glue) = make_glued_arrow(cid, from, fs, to, ts);
                conns.push(line);
                glues.push((cid, glue));
            }
            shapes.extend([s_start, s_check, s_yes, s_no]);
            shapes.extend(conns);
        }
    }

    let first_id = shapes.first().map(|s| s.id);
    let indexed = shapes
        .into_iter()
        .enumerate()
        .map(|(i, s)| (base_index + i, s))
        .collect();
    let mut commands = vec![Command::InsertShape {
        slide: slide_id,
        shapes: indexed,
    }];
    for (cid, glue) in glues {
        commands.push(Command::SetShapeGlue {
            slide: slide_id,
            shape: cid,
            glue: Some(glue),
        });
    }
    editor.run(commands, cx);
    if let Some(id) = first_id {
        editor.set_selection(vec![id]);
    }
    cx.notify();
}

/// Apply a quick solid fill color to the current selection, or set it as the
/// fill override for subsequent stencil placements when nothing is selected.
fn apply_quick_color(editor: &mut SlideEditor, hex: u32, cx: &mut Context<SlideEditor>) {
    let has_selection = !editor.selected_shapes().is_empty();
    if let Some(st) = editor.pptx_mut() {
        st.visio_fill_override = if st.visio_fill_override == Some(hex) && !has_selection {
            None
        } else {
            Some(hex)
        };
    }
    if has_selection {
        editor.fill_selected(Fill::Solid(Color::opaque(Rgb(hex))), cx);
    }
    cx.notify();
}

/// Render the Visio-style stencil palette in the left pane.
pub(crate) fn render_stencil_pane(editor: &SlideEditor, cx: &mut Context<SlideEditor>) -> AnyElement {
    let theme = cx.theme();
    let pane_w = editor.pane_width_now();
    let tool = editor.pptx_tool();
    let auto_connect = editor.pptx().map(|s| s.visio_auto_connect).unwrap_or(true);
    let fill_override = editor.pptx().and_then(|s| s.visio_fill_override);

    let section_title = |key: &str| {
        div()
            .text_xs()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.muted_foreground)
            .pb_1()
            .child(text(key))
    };

    let mut stencil_rows = v_flex().gap_1p5();
    for pair in STENCILS.chunks(2) {
        let mut row = h_flex().gap_1p5().w_full();
        for item in pair {
            let id = item.id;
            let swatch = fill_override.unwrap_or(item.color);
            row = row.child(
                v_flex()
                    .id(id)
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .justify_center()
                    .gap_1()
                    .px_1p5()
                    .py_2()
                    .rounded_md()
                    .border_1()
                    .border_color(theme.sidebar_border)
                    .bg(theme.background)
                    .hover(|s| s.bg(theme.accent).border_color(theme.primary))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        place_stencil(this, id, window, cx);
                    }))
                    .child(
                        div()
                            .w(px(30.))
                            .h(px(24.))
                            .rounded_sm()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(rgb(swatch))
                            .child(Icon::new(Material(item.icon)).size_4().text_color(rgb(0xffffff))),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.foreground)
                            .truncate()
                            .child(text(item.label_key)),
                    ),
            );
        }
        stencil_rows = stencil_rows.child(row);
    }

    let auto_connect_row = h_flex()
        .id("visio-auto-connect-toggle")
        .w_full()
        .items_center()
        .justify_between()
        .px_2()
        .py_1()
        .rounded_md()
        .border_1()
        .border_color(if auto_connect { theme.primary } else { theme.sidebar_border })
        .bg(if auto_connect { theme.accent } else { theme.background })
        .cursor_pointer()
        .on_click(cx.listener(|this, _, _, cx| {
            if let Some(st) = this.pptx_mut() {
                st.visio_auto_connect = !st.visio_auto_connect;
            }
            cx.notify();
        }))
        .child(
            h_flex()
                .items_center()
                .gap_1p5()
                .child(Icon::new(Material("route")).size_3p5().text_color(if auto_connect { theme.primary } else { theme.muted_foreground }))
                .child(div().text_xs().text_color(theme.foreground).child(text("vsdx.stencil.auto-connect"))),
        )
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(if auto_connect { theme.primary } else { theme.muted_foreground })
                .child(if auto_connect { "ON" } else { "OFF" }),
        );

    let tool_button = |id: &'static str, icon: &'static str, label_key: &'static str, active: bool, next_tool: Tool| {
        h_flex()
            .id(id)
            .w_full()
            .items_center()
            .gap_2()
            .px_2()
            .py_1p5()
            .rounded_md()
            .border_1()
            .border_color(if active { theme.primary } else { theme.sidebar_border })
            .bg(if active { theme.accent } else { theme.background })
            .hover(|s| s.bg(theme.accent))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| {
                this.set_pptx_tool(next_tool, window, cx);
            }))
            .child(Icon::new(Material(icon)).size_4().text_color(if active { theme.primary } else { theme.foreground }))
            .child(div().text_xs().text_color(theme.foreground).child(text(label_key)))
    };

    let connector_tools = v_flex()
        .gap_1()
        .child(tool_button(
            "visio-tool-select",
            "arrow_selector_tool",
            "vsdx.stencil.tool.select",
            tool == Tool::Select,
            Tool::Select,
        ))
        .child(tool_button(
            "visio-tool-arrow",
            "arrow_right_alt",
            "vsdx.stencil.tool.arrow",
            tool == Tool::Line { kind: LineKind::Straight, arrow: ArrowKind::Single },
            Tool::Line { kind: LineKind::Straight, arrow: ArrowKind::Single },
        ))
        .child(tool_button(
            "visio-tool-double-arrow",
            "sync_alt",
            "vsdx.stencil.tool.double-arrow",
            tool == Tool::Line { kind: LineKind::Straight, arrow: ArrowKind::Double },
            Tool::Line { kind: LineKind::Straight, arrow: ArrowKind::Double },
        ))
        .child(tool_button(
            "visio-tool-line",
            "horizontal_rule",
            "vsdx.stencil.tool.line",
            tool == Tool::Line { kind: LineKind::Straight, arrow: ArrowKind::None },
            Tool::Line { kind: LineKind::Straight, arrow: ArrowKind::None },
        ));

    let mut color_row = h_flex().gap_1p5().flex_wrap();
    for &(cid, hex) in QUICK_COLORS {
        let active = fill_override == Some(hex);
        color_row = color_row.child(
            div()
                .id(cid)
                .w(px(24.))
                .h(px(24.))
                .rounded_full()
                .bg(rgb(hex))
                .border_2()
                .border_color(if active { theme.foreground } else { theme.background })
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| apply_quick_color(this, hex, cx))),
        );
    }

    let template_btn = |id: &'static str, icon: &'static str, label_key: &'static str, tpl: StarterTemplate| {
        h_flex()
            .id(id)
            .w_full()
            .items_center()
            .gap_2()
            .px_2()
            .py_1p5()
            .rounded_md()
            .border_1()
            .border_color(theme.sidebar_border)
            .bg(theme.background)
            .hover(|s| s.bg(theme.accent).border_color(theme.primary))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| {
                insert_starter_template(this, tpl, window, cx);
            }))
            .child(Icon::new(Material(icon)).size_4().text_color(theme.primary))
            .child(div().text_xs().text_color(theme.foreground).child(text(label_key)))
    };

    let templates = v_flex()
        .gap_1()
        .child(template_btn(
            "visio-template-basic",
            "account_tree",
            "vsdx.template.basic-flow",
            StarterTemplate::BasicFlow,
        ))
        .child(template_btn(
            "visio-template-decision",
            "call_split",
            "vsdx.template.decision-flow",
            StarterTemplate::DecisionFlow,
        ));

    let header = v_flex()
        .w_full()
        .px_2p5()
        .py_2()
        .border_b_1()
        .border_color(theme.sidebar_border)
        .child(
            h_flex()
                .items_center()
                .gap_1p5()
                .child(Icon::new(Material("draw")).size_4().text_color(theme.primary))
                .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).text_color(theme.foreground).child(text("vsdx.stencil.title"))),
        )
        .child(div().text_xs().text_color(theme.muted_foreground).child(text("vsdx.stencil.subtitle")));

    let body = v_flex()
        .id("visio-stencil-scroll")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .px_2p5()
        .py_2()
        .gap_3()
        .child(
            v_flex()
                .gap_1p5()
                .child(section_title("vsdx.stencil.section.shapes"))
                .child(stencil_rows)
                .child(auto_connect_row),
        )
        .child(
            v_flex()
                .gap_1()
                .child(section_title("vsdx.stencil.section.connectors"))
                .child(connector_tools),
        )
        .child(
            v_flex()
                .gap_1()
                .child(section_title("vsdx.stencil.section.colors"))
                .child(color_row),
        )
        .child(
            v_flex()
                .gap_1()
                .child(section_title("vsdx.stencil.section.templates"))
                .child(templates),
        );

    v_flex()
        .id("visio-stencil-pane")
        .relative()
        .w(px(pane_w))
        .flex_shrink_0()
        .h_full()
        .bg(theme.sidebar)
        .border_r_1()
        .border_color(theme.sidebar_border)
        .child(header)
        .child(body)
        .into_any_element()
}

/// Render an unobtrusive empty-canvas guide when a new Visio draft has no shapes yet.
///
/// The outer wrapper does not intercept pointer events so dragging on the canvas
/// with a shape/connector tool works as usual; only the quick-start template
/// buttons capture clicks.
pub(crate) fn render_empty_canvas_guide(editor: &SlideEditor, cx: &mut Context<SlideEditor>) -> Option<AnyElement> {
    if !editor.is_visio_draft() || !editor.slide().shapes.is_empty() {
        return None;
    }
    let theme = cx.theme();
    let step_row = |num: &'static str, key: &'static str| {
        h_flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .w(px(20.))
                    .h(px(20.))
                    .rounded_full()
                    .bg(theme.accent)
                    .text_color(theme.primary)
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(num),
            )
            .child(div().text_xs().text_color(theme.muted_foreground).child(text(key)))
    };

    let action_btn = |id: &'static str, label_key: &'static str, tpl: StarterTemplate| {
        div()
            .id(id)
            .px_3()
            .py_1p5()
            .rounded_md()
            .border_1()
            .border_color(theme.primary)
            .bg(theme.accent)
            .hover(|s| s.opacity(0.9))
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme.primary)
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |this, _, window, cx| {
                insert_starter_template(this, tpl, window, cx);
            }))
            .child(text(label_key))
    };

    Some(
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .child(
                v_flex()
                    .id("visio-empty-guide")
                    .max_w(px(380.))
                    .px_5()
                    .py_4()
                    .gap_2p5()
                    .rounded_lg()
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.background)
                    .shadow_sm()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(Icon::new(Material("draw")).size_5().text_color(theme.primary))
                            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).text_color(theme.foreground).child(text("vsdx.guide.title"))),
                    )
                    .child(
                        v_flex()
                            .gap_1p5()
                            .child(step_row("1", "vsdx.guide.step1"))
                            .child(step_row("2", "vsdx.guide.step2"))
                            .child(step_row("3", "vsdx.guide.step3")),
                    )
                    .child(
                        h_flex()
                            .pt_1()
                            .gap_2()
                            .child(action_btn("visio-guide-basic-flow", "vsdx.template.basic-flow", StarterTemplate::BasicFlow))
                            .child(action_btn("visio-guide-decision-flow", "vsdx.template.decision-flow", StarterTemplate::DecisionFlow)),
                    ),
            )
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn stencils_only_use_vsdx_supported_presets() {
        for item in STENCILS {
            assert!(
                matches!(item.preset, "rect" | "roundRect" | "diamond" | "parallelogram" | "ellipse"),
                "stencil {} uses unsupported VSDX preset {}",
                item.id,
                item.preset
            );
        }
    }

    #[::core::prelude::v1::test]
    fn cardinal_and_ellipse_site_indices_match_vsdx() {
        assert_eq!(site_index("rect", StencilSide::Top), 0);
        assert_eq!(site_index("rect", StencilSide::Left), 1);
        assert_eq!(site_index("rect", StencilSide::Bottom), 2);
        assert_eq!(site_index("rect", StencilSide::Right), 3);

        assert_eq!(site_index("diamond", StencilSide::Top), 0);
        assert_eq!(site_index("diamond", StencilSide::Right), 3);

        assert_eq!(site_index("ellipse", StencilSide::Top), 0);
        assert_eq!(site_index("ellipse", StencilSide::Left), 2);
        assert_eq!(site_index("ellipse", StencilSide::Bottom), 4);
        assert_eq!(site_index("ellipse", StencilSide::Right), 6);
    }

    #[::core::prelude::v1::test]
    fn glued_arrow_connects_expected_sites() {
        let a = make_diagram_shape(
            ShapeId(1),
            "ellipse",
            EmuRect::new(0, 0, 1_000_000, 500_000),
            0x70ad47,
            "Start",
        );
        let b = make_diagram_shape(
            ShapeId(2),
            "rect",
            EmuRect::new(2_000_000, 0, 1_000_000, 500_000),
            0x4472c4,
            "Process",
        );
        let (line, glue) = make_glued_arrow(ShapeId(3), &a, StencilSide::Right, &b, StencilSide::Left);
        assert!(matches!(line.kind, ShapeKind::Line(_)));
        assert_eq!(glue.start, Some((1, 6)));
        assert_eq!(glue.end, Some((2, 1)));
    }

    #[::core::prelude::v1::test]
    fn cardinal_sites_snap_only_to_vsdx_cardinal_anchors_and_respect_exclusion() {
        let ellipse = make_diagram_shape(
            ShapeId(10),
            "ellipse",
            EmuRect::new(0, 0, 1_000_000, 1_000_000),
            0x70ad47,
            "Start",
        );
        let rect = make_diagram_shape(
            ShapeId(11),
            "rect",
            EmuRect::new(1_500_000, 0, 1_000_000, 1_000_000),
            0x4472c4,
            "Next",
        );
        let ellipse_sites: Vec<u32> = cardinal_sites_of(&ellipse).into_iter().map(|(i, _)| i).collect();
        assert_eq!(ellipse_sites, vec![0, 2, 4, 6]);

        let snap = nearest_cardinal_site([&ellipse, &rect], 980_000, 500_000, VISIO_SNAP_RADIUS, None);
        assert_eq!(snap, Some((10, 6, EmuPoint::new(1_000_000, 500_000))));

        let excluded = nearest_cardinal_site([&ellipse, &rect], 1_000_000, 500_000, VISIO_SNAP_RADIUS, Some(10));
        assert_eq!(excluded, None);
    }
}
