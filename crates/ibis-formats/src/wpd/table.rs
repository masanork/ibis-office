// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.

use ibis_doc_model::{
    Emu, Rgb,
    flow::{
        Block, CellAnchor, FlowAlign, FlowBorder, FlowCell, FlowParagraph, FlowRow, FlowTable,
        RowHeight, VerticalMerge,
    },
};

use super::appearance::{Packets, default_line, references, rgb};
use super::{Error, malformed, next_id};

// A WordPerfect unit is 1/1200 inch.
pub(super) fn units(value: u16) -> Emu {
    Emu::from(value) * 762
}

fn word(data: &[u8], offset: usize) -> Result<u16, Error> {
    let bytes = data.get(offset..offset + 2).ok_or_else(malformed)?;
    Ok(u16::from_le_bytes(bytes.try_into().unwrap()))
}

/// The non-deletable part of a variable-length control, after optional prefix IDs.
pub(super) fn payload(record: &[u8]) -> Result<&[u8], Error> {
    let offset = if record[4] & 0x80 != 0 {
        6 + usize::from(record[5]) * 2
    } else {
        5
    };
    let contents = &record[..record.len() - 3];
    let size = usize::from(word(contents, offset)?);
    contents
        .get(offset + 2..offset + 2 + size)
        .ok_or_else(malformed)
}

#[derive(Clone, Copy, Default)]
struct Column {
    attributes: u16,
    align: Option<FlowAlign>,
    left: Emu,
    right: Emu,
}

fn alignment(value: u8) -> Option<FlowAlign> {
    match value & 7 {
        0 => Some(FlowAlign::Left),
        1 => Some(FlowAlign::Justify),
        2 => Some(FlowAlign::Center),
        3 => Some(FlowAlign::Right),
        4 => Some(FlowAlign::Distribute),
        _ => None,
    }
}

pub(super) struct CellInfo {
    across: usize,
    down: usize,
    covered: bool,
    attributes: Option<u16>,
    align: Option<FlowAlign>,
    anchor: CellAnchor,
    header: bool,
    height: RowHeight,
    fill: Option<Rgb>,
    color: Option<Rgb>,
    lines: [Option<Option<FlowBorder>>; 4],
    off: u8,
}

impl Default for CellInfo {
    fn default() -> Self {
        Self {
            across: 1,
            down: 1,
            covered: false,
            attributes: None,
            align: None,
            anchor: CellAnchor::Top,
            header: false,
            height: RowHeight::Auto,
            fill: None,
            color: None,
            lines: [None; 4],
            off: 0,
        }
    }
}

impl CellInfo {
    pub(super) fn read(record: &[u8], packets: Packets<'_>) -> Result<Self, Error> {
        let payload = payload(record)?;
        // D0 begins with the length of formatter data before the embedded controls.
        let start = 2 + usize::from(word(payload, 0)?);
        let mut remaining = payload.get(start..).ok_or_else(malformed)?;
        let mut info = Self::default();
        let ids = references(record).ok_or_else(malformed)?;
        for (side, offset) in [0, 6, 4, 2].into_iter().enumerate() {
            info.lines[side] = appearance_id(ids, offset).and_then(|pid| packets.line(pid));
        }
        info.fill = appearance_id(ids, 8).and_then(|pid| packets.fill(pid));
        while let Some(&code) = remaining.first() {
            let len = match code {
                0x80 => 5,
                0x81 | 0x8e | 0x8f => usize::from(word(remaining, 1)?),
                0x82 | 0x83 | 0x85 => 4,
                0x84 => 9,
                0x86 => 10,
                0x87 | 0x88 => 6,
                0x89 => 11,
                0x8b | 0x8c => 3,
                0x8d => 1,
                _ => return Err(malformed()),
            };
            let item = remaining
                .get(..len)
                .filter(|item| {
                    item.last() == Some(&code) && (!matches!(code, 0x81 | 0x8e | 0x8f) || len >= 6)
                })
                .ok_or_else(malformed)?;
            match code {
                0x80 => {
                    info.header = item[1] & 4 != 0;
                    if item[1] & 2 != 0 {
                        let height = units(word(item, 2)?);
                        info.height = if item[1] & 16 != 0 {
                            RowHeight::AtLeast(height)
                        } else {
                            RowHeight::Exact(height)
                        };
                    }
                }
                0x84 => {
                    if item[1] & 1 != 0 {
                        info.attributes = Some(word(item, 4)?);
                    }
                    if item[1] & 2 != 0 {
                        info.align = alignment(item[2]);
                    }
                    info.anchor = match item[3] & 3 {
                        1 => CellAnchor::Center,
                        2 => CellAnchor::Bottom,
                        _ => CellAnchor::Top,
                    };
                }
                0x85 => {
                    info.covered = (item[1] | item[2]) & 0x80 != 0;
                    info.across = usize::from(item[1] & 0x7f).max(1);
                    info.down = usize::from(item[2] & 0x7f).max(1);
                }
                0x86 => info.fill = rgb(&item[1..]),
                0x87 => info.color = rgb(&item[1..]),
                0x8b => info.off = item[1],
                _ => {}
            }
            remaining = &remaining[len..];
        }
        Ok(info)
    }
}

fn appearance_id(ids: &[u8], offset: usize) -> Option<u16> {
    super::appearance::word(ids, offset)
}

#[derive(Clone, Copy, Default)]
struct Merge {
    // Only the first grid column stores a merge. The cursor skips its entire span.
    span: usize,
    end_row: usize,
    borders: [Option<Option<FlowBorder>>; 4],
    fill: Option<Rgb>,
}

pub(super) struct Table {
    model: FlowTable,
    columns: Vec<Column>,
    default_fill: Option<Rgb>,
    merges: Vec<Merge>,
    cursor: usize,
    pub(super) started: bool,
    pub(super) active: bool,
    pub(super) attributes: u16,
    pub(super) align: Option<FlowAlign>,
}

impl Table {
    pub(super) fn new(id: u32, record: &[u8], packets: Packets<'_>) -> Result<Self, Error> {
        let data = payload(record)?;
        let position = *data.get(1).ok_or_else(malformed)? & 7;
        let ids = references(record).ok_or_else(malformed)?;
        let mut line = appearance_id(ids, 4)
            .and_then(|pid| packets.line(pid))
            .unwrap_or_else(|| Some(default_line()));
        let default_fill = appearance_id(ids, 8).and_then(|pid| packets.fill(pid));
        let outer = appearance_id(ids, 2).and_then(|pid| packets.border(pid));
        let color_override = (data[0] & 1 != 0).then(|| rgb(data.get(4..)?)).flatten();
        if let (Some(line), Some(color)) = (&mut line, color_override) {
            line.color = color;
        }
        let mut model = FlowTable {
            id,
            borders: [Some(line); 6],
            align: match position {
                1 => FlowAlign::Right,
                2 => FlowAlign::Center,
                _ => FlowAlign::Left,
            },
            fixed_percent: (position == 3).then_some(5000),
            ..Default::default()
        };
        if let Some(outer) = outer {
            for (slot, border) in model.borders.iter_mut().zip(outer) {
                *slot = Some(border.map(|mut line| {
                    line.color = color_override.unwrap_or(line.color);
                    line
                }));
            }
        }
        Ok(Self {
            model,
            default_fill,
            columns: Vec::new(),
            merges: Vec::new(),
            cursor: 0,
            started: false,
            active: false,
            attributes: 0,
            align: None,
        })
    }

    pub(super) fn column(&mut self, record: &[u8]) -> Result<(), Error> {
        if self.started {
            return Err(malformed());
        }
        let data = payload(record)?;
        let align = alignment(*data.get(11).ok_or_else(malformed)?);
        self.columns.push(Column {
            attributes: word(data, 7)?,
            align,
            left: units(word(data, 3)?),
            right: units(word(data, 5)?),
        });
        self.model.columns.push(units(word(data, 1)?).max(762));
        self.merges.push(Merge::default());
        Ok(())
    }

    pub(super) fn start(&mut self) -> Result<(), Error> {
        if self.started || self.columns.is_empty() {
            return Err(malformed());
        }
        self.started = true;
        Ok(())
    }

    pub(super) fn push(&mut self, block: Block) {
        if let Some(cell) = self
            .model
            .rows
            .last_mut()
            .and_then(|row| row.cells.last_mut())
            .filter(|_| self.active)
        {
            cell.blocks.push(block);
        }
    }

    pub(super) fn cell(
        &mut self,
        new_row: bool,
        info: CellInfo,
        ids: &mut u32,
    ) -> Result<(), Error> {
        if !self.started {
            return Err(malformed());
        }
        self.active = false;
        if new_row {
            self.finish_row(ids)?;
            self.cursor = 0;
            self.model.rows.push(FlowRow {
                id: next_id(ids),
                header: info.header,
                height: info.height,
                ..Default::default()
            });
        }
        if self.model.rows.is_empty() {
            return Err(malformed());
        }
        if info.covered {
            // Covered-cell records may be present or omitted. The span on the
            // original cell determines the actual grid, independently of them.
            return Ok(());
        }
        while self.continuation(ids) {}
        let end = self.cursor + info.across;
        let grid = self
            .merges
            .get_mut(self.cursor..end)
            .ok_or_else(malformed)?;
        let row = self.model.rows.len();
        if grid.iter().any(|merge| merge.end_row >= row) {
            return Err(malformed());
        }
        let borders = std::array::from_fn(|side| {
            if info.off & [4, 2, 8, 1][side] != 0 {
                return Some(None);
            }
            let line = info.lines[side];
            let Some(color) = info.color else { return line };
            let fallback = self.model.borders[4].flatten();
            Some(line.unwrap_or(fallback).map(|mut line| {
                line.color = color;
                line
            }))
        });
        let fill = info.fill.or(self.default_fill);
        if info.down > 1 {
            grid[0] = Merge {
                span: info.across,
                end_row: row + info.down - 1,
                borders,
                fill,
            };
        }
        let column = self.columns[self.cursor];
        self.attributes = info.attributes.unwrap_or(column.attributes);
        self.align = info.align.or(column.align);
        self.model.rows.last_mut().unwrap().cells.push(FlowCell {
            id: next_id(ids),
            span: info.across as u32,
            merge: if info.down > 1 {
                VerticalMerge::Start
            } else {
                VerticalMerge::None
            },
            margins: [0, self.columns[end - 1].right, 0, column.left],
            anchor: info.anchor,
            shading: fill,
            shading_fill: fill,
            own_borders: borders,
            ..Default::default()
        });
        self.cursor = end;
        self.active = true;
        Ok(())
    }

    fn continuation(&mut self, ids: &mut u32) -> bool {
        let Some(&merge) = self.merges.get(self.cursor) else {
            return false;
        };
        if merge.end_row < self.model.rows.len() {
            return false;
        }
        let [_, right, bottom, left] = merge.borders;
        let bottom = if merge.end_row == self.model.rows.len() {
            bottom
        } else {
            Some(None)
        };
        let borders = [Some(None), right, bottom, left];
        self.model.rows.last_mut().unwrap().cells.push(FlowCell {
            id: next_id(ids),
            span: merge.span as u32,
            own_borders: borders,
            shading: merge.fill,
            shading_fill: merge.fill,
            merge: VerticalMerge::Continue,
            // Even a covered DOCX cell needs a block of its own.
            blocks: vec![Block::Paragraph(FlowParagraph {
                id: next_id(ids),
                ..Default::default()
            })],
            ..Default::default()
        });
        self.cursor += merge.span;
        true
    }

    fn finish_row(&mut self, ids: &mut u32) -> Result<(), Error> {
        if !self.model.rows.is_empty() {
            while self.continuation(ids) {}
            if self.cursor != self.columns.len() {
                return Err(malformed());
            }
        }
        Ok(())
    }

    pub(super) fn finish(mut self, ids: &mut u32) -> Result<FlowTable, Error> {
        if self.model.rows.is_empty()
            || self
                .merges
                .iter()
                .any(|merge| merge.end_row > self.model.rows.len())
        {
            return Err(malformed());
        }
        self.finish_row(ids)?;
        Ok(self.model)
    }
}
