// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (c) ibis inc.
// Additional terms under AGPL-3.0 section 7(b): see NOTICE.
//! UI-independent helpers for AI Assistant spreadsheet tools.
//!
//! Converts between A1-style references / lightweight cell snapshots and
//! [`Command`] / [`SheetCommand`] values so the AI tool adapter can be
//! tested without a window or HTTP client.

use crate::command::Command;
use crate::drawing::Rgb;
use crate::model::SlideId;
use crate::sheet::{
    Border, BorderLine, CellAlign, CellRange, CellValue, CellVerticalAlign, Sheet, Workbook,
    cell_name,
};
use crate::sheet_command::{Axis, CellEdit, CellInput, Edge, SheetCommand, StyleChange};

/// Summary of one sheet in a workbook.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SheetSummary {
    pub name: String,
    pub slide: Option<SlideId>,
    pub active: bool,
    pub used_range: Option<String>,
    pub cell_count: usize,
}

/// Overview of a workbook for LLM context.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkbookOverview {
    pub sheets: Vec<SheetSummary>,
    pub active_sheet: Option<String>,
    pub selection: Option<String>,
}

/// Lightweight snapshot of a single non-empty cell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CellSnapshot {
    pub cell: String,
    pub row: u32,
    pub column: u32,
    pub text: String,
    pub formula: Option<String>,
}

/// Input value for `set_cells`.
#[derive(Clone, Debug, PartialEq)]
pub enum AiCellValue {
    Empty,
    Number(f64),
    Text(String),
    Bool(bool),
    Formula(String),
}

/// Single cell edit requested by an AI tool.
#[derive(Clone, Debug, PartialEq)]
pub struct AiCellEdit {
    pub cell: String,
    pub value: AiCellValue,
}

/// Border preset requested by `set_format`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AiBorderStyle {
    All,
    Outline,
    Bottom,
    None,
}

/// Formatting options requested by `set_format`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AiCellFormat {
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub text_color: Option<String>,
    pub fill_color: Option<Option<String>>,
    pub number_format: Option<String>,
    pub align: Option<CellAlign>,
    pub vertical_align: Option<CellVerticalAlign>,
    pub wrap_text: Option<bool>,
    pub font_size_pt: Option<f32>,
    pub border: Option<AiBorderStyle>,
}

/// Parse an A1-style cell reference (e.g., `"A1"`, `"$B$12"`) into `(row, column)` (0-based).
pub fn parse_cell_ref(reference: &str) -> Option<(u32, u32)> {
    let corner = ibis_calc::formula::parser::parse_cell(reference.trim())?;
    Some((corner.row, corner.column))
}

/// Parse an A1-style range (e.g., `"A1:C10"` or `"B2"`) into a normalized [`CellRange`].
pub fn parse_cell_range(range: &str) -> Option<CellRange> {
    let trimmed = range.trim();
    if let Some((start, end)) = trimmed.split_once(':') {
        let (r1, c1) = parse_cell_ref(start)?;
        let (r2, c2) = parse_cell_ref(end)?;
        Some(CellRange {
            first_row: r1.min(r2),
            first_column: c1.min(c2),
            last_row: r1.max(r2),
            last_column: c1.max(c2),
        })
    } else {
        let (r, c) = parse_cell_ref(trimmed)?;
        Some(CellRange {
            first_row: r,
            first_column: c,
            last_row: r,
            last_column: c,
        })
    }
}

/// Format a [`CellRange`] as A1 notation (e.g., `"A1:C10"` or `"B2"`).
pub fn format_range(range: &CellRange) -> String {
    let start = cell_name(range.first_column, range.first_row);
    if range.first_row == range.last_row && range.first_column == range.last_column {
        start
    } else {
        format!("{start}:{}", cell_name(range.last_column, range.last_row))
    }
}

/// Parse a `#RRGGBB` or `RRGGBB` hex color into [`Rgb`].
pub fn parse_hex_rgb(hex: &str) -> Result<Rgb, String> {
    let clean = hex.trim().strip_prefix('#').unwrap_or(hex.trim());
    if clean.len() != 6 || !clean.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("invalid hex color '{hex}', expected #RRGGBB"));
    }
    let r = u8::from_str_radix(&clean[0..2], 16).map_err(|e| e.to_string())?;
    let g = u8::from_str_radix(&clean[2..4], 16).map_err(|e| e.to_string())?;
    let b = u8::from_str_radix(&clean[4..6], 16).map_err(|e| e.to_string())?;
    Ok(Rgb::from_rgb(r, g, b))
}

/// Compute the bounding box of cells that have non-empty values or formulas.
pub fn sheet_used_range(sheet: &Sheet) -> Option<CellRange> {
    let mut bounds: Option<CellRange> = None;
    for cell in &sheet.cells {
        if matches!(cell.value, CellValue::Empty) && cell.formula.is_none() && cell.text.is_empty() {
            continue;
        }
        bounds = Some(match bounds {
            None => CellRange {
                first_row: cell.row,
                first_column: cell.column,
                last_row: cell.row,
                last_column: cell.column,
            },
            Some(mut b) => {
                b.first_row = b.first_row.min(cell.row);
                b.first_column = b.first_column.min(cell.column);
                b.last_row = b.last_row.max(cell.row);
                b.last_column = b.last_column.max(cell.column);
                b
            }
        });
    }
    bounds
}

/// Build an overview of the workbook and the active sheet's selection.
pub fn workbook_overview<'a>(
    workbook: &'a Workbook,
    active_slide: SlideId,
    sheet_for_slide: &dyn Fn(SlideId) -> Option<&'a Sheet>,
    selection: Option<CellRange>,
) -> WorkbookOverview {
    let mut sheets = Vec::with_capacity(workbook.sheets.len());
    let mut active_sheet = None;
    for entry in &workbook.sheets {
        let active = entry.slide == Some(active_slide);
        if active {
            active_sheet = Some(entry.name.clone());
        }
        let sheet = entry
            .slide
            .and_then(|id| sheet_for_slide(id))
            .or(entry.hidden.as_deref());
        let (used_range, cell_count) = match sheet {
            Some(s) => (
                sheet_used_range(s).map(|r| format_range(&r)),
                s.cells.iter().filter(|c| !matches!(c.value, CellValue::Empty) || c.formula.is_some()).count(),
            ),
            None => (None, 0),
        };
        sheets.push(SheetSummary {
            name: entry.name.clone(),
            slide: entry.slide,
            active,
            used_range,
            cell_count,
        });
    }
    WorkbookOverview {
        sheets,
        active_sheet,
        selection: selection.map(|r| format_range(&r)),
    }
}

/// Read non-empty cells in `range`, capped at `max_cells` total grid cells.
pub fn read_sheet_range(
    sheet: &Sheet,
    range: CellRange,
    max_cells: usize,
) -> Result<Vec<CellSnapshot>, String> {
    let rows = (range.last_row - range.first_row) as usize + 1;
    let cols = (range.last_column - range.first_column) as usize + 1;
    let total = rows.saturating_mul(cols);
    if total > max_cells {
        return Err(format!(
            "range {} spans {total} cells, exceeding limit of {max_cells}; request a smaller range",
            format_range(&range)
        ));
    }
    let mut out = Vec::new();
    for cell in sheet.cells_in_rows(range.first_row, range.last_row) {
        if cell.column < range.first_column || cell.column > range.last_column {
            continue;
        }
        if matches!(cell.value, CellValue::Empty) && cell.formula.is_none() && cell.text.is_empty() {
            continue;
        }
        let formula = cell
            .formula
            .as_deref()
            .map(|f| format!("={}", sheet.formula_shown(cell.row, cell.column, f)));
        out.push(CellSnapshot {
            cell: cell_name(cell.column, cell.row),
            row: cell.row,
            column: cell.column,
            text: cell.text.clone(),
            formula,
        });
    }
    Ok(out)
}

/// Convert a batch of [`AiCellEdit`]s into a [`Command`].
pub fn build_set_cells_command(slide: SlideId, edits: Vec<AiCellEdit>) -> Result<Command, String> {
    if edits.is_empty() {
        return Err("edits must not be empty".to_string());
    }
    let mut cell_edits = Vec::with_capacity(edits.len());
    for edit in edits {
        let (row, column) = parse_cell_ref(&edit.cell)
            .ok_or_else(|| format!("invalid cell reference '{}'", edit.cell))?;
        let input = match edit.value {
            AiCellValue::Empty => CellInput::Clear,
            AiCellValue::Number(n) => CellInput::Value(CellValue::Number(n)),
            AiCellValue::Text(s) => CellInput::Value(CellValue::Text(s)),
            AiCellValue::Bool(b) => CellInput::Value(CellValue::Bool(b)),
            AiCellValue::Formula(f) => {
                let stripped = f.trim().strip_prefix('=').unwrap_or(f.trim()).to_string();
                if stripped.is_empty() {
                    return Err(format!("empty formula for cell '{}'", edit.cell));
                }
                CellInput::Formula(stripped)
            }
        };
        cell_edits.push(CellEdit { row, column, input });
    }
    Ok(Command::Sheet(SheetCommand::SetCells {
        slide,
        edits: cell_edits,
    }))
}

/// Convert [`AiCellFormat`] options into a [`Command`] (possibly grouped).
pub fn build_set_format_command(
    slide: SlideId,
    ranges: Vec<CellRange>,
    format: AiCellFormat,
) -> Result<Command, String> {
    if ranges.is_empty() {
        return Err("ranges must not be empty".to_string());
    }
    let mut changes = Vec::new();
    if let Some(bold) = format.bold {
        changes.push(StyleChange::Bold(bold));
    }
    if let Some(italic) = format.italic {
        changes.push(StyleChange::Italic(italic));
    }
    if let Some(underline) = format.underline {
        changes.push(StyleChange::Underline(underline));
    }
    if let Some(color) = format.text_color {
        changes.push(StyleChange::Color(parse_hex_rgb(&color)?));
    }
    if let Some(fill) = format.fill_color {
        let rgb = match fill {
            Some(hex) => Some(parse_hex_rgb(&hex)?),
            None => None,
        };
        changes.push(StyleChange::Fill(rgb));
    }
    if let Some(fmt) = format.number_format {
        changes.push(StyleChange::NumberFormat(fmt));
    }
    if let Some(align) = format.align {
        changes.push(StyleChange::Align(align));
    }
    if let Some(vertical) = format.vertical_align {
        changes.push(StyleChange::Vertical(vertical));
    }
    if let Some(wrap) = format.wrap_text {
        changes.push(StyleChange::Wrap(wrap));
    }
    if let Some(pt) = format.font_size_pt {
        if !pt.is_finite() || pt <= 0.0 || pt > 409.0 {
            return Err(format!("invalid font_size '{pt}' (expected 1..=409 pt)"));
        }
        changes.push(StyleChange::FontSize((pt * 100.0).round() as u32));
    }
    let mut cmds: Vec<Command> = changes
        .into_iter()
        .map(|change| {
            Command::Sheet(SheetCommand::SetFormat {
                slide,
                ranges: ranges.clone(),
                change,
            })
        })
        .collect();
    if let Some(border_style) = format.border {
        let thin = Some(Border {
            line: BorderLine::Thin,
            color: Rgb(0),
        });
        match border_style {
            AiBorderStyle::All => {
                for edge in [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom] {
                    cmds.push(Command::Sheet(SheetCommand::SetFormat {
                        slide,
                        ranges: ranges.clone(),
                        change: StyleChange::Border(edge, thin),
                    }));
                }
            }
            AiBorderStyle::None => {
                for edge in [Edge::Left, Edge::Right, Edge::Top, Edge::Bottom] {
                    cmds.push(Command::Sheet(SheetCommand::SetFormat {
                        slide,
                        ranges: ranges.clone(),
                        change: StyleChange::Border(edge, None),
                    }));
                }
            }
            AiBorderStyle::Bottom => {
                let bottom_ranges: Vec<CellRange> = ranges
                    .iter()
                    .map(|r| CellRange {
                        first_row: r.last_row,
                        first_column: r.first_column,
                        last_row: r.last_row,
                        last_column: r.last_column,
                    })
                    .collect();
                cmds.push(Command::Sheet(SheetCommand::SetFormat {
                    slide,
                    ranges: bottom_ranges,
                    change: StyleChange::Border(Edge::Bottom, thin),
                }));
            }
            AiBorderStyle::Outline => {
                let top_ranges: Vec<CellRange> = ranges
                    .iter()
                    .map(|r| CellRange {
                        first_row: r.first_row,
                        first_column: r.first_column,
                        last_row: r.first_row,
                        last_column: r.last_column,
                    })
                    .collect();
                let bottom_ranges: Vec<CellRange> = ranges
                    .iter()
                    .map(|r| CellRange {
                        first_row: r.last_row,
                        first_column: r.first_column,
                        last_row: r.last_row,
                        last_column: r.last_column,
                    })
                    .collect();
                let left_ranges: Vec<CellRange> = ranges
                    .iter()
                    .map(|r| CellRange {
                        first_row: r.first_row,
                        first_column: r.first_column,
                        last_row: r.last_row,
                        last_column: r.first_column,
                    })
                    .collect();
                let right_ranges: Vec<CellRange> = ranges
                    .iter()
                    .map(|r| CellRange {
                        first_row: r.first_row,
                        first_column: r.last_column,
                        last_row: r.last_row,
                        last_column: r.last_column,
                    })
                    .collect();
                for (edge, edge_ranges) in [
                    (Edge::Top, top_ranges),
                    (Edge::Bottom, bottom_ranges),
                    (Edge::Left, left_ranges),
                    (Edge::Right, right_ranges),
                ] {
                    cmds.push(Command::Sheet(SheetCommand::SetFormat {
                        slide,
                        ranges: edge_ranges,
                        change: StyleChange::Border(edge, thin),
                    }));
                }
            }
        }
    }
    if cmds.is_empty() {
        return Err("at least one format property must be specified".to_string());
    }
    if cmds.len() == 1 {
        Ok(cmds.pop().expect("single command"))
    } else {
        Ok(Command::Group(cmds))
    }
}

/// Build a [`Command`] to clear cell values and/or formats in `ranges`.
pub fn build_clear_cells_command(
    slide: SlideId,
    ranges: Vec<CellRange>,
    clear_values: bool,
    clear_formats: bool,
    max_cells: usize,
) -> Result<Command, String> {
    if ranges.is_empty() {
        return Err("ranges must not be empty".to_string());
    }
    if !clear_values && !clear_formats {
        return Err("at least one of clear_values or clear_formats must be true".to_string());
    }
    let mut cmds = Vec::new();
    if clear_values {
        let mut edits = Vec::new();
        for range in &ranges {
            let rows = (range.last_row - range.first_row) as usize + 1;
            let cols = (range.last_column - range.first_column) as usize + 1;
            let total = rows.saturating_mul(cols);
            if edits.len().saturating_add(total) > max_cells {
                return Err(format!(
                    "clear_cells exceeds maximum of {max_cells} cells"
                ));
            }
            for row in range.first_row..=range.last_row {
                for column in range.first_column..=range.last_column {
                    edits.push(CellEdit {
                        row,
                        column,
                        input: CellInput::Clear,
                    });
                }
            }
        }
        if !edits.is_empty() {
            cmds.push(Command::Sheet(SheetCommand::SetCells { slide, edits }));
        }
    }
    if clear_formats {
        cmds.push(Command::Sheet(SheetCommand::ClearFormats { slide, ranges }));
    }
    if cmds.len() == 1 {
        Ok(cmds.pop().expect("single command"))
    } else {
        Ok(Command::Group(cmds))
    }
}

/// Resolve an optional sheet name to its [`SlideId`], defaulting to `active_slide`.
pub fn resolve_sheet_slide(
    workbook: &Workbook,
    name: Option<&str>,
    active_slide: SlideId,
) -> Result<SlideId, String> {
    let Some(name) = name.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(active_slide);
    };
    workbook
        .sheets
        .iter()
        .find(|s| s.name == name)
        .and_then(|s| s.slide)
        .ok_or_else(|| format!("sheet '{name}' not found"))
}

/// Build a [`Command`] to insert `count` rows or columns at 0-based index `at`.
pub fn build_insert_axis_command(
    slide: SlideId,
    axis: Axis,
    at: u32,
    count: u32,
) -> Result<Command, String> {
    if count == 0 || count > 1000 {
        return Err(format!("count must be between 1 and 1000 (got {count})"));
    }
    Ok(Command::Sheet(SheetCommand::Insert {
        slide,
        axis,
        at,
        count,
    }))
}

/// Build a [`Command`] to delete `count` rows or columns starting at 0-based index `at`.
pub fn build_delete_axis_command(
    slide: SlideId,
    axis: Axis,
    at: u32,
    count: u32,
) -> Result<Command, String> {
    if count == 0 || count > 1000 {
        return Err(format!("count must be between 1 and 1000 (got {count})"));
    }
    Ok(Command::Sheet(SheetCommand::Delete {
        slide,
        axis,
        at,
        count,
    }))
}

/// Build a [`Command`] to merge or unmerge `range`.
pub fn build_merge_command(
    slide: SlideId,
    range: CellRange,
    merge: bool,
    center: bool,
) -> Command {
    if merge {
        Command::Sheet(SheetCommand::Merge {
            slide,
            range,
            center,
        })
    } else {
        Command::Sheet(SheetCommand::Unmerge { slide, range })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sheet::{CellKind, SheetCell};

    #[test]
    fn parses_cell_refs_and_ranges() {
        assert_eq!(parse_cell_ref("A1"), Some((0, 0)));
        assert_eq!(parse_cell_ref("$C$12"), Some((11, 2)));
        assert_eq!(parse_cell_ref("bad"), None);

        let range = parse_cell_range("C10:A2").expect("normalized range");
        assert_eq!(
            range,
            CellRange {
                first_row: 1,
                first_column: 0,
                last_row: 9,
                last_column: 2,
            }
        );
        assert_eq!(format_range(&range), "A2:C10");
        assert_eq!(
            format_range(&parse_cell_range("B3").expect("single cell")),
            "B3"
        );
    }

    #[test]
    fn reads_range_and_respects_cell_limit() {
        let mut sheet = Sheet::default();
        sheet.cells.push(SheetCell {
            row: 0,
            column: 0,
            value: CellValue::Number(10.0),
            text: "10".to_string(),
            kind: CellKind::Number,
            formula: None,
            style: 0,
            phonetic: None,
            runs: None,
        });
        sheet.cells.push(SheetCell {
            row: 0,
            column: 1,
            value: CellValue::Number(20.0),
            text: "20".to_string(),
            kind: CellKind::Number,
            formula: Some("A1*2".to_string()),
            style: 0,
            phonetic: None,
            runs: None,
        });

        let range = parse_cell_range("A1:B2").unwrap();
        let snap = read_sheet_range(&sheet, range, 100).unwrap();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].cell, "A1");
        assert_eq!(snap[0].text, "10");
        assert_eq!(snap[0].formula, None);
        assert_eq!(snap[1].cell, "B1");
        assert_eq!(snap[1].formula.as_deref(), Some("=A1*2"));

        let err = read_sheet_range(&sheet, parse_cell_range("A1:Z100").unwrap(), 200).unwrap_err();
        assert!(err.contains("exceeding limit"));
    }

    #[test]
    fn builds_set_cells_and_format_commands() {
        let slide = SlideId(1);
        let cmd = build_set_cells_command(
            slide,
            vec![
                AiCellEdit {
                    cell: "A1".to_string(),
                    value: AiCellValue::Number(42.0),
                },
                AiCellEdit {
                    cell: "B1".to_string(),
                    value: AiCellValue::Formula("=A1*2".to_string()),
                },
            ],
        )
        .unwrap();
        match cmd {
            Command::Sheet(SheetCommand::SetCells { edits, .. }) => {
                assert_eq!(edits.len(), 2);
                assert_eq!(edits[1].input, CellInput::Formula("A1*2".to_string()));
            }
            other => panic!("unexpected command: {other:?}"),
        }

        let fmt = build_set_format_command(
            slide,
            vec![parse_cell_range("A1:B1").unwrap()],
            AiCellFormat {
                bold: Some(true),
                text_color: Some("#FF0000".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(matches!(fmt, Command::Group(cmds) if cmds.len() == 2));
    }
}
