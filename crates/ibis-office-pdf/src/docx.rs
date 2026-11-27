// SPDX-License-Identifier: AGPL-3.0-only
//! Experimental reconstruction of fixed-layout pages as editable Word content.

use std::{cell::Cell, sync::Arc};

use ibis_doc_model::{
    Block, BorderStyle, EMU_PER_POINT, Emu, EmuRect, EmuSize, FloatingAnchor, FlowBorder, FlowCell,
    FlowDocument, FlowParagraph, FlowRow, FlowRun, FlowRunKind, FlowStyle, FlowTable, MediaFormat,
    ParaFormat, Picture, Presentation, Rgb, RowHeight, SectionBreak, SectionSetup, Shape,
    ShapeKind, StyleKind, VerticalMerge,
};
use ibis_ofd::OfdContainer;
use ibis_pdf::scene::{CharBox, DisplayCmd, PageScene, PageTextIndex, PathCmd, PdfRect};
use ibis_pdf::text_layout::{TextLayout, page_text};
use ibis_pdf::{Error, Result};
use image::{ColorType, ImageEncoder, codecs::png::PngEncoder};

use super::docx_tags::{TaggedBlock, Tree};

const TOLERANCE: f32 = 0.75;

pub fn export_ofd(bytes: &[u8]) -> Result<Option<Vec<u8>>> {
    if !ibis_ofd::is_ofd(bytes) {
        return Ok(None);
    }
    export(bytes).map(Some)
}

/// Reconstruct editable content from the same scenes used by the fixed-layout viewers.
pub fn export(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut scenes = Vec::new();
    let mut tagged = None;
    if bytes.starts_with(b"%PDF-") {
        let mut pdf = ibis_pdf::PdfContainer::from_bytes(bytes.to_vec())?;
        if let Some(tree) = Tree::read(&mut pdf) {
            let mut marked = Vec::new();
            ibis_pdf::interpret::visit_marked_export_scenes(&mut pdf, |scene, ids| {
                marked.push((scene, ids));
                Ok(())
            })?;
            tagged = tree.blocks(&marked);
            scenes.extend(marked.into_iter().map(|(scene, _)| scene));
        } else {
            ibis_pdf::interpret::visit_export_scenes(&mut pdf, |scene| {
                scenes.push(scene);
                Ok(())
            })?;
        }
    } else if ibis_ofd::is_ofd(bytes) {
        let ofd =
            OfdContainer::from_bytes(bytes.to_vec()).map_err(|e| Error::new(e.to_string()))?;
        for i in 0..ofd.page_count() {
            scenes.push(
                ofd.interpret_page_scene(i)
                    .map_err(|e| Error::new(e.to_string()))?,
            );
        }
    } else {
        let xps = ibis_xps::XpsContainer::from_bytes(bytes.to_vec())
            .map_err(|e| Error::new(e.to_string()))?;
        for i in 0..xps.page_infos().len() {
            scenes.push(
                xps.interpret_page_scene(i)
                    .map_err(|e| Error::new(e.to_string()))?,
            );
        }
    }
    #[cfg(all(feature = "ocr", not(target_family = "wasm")))]
    if ibis_office_shell::ocr_setup::are_models_installed()
        && let Some(dir) = ibis_office_base::app_data::ocr_models_dir()
    {
        for scene in &mut scenes {
            if scene.text_index.chars.is_empty()
                && let Ok(ocr_index) = crate::ocr::ocr_page_scene_from_dir(&dir, scene, false)
                && !ocr_index.chars.is_empty()
            {
                scene.text_index = ocr_index;
            }
        }
    }
    export_scenes(scenes, tagged)
}

fn emu(points: f32) -> Emu {
    (f64::from(points) * EMU_PER_POINT as f64).round() as Emu
}

fn export_scenes(scenes: Vec<PageScene>, tagged: Option<Vec<Vec<TaggedBlock>>>) -> Result<Vec<u8>> {
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
    let next_id = Cell::new(flow.new_id());
    let id = || {
        let current = next_id.get();
        next_id.set(current + 1);
        current
    };
    if let Some(pages) = &tagged {
        for block in pages.iter().flatten() {
            if let TaggedBlock::Text(_, Some(level)) = block {
                Arc::make_mut(&mut flow.styles)
                    .styles
                    .entry(format!("Heading{level}"))
                    .or_insert_with(|| FlowStyle {
                        name: format!("heading {level}"),
                        kind: StyleKind::Paragraph,
                        based_on: Some("Normal".into()),
                        run: ibis_doc_model::RunFormat {
                            bold: Some(true),
                            ..Default::default()
                        },
                        ..Default::default()
                    });
            }
        }
    }
    let mut blocks = Vec::new();
    let total = scenes.len();
    if total == 0 {
        return Err(Error::new("document has no pages"));
    }
    for (index, scene) in scenes.into_iter().enumerate() {
        let mut page = append_page(
            &mut flow,
            &mut blank.presentation,
            &id,
            scene,
            tagged.as_ref().map(|pages| pages[index].as_slice()),
        )?;
        if index + 1 < total {
            if !matches!(page.last(), Some(Block::Paragraph(_))) {
                page.push(Block::Paragraph(paragraph(&flow, id(), "", 0, 0)));
            }
            if let Some(Block::Paragraph(last)) = page.last_mut() {
                last.section_break = SectionBreak::NextPage;
                last.section = Some(Arc::new(SectionSetup {
                    page: flow.page.clone(),
                    ..Default::default()
                }));
            }
        }
        blocks.extend(page);
    }
    if let Some(first) = blocks.iter_mut().find_map(|block| match block {
        Block::Paragraph(p) => Some(p),
        _ => None,
    }) {
        first.id = original_id;
    }
    flow.body = blocks;
    blank.presentation.flow = Some(flow.into());
    blank
        .source
        .save(&blank.presentation, Default::default())
        .map_err(format_error)
}

fn append_page(
    flow: &mut FlowDocument,
    presentation: &mut Presentation,
    id: &impl Fn() -> u32,
    scene: PageScene,
    tagged: Option<&[TaggedBlock]>,
) -> Result<Vec<Block>> {
    let page = scene.info.crop_box;
    if !page.width().is_finite()
        || !page.height().is_finite()
        || page.width() <= 0.0
        || page.height() <= 0.0
    {
        return Err(Error::new("invalid fixed-layout page dimensions"));
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

    let mut blocks = if let Some(tagged) = tagged {
        tagged.iter().map(|block| block.to_flow(flow, id)).collect()
    } else if let Some(grid) = grid {
        let bounds = grid.bounds();
        let (inside, outside): (Vec<_>, Vec<_>) =
            chars.iter().cloned().partition(|c| contains(bounds, c));
        let (before, after): (Vec<_>, Vec<_>) =
            outside.into_iter().partition(|c| center(c).1 < bounds.y0);
        let mut blocks = paragraphs(flow, id, before, left, top);
        blocks.push(Block::Table(grid.table(flow, id, &inside, left)));
        blocks.extend(paragraphs(flow, id, after, left, bounds.y1));
        blocks
    } else {
        paragraphs(flow, id, chars.clone(), left, top)
    };
    let first = match blocks.iter().position(|b| matches!(b, Block::Paragraph(_))) {
        Some(i) => i,
        None => {
            blocks.insert(0, Block::Paragraph(paragraph(flow, id(), "", 0, 0)));
            0
        }
    };
    let Block::Paragraph(anchor) = &mut blocks[first] else {
        unreachable!()
    };
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
        let image = presentation.add_media(MediaFormat::Png, png);
        let mut shape = Shape::new(
            presentation.new_shape_id(),
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
    Ok(blocks)
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

pub(super) fn paragraph(
    flow: &FlowDocument,
    id: u32,
    text: &str,
    left: Emu,
    before: Emu,
) -> FlowParagraph {
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

#[derive(Clone, Copy)]
struct CellArea {
    left: usize,
    right: usize,
    top: usize,
    bottom: usize,
}

struct Grid {
    cells: Vec<CellArea>,
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
            .enumerate()
            .map(|(row, y)| {
                let mut cells: Vec<_> = self
                    .cells
                    .iter()
                    .filter(|cell| cell.top <= row && row < cell.bottom)
                    .collect();
                cells.sort_by_key(|cell| cell.left);
                FlowRow {
                    id: id(),
                    height: RowHeight::AtLeast(emu(y[1] - y[0])),
                    cells: cells
                        .into_iter()
                        .map(|cell| {
                            let bounds = PdfRect {
                                x0: self.x[cell.left],
                                x1: self.x[cell.right],
                                y0: self.y[cell.top],
                                y1: self.y[cell.bottom],
                            };
                            let first_row = cell.top == row;
                            let content = if first_row {
                                text(
                                    chars
                                        .iter()
                                        .filter(|c| contains(bounds, c))
                                        .cloned()
                                        .collect(),
                                )
                            } else {
                                String::new()
                            };
                            FlowCell {
                                id: id(),
                                span: (cell.right - cell.left) as u32,
                                merge: if !first_row {
                                    VerticalMerge::Continue
                                } else if cell.bottom > row + 1 {
                                    VerticalMerge::Start
                                } else {
                                    VerticalMerge::None
                                },
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
                }
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
    // Decorative rules such as an underlined title never meet two vertical edges.
    horizontal.retain(|h| vertical.iter().filter(|v| crosses(h, v)).take(2).count() == 2);
    vertical.retain(|v| horizontal.iter().filter(|h| crosses(h, v)).take(2).count() == 2);
    let x = axes(&vertical);
    let y = axes(&horizontal);
    if !(2..=33).contains(&x.len()) || !(2..=129).contains(&y.len()) {
        return None;
    }
    let cells = grid_cells(&x, &y, &horizontal, &vertical)?;
    let grid = Grid {
        x,
        y,
        cells,
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

fn crosses(h: &Rule, v: &Rule) -> bool {
    (h.start - TOLERANCE..=h.end + TOLERANCE).contains(&v.axis)
        && (v.start - TOLERANCE..=v.end + TOLERANCE).contains(&h.axis)
}

fn covers(rules: &[Rule], axis: f32, start: f32, end: f32) -> bool {
    let mut spans: Vec<_> = rules
        .iter()
        .filter(|r| (r.axis - axis).abs() < TOLERANCE)
        .collect();
    spans.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut covered = start;
    for rule in spans {
        if rule.start > covered + TOLERANCE {
            break;
        }
        covered = covered.max(rule.end);
        if covered >= end - TOLERANCE {
            return true;
        }
    }
    false
}

fn grid_cells(x: &[f32], y: &[f32], h: &[Rule], v: &[Rule]) -> Option<Vec<CellArea>> {
    let (cols, rows) = (x.len() - 1, y.len() - 1);
    let horizontal: Vec<Vec<_>> = y
        .iter()
        .map(|&axis| {
            x.windows(2)
                .map(|edge| covers(h, axis, edge[0], edge[1]))
                .collect()
        })
        .collect();
    let vertical: Vec<Vec<_>> = y
        .windows(2)
        .map(|edge| {
            x.iter()
                .map(|&axis| covers(v, axis, edge[0], edge[1]))
                .collect()
        })
        .collect();
    if horizontal[0]
        .iter()
        .chain(&horizontal[rows])
        .any(|covered| !covered)
        || vertical.iter().any(|row| !row[0] || !row[cols])
    {
        return None;
    }
    let mut seen = vec![false; rows * cols];
    let mut areas = Vec::new();
    for row in 0..rows {
        for col in 0..cols {
            if seen[row * cols + col] {
                continue;
            }
            let mut area = CellArea {
                left: col,
                right: col + 1,
                top: row,
                bottom: row + 1,
            };
            let mut pending = vec![(row, col)];
            let mut count = 0;
            seen[row * cols + col] = true;
            while let Some((r, c)) = pending.pop() {
                area.left = area.left.min(c);
                area.right = area.right.max(c + 1);
                area.top = area.top.min(r);
                area.bottom = area.bottom.max(r + 1);
                count += 1;
                for (has_wall, next_r, next_c) in [
                    (horizontal[r][c], r.wrapping_sub(1), c),
                    (horizontal[r + 1][c], r + 1, c),
                    (vertical[r][c], r, c.wrapping_sub(1)),
                    (vertical[r][c + 1], r, c + 1),
                ] {
                    if !has_wall && !seen[next_r * cols + next_c] {
                        seen[next_r * cols + next_c] = true;
                        pending.push((next_r, next_c));
                    }
                }
            }
            if count != (area.right - area.left) * (area.bottom - area.top)
                || (area.top + 1..area.bottom)
                    .any(|r| horizontal[r][area.left..area.right].iter().any(|v| *v))
                || (area.top..area.bottom)
                    .any(|r| vertical[r][area.left + 1..area.right].iter().any(|v| *v))
            {
                return None;
            }
            areas.push(area);
        }
    }
    (areas.len() > 1).then_some(areas)
}

fn axes(rules: &[Rule]) -> Vec<f32> {
    let mut coordinates: Vec<_> = rules.iter().map(|r| r.axis).collect();
    coordinates.sort_by(f32::total_cmp);
    coordinates.dedup_by(|a, b| (*a - *b).abs() < TOLERANCE);
    coordinates
}
