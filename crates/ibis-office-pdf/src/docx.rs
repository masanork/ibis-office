// SPDX-License-Identifier: AGPL-3.0-only
//! Experimental reconstruction of a single horizontal OFD page as editable Word content.

use std::{cell::Cell, sync::Arc};

use ibis_doc_model::{
    Block, BorderStyle, EMU_PER_POINT, Emu, EmuRect, EmuSize, FloatingAnchor, FlowBorder, FlowCell,
    FlowDocument, FlowParagraph, FlowRow, FlowRun, FlowRunKind, FlowTable, MediaFormat, ParaFormat,
    Picture, Rgb, RowHeight, Shape, ShapeKind,
};
use ibis_ofd::OfdContainer;
use ibis_pdf::scene::{CharBox, DisplayCmd, PageScene, PageTextIndex, PathCmd, PdfRect};
use ibis_pdf::text_layout::{TextLayout, page_text};
use ibis_pdf::{Error, Result};
use image::{ColorType, ImageEncoder, codecs::png::PngEncoder};

const TOLERANCE: f32 = 0.75;

pub fn export_ofd(bytes: &[u8]) -> Result<Option<Vec<u8>>> {
    if !ibis_ofd::is_ofd(bytes) {
        return Ok(None);
    }
    let ofd = OfdContainer::from_bytes(bytes.to_vec()).map_err(|e| Error::new(e.to_string()))?;
    if ofd.page_count() != 1 {
        return Err(Error::new(
            "experimental OFD-to-DOCX export requires a single page",
        ));
    }
    let scene = ofd
        .interpret_page_scene(0)
        .map_err(|e| Error::new(e.to_string()))?;
    if scene.text_index.chars.iter().any(|c| c.vertical) {
        return Err(Error::new(
            "experimental OFD-to-DOCX export requires horizontal text",
        ));
    }
    export_scene(scene).map(Some)
}

fn emu(points: f32) -> Emu {
    (f64::from(points) * EMU_PER_POINT as f64).round() as Emu
}

fn export_scene(scene: PageScene) -> Result<Vec<u8>> {
    let format_error = |e: ibis_formats::Error| Error::new(e.to_string());
    let mut blank = ibis_formats::open(
        ibis_formats::blank_file(ibis_formats::Kind::Document).map_err(format_error)?,
    )
    .map_err(format_error)?;
    let mut flow = Arc::unwrap_or_clone(blank.presentation.flow.take().expect("blank document"));
    let Block::Paragraph(original) = &flow.body[0] else {
        return Err(Error::new("missing blank document paragraph"));
    };
    let original_id = original.id;
    let page = scene.info.crop_box;
    if !page.width().is_finite()
        || !page.height().is_finite()
        || page.width() <= 0.0
        || page.height() <= 0.0
    {
        return Err(Error::new("invalid OFD page dimensions"));
    }
    flow.page.size = EmuSize::new(emu(page.width()), emu(page.height()));
    let chars = &scene.text_index.chars;
    let grid = ruled_grid(&scene.display_list, chars);
    let left = chars
        .iter()
        .map(|c| c.rect_pt.x0)
        .chain(grid.iter().map(|g| g.x[0]))
        .reduce(f32::min)
        .unwrap_or(page.x0 + 36.0);
    let top = chars
        .iter()
        .map(|c| c.rect_pt.y0)
        .chain(grid.iter().map(|g| g.y[0]))
        .reduce(f32::min)
        .unwrap_or(page.y0 + 36.0);
    flow.page.left = emu((left - page.x0).max(0.0));
    flow.page.top = emu((top - page.y0).max(0.0));
    flow.page.right = flow.page.left.min(flow.page.size.width / 4);
    flow.page.bottom = emu(24.0).min(flow.page.size.height / 4);

    let next_id = Cell::new(flow.new_id());
    let id = || {
        let current = next_id.get();
        next_id.set(current + 1);
        current
    };
    let mut blocks = if let Some(grid) = grid {
        let bounds = grid.bounds();
        let (inside, outside): (Vec<_>, Vec<_>) =
            chars.iter().cloned().partition(|c| contains(bounds, c));
        let (before, after): (Vec<_>, Vec<_>) =
            outside.into_iter().partition(|c| center(c).1 < bounds.y0);
        let mut blocks = paragraphs(&flow, &id, before, left, top);
        blocks.push(Block::Table(grid.table(&flow, &id, &inside, left)));
        blocks.extend(paragraphs(&flow, &id, after, left, bounds.y1));
        blocks
    } else {
        paragraphs(&flow, &id, chars.clone(), left, top)
    };
    let first = match blocks.iter().position(|b| matches!(b, Block::Paragraph(_))) {
        Some(i) => i,
        None => {
            blocks.insert(0, Block::Paragraph(FlowParagraph::default()));
            0
        }
    };
    let Block::Paragraph(anchor) = &mut blocks[first] else {
        unreachable!()
    };
    anchor.id = original_id;
    for cmd in scene.display_list {
        let DisplayCmd::DrawImage {
            bounds,
            width,
            height,
            rgba,
        } = cmd
        else {
            continue;
        };
        if bounds.width() <= 0.0 || bounds.height() <= 0.0 {
            continue;
        }
        let mut png = Vec::new();
        PngEncoder::new(&mut png)
            .write_image(&rgba, width, height, ColorType::Rgba8.into())
            .map_err(|e| Error::new(e.to_string()))?;
        let image = blank.presentation.add_media(MediaFormat::Png, png);
        let mut shape = Shape::new(
            blank.presentation.new_shape_id(),
            EmuRect::new(
                emu(bounds.x0 - page.x0),
                emu(bounds.y0 - page.y0),
                emu(bounds.width()),
                emu(bounds.height()),
            ),
            ShapeKind::Picture(Picture::new(image)),
        );
        shape.floating = Some(FloatingAnchor {
            offset_x: shape.frame.x,
            offset_y: shape.frame.y,
            relative_h: "page".into(),
            relative_v: "page".into(),
            wrap: "none".into(),
            ..Default::default()
        });
        anchor.runs.push(FlowRun::new(
            FlowRunKind::Shape(Box::new(shape)),
            Default::default(),
        ));
    }
    flow.body = blocks;
    blank.presentation.flow = Some(flow.into());
    blank
        .source
        .save(&blank.presentation, Default::default())
        .map_err(format_error)
}

fn center(c: &CharBox) -> (f32, f32) {
    (
        (c.rect_pt.x0 + c.rect_pt.x1) / 2.0,
        (c.rect_pt.y0 + c.rect_pt.y1) / 2.0,
    )
}

fn contains(rect: PdfRect, c: &CharBox) -> bool {
    let (x, y) = center(c);
    x >= rect.x0 && x < rect.x1 && y >= rect.y0 && y < rect.y1
}

fn text(chars: Vec<CharBox>) -> String {
    page_text(&PageTextIndex { chars }, TextLayout::Reading)
}

fn paragraph(flow: &FlowDocument, id: u32, text: &str, left: Emu, before: Emu) -> FlowParagraph {
    let mut p = FlowParagraph {
        id,
        direct: ParaFormat {
            left: Some(left),
            before: Some(before),
            after: Some(0),
            ..Default::default()
        },
        ..Default::default()
    };
    for (index, line) in text.lines().enumerate() {
        if index > 0 {
            p.runs
                .push(FlowRun::new(FlowRunKind::LineBreak, Default::default()));
        }
        p.runs.push(FlowRun::new(
            FlowRunKind::Text(line.into()),
            Default::default(),
        ));
    }
    p.resolve(&flow.styles);
    p
}

fn paragraphs(
    flow: &FlowDocument,
    id: &impl Fn() -> u32,
    mut chars: Vec<CharBox>,
    left: f32,
    mut last_bottom: f32,
) -> Vec<Block> {
    chars.retain(|c| !c.ch.is_whitespace() && !c.ch.is_control());
    chars.sort_by(|a, b| a.rect_pt.y0.total_cmp(&b.rect_pt.y0));
    let mut groups: Vec<Vec<CharBox>> = Vec::new();
    for c in chars {
        if let Some(group) = groups.last_mut() {
            let prev = group.last().unwrap().rect_pt;
            if c.rect_pt.y0 - prev.y1 <= prev.height().max(1.0) * 0.65 {
                group.push(c);
                continue;
            }
        }
        groups.push(vec![c]);
    }
    groups
        .into_iter()
        .filter_map(|chars| {
            let y = chars[0].rect_pt.y0;
            let x = chars.iter().map(|c| c.rect_pt.x0).reduce(f32::min)?;
            let bottom = chars.iter().map(|c| c.rect_pt.y1).reduce(f32::max)?;
            let before = emu((y - last_bottom).max(0.0));
            last_bottom = bottom;
            let content = text(chars).lines().collect::<Vec<_>>().join(" ");
            Some(Block::Paragraph(paragraph(
                flow,
                id(),
                &content,
                emu((x - left).max(0.0)),
                before,
            )))
        })
        .collect()
}

struct Rule {
    axis: f32,
    start: f32,
    end: f32,
}

struct Grid {
    x: Vec<f32>,
    y: Vec<f32>,
    border: FlowBorder,
}

impl Grid {
    fn bounds(&self) -> PdfRect {
        PdfRect {
            x0: self.x[0],
            y0: self.y[0],
            x1: *self.x.last().unwrap(),
            y1: *self.y.last().unwrap(),
        }
    }

    fn table(
        self,
        flow: &FlowDocument,
        id: &impl Fn() -> u32,
        chars: &[CharBox],
        left: f32,
    ) -> FlowTable {
        let rows = self
            .y
            .windows(2)
            .map(|y| FlowRow {
                id: id(),
                height: RowHeight::AtLeast(emu(y[1] - y[0])),
                cells: self
                    .x
                    .windows(2)
                    .map(|x| {
                        let bounds = PdfRect {
                            x0: x[0],
                            y0: y[0],
                            x1: x[1],
                            y1: y[1],
                        };
                        let content = text(
                            chars
                                .iter()
                                .filter(|c| contains(bounds, c))
                                .cloned()
                                .collect(),
                        );
                        FlowCell {
                            id: id(),
                            span: 1,
                            blocks: vec![Block::Paragraph(paragraph(
                                flow,
                                id(),
                                content.trim_end(),
                                0,
                                0,
                            ))],
                            ..Default::default()
                        }
                    })
                    .collect(),
                ..Default::default()
            })
            .collect();
        FlowTable {
            id: id(),
            borders: [Some(Some(self.border)); 6],
            columns: self.x.windows(2).map(|x| emu(x[1] - x[0])).collect(),
            indent: emu(self.x[0] - left),
            rows,
            ..Default::default()
        }
    }
}

fn ruled_grid(list: &[DisplayCmd], chars: &[CharBox]) -> Option<Grid> {
    let mut horizontal = Vec::new();
    let mut vertical = Vec::new();
    let mut border = None;
    for cmd in list {
        let DisplayCmd::DrawPath {
            commands,
            stroke_rgba: Some(color),
            stroke_width,
            ..
        } = cmd
        else {
            continue;
        };
        let mut start = None;
        let mut prev = None;
        for cmd in commands {
            let next = match *cmd {
                PathCmd::MoveTo(x, y) => {
                    start = Some((x, y));
                    prev = start;
                    continue;
                }
                PathCmd::LineTo(x, y) => Some((x, y)),
                PathCmd::Close => start,
                _ => None,
            };
            if let (Some((x0, y0)), Some((x1, y1))) = (prev, next) {
                let rule = |axis, a: f32, b: f32| Rule {
                    axis,
                    start: a.min(b),
                    end: a.max(b),
                };
                if (y1 - y0).abs() < TOLERANCE && (x1 - x0).abs() >= 4.0 {
                    horizontal.push(rule(y0, x0, x1));
                } else if (x1 - x0).abs() < TOLERANCE && (y1 - y0).abs() >= 4.0 {
                    vertical.push(rule(x0, y0, y1));
                }
                border.get_or_insert(FlowBorder {
                    width: emu(*stroke_width).max(EMU_PER_POINT / 8),
                    color: Rgb(u32::from_be_bytes([0, color[0], color[1], color[2]])),
                    style: BorderStyle::Single,
                    space: 0,
                    art: None,
                });
            }
            prev = next;
        }
    }
    let x = axes(&vertical);
    let y = axes(&horizontal);
    if !(3..=33).contains(&x.len()) || !(3..=129).contains(&y.len()) {
        return None;
    }
    let complete = |rules: &[Rule], axes: &[f32], cross: &[f32]| {
        axes.iter().all(|a| {
            cross.windows(2).all(|b| {
                rules.iter().any(|r| {
                    (r.axis - a).abs() < TOLERANCE
                        && r.start <= b[0] + TOLERANCE
                        && r.end >= b[1] - TOLERANCE
                })
            })
        })
    };
    if !complete(&vertical, &x, &y) || !complete(&horizontal, &y, &x) {
        return None;
    }
    let grid = Grid {
        x,
        y,
        border: border?,
    };
    let rect = grid.bounds();
    // Text alongside the grid is outside this single-column prototype.
    if chars.iter().any(|c| {
        let (_, y) = center(c);
        y >= rect.y0 && y < rect.y1 && !contains(rect, c)
    }) || !chars.iter().any(|c| contains(rect, c))
    {
        return None;
    }
    Some(grid)
}

fn axes(rules: &[Rule]) -> Vec<f32> {
    let mut coordinates: Vec<_> = rules.iter().map(|r| r.axis).collect();
    coordinates.sort_by(f32::total_cmp);
    coordinates.dedup_by(|a, b| (*a - *b).abs() < TOLERANCE);
    coordinates
}
